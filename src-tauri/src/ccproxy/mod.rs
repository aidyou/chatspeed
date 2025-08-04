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

pub use handler::{handle_chat_completion, handle_ollama_tags, handle_openai_list_models};
pub use helper::StreamProcessor;
pub use router::routes;
pub use types::{claude, gemini, openai, StreamFormat};
