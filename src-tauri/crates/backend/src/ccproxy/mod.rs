//! # Chat Completion Proxy (ccproxy) Module
//!
//! This module provides HTTP endpoints to proxy chat completion requests
//! to various AI models, offering a unified interface and centralized key management.
// The adapter, decision and helper layers are the runtime proxy engine; the
// desktop only keeps the wire protocol types and the shared token estimator.
#[cfg(not(feature = "desktop"))]
pub(crate) mod adapter;
#[cfg(not(feature = "desktop"))]
mod auth;
#[cfg(not(feature = "desktop"))]
pub(crate) mod decision;
mod errors;
#[cfg(not(feature = "desktop"))]
mod handler;
#[cfg(not(feature = "desktop"))]
mod helper;
#[cfg(not(feature = "desktop"))]
pub mod launcher;
// Runtime-only home for the chat proxy resolver: the desktop crate takes it
// from `commands::chat`, which the runtime cannot link.
#[cfg(not(feature = "desktop"))]
pub mod proxy_settings;
#[cfg(not(feature = "desktop"))]
mod router;
mod types;
pub mod utils;

pub use errors::CCProxyError;
#[cfg(not(feature = "desktop"))]
pub use handler::{
    handle_chat_completion, handle_decision, handle_embedding, handle_list_models,
    handle_ollama_tags, handle_responses,
};
#[cfg(not(feature = "desktop"))]
pub use helper::{get_tool_id, StreamProcessor};
#[cfg(not(feature = "desktop"))]
pub use router::routes;
#[cfg(not(feature = "desktop"))]
pub use types::claude;
#[cfg(not(feature = "desktop"))]
pub use types::gemini;
#[cfg(not(feature = "desktop"))]
pub use types::openai;
// The desktop keeps only the wire protocol enum; the proxy engine config and
// stream format are runtime-owned.
pub use types::ChatProtocol;
#[cfg(not(feature = "desktop"))]
pub use types::{ChatCompletionProxyConfig, StreamFormat};
