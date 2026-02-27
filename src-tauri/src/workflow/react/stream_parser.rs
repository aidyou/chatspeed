use std::sync::Arc;
use crate::workflow::react::gateway::Gateway;
use crate::workflow::react::types::GatewayPayload;

pub struct StreamParser {
    buffer: String,
    session_id: String,
    gateway: Arc<dyn Gateway>,
    is_in_answer_user: bool,
    is_in_text_field: bool,
    last_emitted_pos: usize,
}

impl StreamParser {
    pub fn new(session_id: String, gateway: Arc<dyn Gateway>) -> Self {
        Self {
            buffer: String::new(),
            session_id,
            gateway,
            is_in_answer_user: false,
            is_in_text_field: false,
            last_emitted_pos: 0,
        }
    }

    /// Processes a new chunk of JSON text from LLM
    pub async fn push(&mut self, chunk: &str) {
        self.buffer.push_str(chunk);
        
        // Very basic heuristic for Phase 2:
        // Look for "answer_user" first
        if !self.is_in_answer_user {
            if self.buffer.contains("\"answer_user\"") {
                self.is_in_answer_user = true;
            }
        }

        if self.is_in_answer_user && !self.is_in_text_field {
            // Look for "text": "
            if let Some(pos) = self.buffer.find("\"text\"") {
                // Find the first quote after "text":
                let remaining = &self.buffer[pos + 6..];
                if let Some(quote_pos) = remaining.find('\"') {
                    self.is_in_text_field = true;
                    self.last_emitted_pos = pos + 6 + quote_pos + 1;
                }
            }
        }

        if self.is_in_text_field {
            let current_content = &self.buffer[self.last_emitted_pos..];
            // We need to handle escaping in a real implementation, 
            // but for Phase 2 skeleton, we'll just push what we have.
            // Stop if we find a non-escaped ending quote
            if let Some(end_quote_pos) = self.find_unescaped_quote(current_content) {
                let to_emit = &current_content[..end_quote_pos];
                if !to_emit.is_empty() {
                    let _ = self.gateway.send(&self.session_id, GatewayPayload::Text { content: to_emit.to_string() }).await;
                }
                self.is_in_text_field = false;
                self.is_in_answer_user = false; // Reset for next tool call
                self.last_emitted_pos += end_quote_pos + 1;
            } else {
                // Emit everything we have so far
                if !current_content.is_empty() {
                    let _ = self.gateway.send(&self.session_id, GatewayPayload::Text { content: current_content.to_string() }).await;
                    self.last_emitted_pos = self.buffer.len();
                }
            }
        }
    }

    fn find_unescaped_quote(&self, s: &str) -> Option<usize> {
        let mut escaped = false;
        for (i, c) in s.char_indices() {
            if c == '\\' {
                escaped = !escaped;
            } else if c == '\"' && !escaped {
                return Some(i);
            } else {
                escaped = false;
            }
        }
        None
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.is_in_answer_user = false;
        self.is_in_text_field = false;
        self.last_emitted_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::gateway::Gateway;
    use crate::workflow::react::error::WorkflowEngineError;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct MockGateway {
        emitted_texts: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Gateway for MockGateway {
        async fn send(&self, _session_id: &str, payload: GatewayPayload) -> Result<(), WorkflowEngineError> {
            if let GatewayPayload::Text { content } = payload {
                self.emitted_texts.lock().unwrap().push(content);
            }
            Ok(())
        }
        async fn receive_input(&self, _session_id: &str) -> Result<String, WorkflowEngineError> {
            Ok("".to_string())
        }
    }

    #[tokio::test]
    async fn test_stream_parser_basic() {
        let gateway = Arc::new(MockGateway { emitted_texts: Mutex::new(vec![]) });
        let mut parser = StreamParser::new("test_session".to_string(), gateway.clone());

        // Fragmented JSON
        parser.push("{\"tool\": \"answer").await;
        parser.push("_user\", \"arguments\": {\"text\": \"Hello").await;
        parser.push(", world!\"}}").await;

        let texts = gateway.emitted_texts.lock().unwrap();
        // Depending on implementation, it might be ["Hello", ", world!"] or merged.
        // With current heuristic:
        // 1. "{\"tool\": \"answer" -> no text field yet
        // 2. "_user\", \"arguments\": {\"text\": \"Hello" -> sets is_in_text_field, emits "Hello"
        // 3. ", world!\"}}" -> finds unescaped quote, emits ", world!"
        assert!(texts.contains(&"Hello".to_string()));
        assert!(texts.contains(&", world!".to_string()));
    }
}
