mod claude;
mod common;
mod gemini;
mod openai;
mod traits;

pub use claude::ClaudeBackendAdapter;
pub use common::update_message_block;
pub use gemini::GeminiBackendAdapter;
pub use openai::OpenAIBackendAdapter;
pub use traits::{BackendAdapter, BackendResponse};
