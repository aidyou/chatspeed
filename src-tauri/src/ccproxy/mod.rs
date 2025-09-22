//! # Chat Completion Proxy (ccproxy) Module
//!
//! This module provides HTTP endpoints to proxy chat completion requests
//! to various AI models, offering a unified interface and centralized key management.
mod adapter;
mod auth;
mod errors;
mod handler;
mod helper;
mod router;
mod types;
pub mod utils;

pub use handler::{handle_chat_completion, handle_list_models, handle_ollama_tags};
pub use helper::{get_tool_id, StreamProcessor};
pub use router::routes;
pub use types::{claude, gemini, openai, ChatProtocol, StreamFormat};
