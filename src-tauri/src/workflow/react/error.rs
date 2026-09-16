use crate::ai::error::AiError;
use crate::db::error::StoreError;
use crate::tools::ToolError;

use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowTerminalError {
    pub content: String,
    pub error_type: &'static str,
    pub metadata: Value,
}

#[derive(Error, Debug, Serialize)]
#[serde(tag = "type", content = "details")]
pub enum WorkflowEngineError {
    #[error("{0}")]
    Cancelled(String),

    #[error("Database error: {0}")]
    Db(#[from] StoreError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("AI Model error: {0}")]
    Ai(#[from] AiError),

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Gateway error: {0}")]
    Gateway(String),

    #[error("Gateway input channel closed")]
    GatewayInputChannelClosed,

    #[error("Gateway input channel missing")]
    GatewayInputChannelMissing,

    #[error("General error: {0}")]
    General(String),

    #[error("Dispatcher channel full")]
    DispatcherChannelFull,

    #[error("Dispatcher closed")]
    DispatcherClosed,
}

impl WorkflowEngineError {
    pub fn terminal_error(&self) -> WorkflowTerminalError {
        let (content, error_type, upstream) = match self {
            Self::Ai(AiError::ApiRequestFailed {
                status_code,
                provider,
                ..
            }) => (
                self.to_string(),
                match status_code {
                    401 => "llm_authentication",
                    402 => "llm_billing",
                    _ => "engine",
                },
                Some((*status_code, provider.clone())),
            ),
            Self::Ai(AiError::RawApiRequestFailed {
                status_code,
                provider,
                details,
            }) => (
                details.clone(),
                match status_code {
                    401 => "llm_authentication",
                    402 => "llm_billing",
                    _ => "engine",
                },
                Some((*status_code, provider.clone())),
            ),
            _ => (
                format!(
                    "Critical Error: {}\n<SYSTEM_REMINDER>A fatal error occurred in the execution engine. If this error is related to invalid tool arguments, please correct your parameters and retry. If it is a system-level issue, please inform the user about the failure.</SYSTEM_REMINDER>",
                    self
                ),
                "engine",
                None,
            ),
        };
        let mut metadata = json!({
            "error_type": error_type
        });
        if let Some((status_code, provider)) = upstream {
            metadata["upstream_status_code"] = json!(status_code);
            metadata["provider"] = json!(provider);
        }
        WorkflowTerminalError {
            content,
            error_type,
            metadata,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowEngineError;
    use crate::ai::error::AiError;

    #[test]
    fn terminal_ai_errors_expose_structured_auth_and_billing_types() {
        for (status_code, expected_type) in [
            (401, "llm_authentication"),
            (402, "llm_billing"),
            (403, "engine"),
            (404, "engine"),
        ] {
            let terminal = WorkflowEngineError::Ai(AiError::RawApiRequestFailed {
                status_code,
                provider: "provider".to_string(),
                details: "upstream error".to_string(),
            })
            .terminal_error();

            assert_eq!(terminal.error_type, expected_type);
            assert_eq!(terminal.metadata["error_type"], expected_type);
            assert_eq!(terminal.metadata["upstream_status_code"], status_code);
            assert_eq!(terminal.metadata["provider"], "provider");
            assert_eq!(terminal.content, "upstream error");
        }
    }
}
