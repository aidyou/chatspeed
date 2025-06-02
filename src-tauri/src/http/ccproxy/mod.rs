//! # Chat Completion Proxy (ccproxy) Module
//!
//! This module provides HTTP endpoints to proxy chat completion requests
//! to various AI models, offering a unified interface and centralized key management.
pub mod adapter;
pub mod errors;
pub mod handlers;
// pub mod protocol_adapter;
pub mod proxy_rotator;
pub mod router;
pub mod types;
