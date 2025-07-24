//! # Chat Completion Proxy (ccproxy) Module
//!
//! This module provides HTTP endpoints to proxy chat completion requests
//! to various AI models, offering a unified interface and centralized key management.
pub mod adapter;
pub mod claude_handler;
pub mod claude_types;
pub mod common;
pub mod errors;
pub mod openai_handler;
pub mod openai_types;
pub mod proxy_rotator;
pub mod router;
