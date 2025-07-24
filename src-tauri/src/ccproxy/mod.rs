//! # Chat Completion Proxy (ccproxy) Module
//!
//! This module provides HTTP endpoints to proxy chat completion requests
//! to various AI models, offering a unified interface and centralized key management.
mod adapter;
mod auth;
mod common;
mod errors;
mod handler;
mod proxy_rotator;
mod router;
mod types;

pub use errors::handle_proxy_rejection;
pub use handler::{claude_handler, gemini_handler, openai_handler};
pub use router::routes;
pub use types::{claude, gemini, openai};
