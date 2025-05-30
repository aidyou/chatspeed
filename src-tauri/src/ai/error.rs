use rust_i18n::t;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AiError {
    #[error("{}", t!("chat.error.api_request_failed", provider = .provider, details = .details))]
    ApiRequestFailed { provider: String, details: String },

    #[error("{}", t!("chat.error.invalid_input", details =.0))]
    InvalidInput(String),

    #[error("{}", t!("chat.error.response_parse_failed", provider = .provider, details = .details))]
    ResponseParseFailed { provider: String, details: String },

    #[error("{}", t!("chat.error.stream_processing_failed", provider = .provider, details = .details))]
    StreamProcessingFailed { provider: String, details: String },

    #[error("{}", t!("chat.error.tool_call_serialization_failed", details = .details))]
    ToolCallSerializationFailed { details: String },

    #[error("{}", t!("ai.error.deserialization_failed", context = .context, details = .details))]
    DeserializationFailed { context: String, details: String },

    #[error("{}", t!("ai.error.upstream_chat_error", message = .message))]
    UpstreamChatError { message: String },
}
