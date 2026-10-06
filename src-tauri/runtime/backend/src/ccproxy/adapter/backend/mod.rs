#[cfg(not(feature = "desktop"))]
mod claude;
mod common;
#[cfg(not(feature = "desktop"))]
mod gemini;
#[cfg(not(feature = "desktop"))]
mod ollama;
#[cfg(not(feature = "desktop"))]
mod openai;
#[cfg(not(feature = "desktop"))]
mod traits;

#[cfg(not(feature = "desktop"))]
pub use claude::ClaudeBackendAdapter;
pub use common::generate_tool_prompt;
#[cfg(not(feature = "desktop"))]
pub use common::update_message_block;
#[cfg(not(feature = "desktop"))]
pub use gemini::GeminiBackendAdapter;
#[cfg(not(feature = "desktop"))]
pub use ollama::OllamaBackendAdapter;
#[cfg(not(feature = "desktop"))]
pub use openai::OpenAIBackendAdapter;
#[cfg(not(feature = "desktop"))]
pub use traits::{BackendAdapter, BackendRequestContext, BackendResponse};

#[cfg(all(test, not(feature = "desktop")))]
mod openai_test;
