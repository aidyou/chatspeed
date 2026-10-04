#[cfg(not(feature = "desktop"))]
mod claude_input;
#[cfg(not(feature = "desktop"))]
mod gemini_input;
pub mod helper;
#[cfg(not(feature = "desktop"))]
mod ollama_input;
#[cfg(not(feature = "desktop"))]
mod openai_input;
#[cfg(not(feature = "desktop"))]
mod openai_responses_input;

#[cfg(not(feature = "desktop"))]
pub use claude_input::from_claude;
#[cfg(not(feature = "desktop"))]
pub use gemini_input::{from_gemini, from_gemini_embedding};
#[cfg(not(feature = "desktop"))]
pub use ollama_input::{from_ollama, from_ollama_embed, from_ollama_embedding};
#[cfg(not(feature = "desktop"))]
pub use openai_input::{from_openai, from_openai_embedding};
#[cfg(not(feature = "desktop"))]
pub use openai_responses_input::from_openai_responses;
