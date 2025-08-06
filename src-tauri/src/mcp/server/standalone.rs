//! Standalone MCP Server
//!
//! This module provides functionality to run the MCP proxy server as a standalone service
//! on a separate port from the main HTTP server.

use crate::ai::interaction::chat_completion::ChatState;
use crate::mcp::server::handler::McpProxyHandler;
use axum::Router;
use rmcp::transport::sse_server::{SseServer, SseServerConfig};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Creates a router for the MCP proxy server.
///
/// # Arguments
/// * `chat_state` - Chat state instance for accessing the tool manager
///
/// # Returns
/// Returns an Axum Router configured for the MCP service.
pub fn create_mcp_router(chat_state: Arc<ChatState>) -> Router {
    // Create a dummy cancellation token, as it's required by SseServerConfig,
    // but the actual server lifecycle will be managed by the main ccproxy server.
    let cancellation_token = CancellationToken::new();

    // The SseServerConfig no longer needs a bind address, as it's not binding a port itself.
    // The paths are kept at the root, as the main router will handle nesting.
    let sse_config = SseServerConfig {
        bind: "0.0.0.0:0".parse().unwrap(), // Dummy address
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        ct: cancellation_token,
        sse_keep_alive: Some(Duration::from_secs(30)),
    };

    // Create SSE server and get the router
    let (sse_server, router) = SseServer::new(sse_config);

    // The service (handler) is now attached directly.
    // The state management will be implicitly handled by the closure capturing `chat_state`.
    let _service_ct = sse_server.with_service(move || McpProxyHandler::new(chat_state.clone()));

    // Return the configured router, ready to be nested.
    // Note: The state here is managed within the McpProxyHandler,
    // so the router itself doesn't need a top-level state.
    router
}
