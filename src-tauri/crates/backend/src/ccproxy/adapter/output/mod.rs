mod adapter_enum;
mod claude_output;
mod error_response;
mod gemini_output;
mod ollama_output;
mod openai_output;
mod openai_responses_output;
pub mod traits;

pub use adapter_enum::OutputAdapterEnum;
pub use claude_output::ClaudeOutputAdapter;
pub use gemini_output::GeminiOutputAdapter;
pub use ollama_output::OllamaOutputAdapter;
pub use openai_output::OpenAIOutputAdapter;
pub use openai_responses_output::OpenAIResponsesOutputAdapter;
pub use traits::OutputAdapter;
