#[cfg(not(feature = "desktop"))]
pub mod claude;
mod common;
#[cfg(not(feature = "desktop"))]
mod constants;
#[cfg(not(feature = "desktop"))]
pub mod gemini;
#[cfg(not(feature = "desktop"))]
pub mod ollama;
#[cfg(not(feature = "desktop"))]
pub mod openai;
#[cfg(not(feature = "desktop"))]
pub mod openai_responses;

pub use common::*;
#[cfg(not(feature = "desktop"))]
pub use constants::*;
