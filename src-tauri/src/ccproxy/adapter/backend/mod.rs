mod claude;
mod common;
mod gemini;
mod ollama;
mod openai;
mod traits;

#[cfg(test)]
mod openai_test;

pub use claude::ClaudeBackendAdapter;
pub use common::{
    generate_tool_prompt, update_message_block, ToolUse, TOOL_TAG_END, TOOL_TAG_START,
};
pub use gemini::GeminiBackendAdapter;
pub use ollama::OllamaBackendAdapter;
pub use openai::OpenAIBackendAdapter;
pub use traits::{BackendAdapter, BackendResponse};
