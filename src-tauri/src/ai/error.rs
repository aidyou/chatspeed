use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AiError {
    #[error("{}",t!("chat.error.api_request_failed",provider=provider,details=details,status_code=status_code))]
    ApiRequestFailed {
        status_code: u16,
        provider: String,
        details: String,
    },

    #[error("{0}")]
    InitFailed(String),

    #[error("{}", t!("chat.error.invalid_input", details =.0))]
    InvalidInput(String),

    #[error("{}", t!("chat.error.response_parse_failed", provider = .provider, details = .details))]
    ResponseParseFailed { provider: String, details: String },

    #[error("{}", t!("chat.error.stream_processing_failed", provider = .provider, details = .details))]
    StreamProcessingFailed { provider: String, details: String },

    #[error("{}", t!("chat.error.tool_call_serialization_failed", details = .details))]
    ToolCallSerializationFailed { details: String },

    #[error("{}", t!("chat.error.failed_to_get_or_create_window_channel", error = .0))]
    FailedToGetOrCreateWindowChannel(String),
}
