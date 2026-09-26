mod chat_handler;
mod decision_handler;
mod direct_handler;
mod embedding_handler;
mod list_models_handler;
pub mod ollama_extra_handler;
mod request_preprocessor;
mod responses_handler;

use crate::ccproxy::errors::CCProxyError;
use rust_i18n::t;

/// System One decision models are chat-incompatible: `/v1/systemone` serves them instead.
pub(crate) fn decision_is_not_a_chat_protocol() -> CCProxyError {
    CCProxyError::InvalidProtocolError(t!("proxy.error.decision_not_chat_protocol").to_string())
}

pub use chat_handler::handle_chat_completion;
pub use decision_handler::handle_decision;
pub use direct_handler::handle_direct_forward;
pub use embedding_handler::handle_embedding;
pub use list_models_handler::{handle_gemini_list_models, handle_list_models, handle_ollama_tags};
pub use ollama_extra_handler::handle_ollama_show;
pub use responses_handler::handle_responses;
