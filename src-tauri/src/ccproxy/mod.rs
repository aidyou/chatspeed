//! # Chat Completion Proxy (ccproxy) Module
//!
//! This module provides HTTP endpoints to proxy chat completion requests
//! to various AI models, offering a unified interface and centralized key management.
pub(crate) mod adapter;
mod auth;
pub(crate) mod decision;
mod errors;
mod handler;
mod helper;
pub mod launcher;
// Runtime-only home for the chat proxy resolver: the desktop crate takes it
// from `commands::chat`, which the runtime cannot link.
#[cfg(not(feature = "desktop"))]
pub mod proxy_settings;
mod router;
mod types;
pub mod utils;

pub use errors::CCProxyError;
pub use handler::{
    handle_chat_completion, handle_decision, handle_embedding, handle_list_models,
    handle_ollama_tags, handle_responses,
};
pub use helper::{get_tool_id, StreamProcessor};
pub use router::routes;
pub use types::{claude, gemini, openai, ChatCompletionProxyConfig, ChatProtocol, StreamFormat};
