mod claude;
mod common;
mod gemini;
mod ollama;
mod openai;
mod traits;

pub use claude::ClaudeBackendAdapter;
pub use common::{generate_tool_prompt, update_message_block, TOOL_TAG_END, TOOL_TAG_START};
pub use gemini::GeminiBackendAdapter;
pub use ollama::OllamaBackendAdapter;
pub use openai::OpenAIBackendAdapter;
pub use traits::{BackendAdapter, BackendResponse};

#[cfg(test)]
mod openai_test;
