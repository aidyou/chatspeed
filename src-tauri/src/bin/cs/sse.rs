//! Incremental SSE framing parser for the `cs` CLI.
//!
//! Deliberately small and dependency-free: it tolerates arbitrary network
//! chunk boundaries, CRLF or LF line endings, multi-line `data:` fields and
//! comment/keepalive lines, so JSONL output is never corrupted by framing.

/// One parsed SSE frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub id: Option<String>,
    pub event: Option<String>,
    /// `data:` lines joined with `\n`.
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
    current_id: Option<String>,
    current_event: Option<String>,
    current_data: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds raw bytes (lossily decoded) into the parser and returns any
    /// complete frames.
    pub fn push(&mut self, chunk: &str) -> Vec<SseFrame> {
        self.buffer.push_str(chunk);
        let mut frames = Vec::new();

        // Normalize CRLF so the rest of the parser only deals with '\n'.
        while let Some(position) = find_line_end(&self.buffer) {
            let (line, rest) = self.buffer.split_at(position);
            let line = line.trim_end_matches('\r');
            let rest = &rest[1..];
            let line = line.to_string();
            self.buffer = rest.to_string();

            if line.is_empty() {
                // Empty line: dispatch the current frame, if any.
                if !self.current_data.is_empty() || self.current_event.is_some() {
                    frames.push(SseFrame {
                        id: self.current_id.take(),
                        event: self.current_event.take(),
                        data: self.current_data.join("\n"),
                    });
                    self.current_data.clear();
                } else {
                    self.current_id = None;
                    self.current_event = None;
                }
                continue;
            }

            if let Some(value) = line.strip_prefix(':') {
                // Comment/keepalive line; ignore.
                let _ = value;
                continue;
            }

            let (field, value) = match line.split_once(':') {
                Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                None => (line.as_str(), ""),
            };
            match field {
                "id" => self.current_id = Some(value.to_string()),
                "event" => self.current_event = Some(value.to_string()),
                "data" => self.current_data.push(value.to_string()),
                _ => {}
            }
        }

        frames
    }
}

/// Finds the next `\n` in `buffer`.
fn find_line_end(buffer: &str) -> Option<usize> {
    buffer.find('\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_of(frames: &[SseFrame]) -> Vec<&str> {
        frames.iter().map(|frame| frame.data.as_str()).collect()
    }

    #[test]
    fn parser_handles_split_chunks_and_multiline_data() {
        let mut parser = SseParser::new();
        assert!(parser.push("id: a:1\ndata: {\"a\":\n").is_empty());
        let frames = parser.push("data: 1}\n\ndata: second\n\n");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].id.as_deref(), Some("a:1"));
        assert_eq!(data_of(&frames)[0], "{\"a\":\n1}");
        assert_eq!(data_of(&frames)[1], "second");
    }

    #[test]
    fn parser_handles_crlf_and_comments() {
        let mut parser = SseParser::new();
        let frames =
            parser.push(": keepalive\r\n\r\nevent: reset_required\r\ndata: {\"r\":1}\r\n\r\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event.as_deref(), Some("reset_required"));
        assert_eq!(data_of(&frames), vec!["{\"r\":1}"]);
    }

    #[test]
    fn parser_ignores_keepalives_without_data() {
        let mut parser = SseParser::new();
        assert!(parser.push(": ping\n\n").is_empty());
        assert!(parser.push("").is_empty());
    }

    #[test]
    fn parser_emits_multiple_frames_from_one_chunk() {
        let mut parser = SseParser::new();
        let frames = parser.push("data: one\n\ndata: two\n\n");
        assert_eq!(data_of(&frames), vec!["one", "two"]);
    }
}
