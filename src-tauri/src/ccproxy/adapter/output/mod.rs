mod claude_output;
mod gemini_output;
mod openai_output;
pub mod traits;

pub use claude_output::ClaudeOutputAdapter;
pub use gemini_output::GeminiOutputAdapter;
pub use openai_output::OpenAIOutputAdapter;
pub use traits::OutputAdapter;
