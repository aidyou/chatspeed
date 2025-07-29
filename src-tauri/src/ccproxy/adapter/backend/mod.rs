mod claude;
mod common;
mod gemini;
mod openai;
mod traits;

#[cfg(test)]
mod openai_test;

pub use claude::ClaudeBackendAdapter;
pub use common::update_message_block;
pub use gemini::GeminiBackendAdapter;
pub use openai::OpenAIBackendAdapter;
pub use traits::{BackendAdapter, BackendResponse};
