//! Standalone MCP Server
//!
//! This module provides functionality to run the MCP proxy server as a standalone service
//! on a separate port from the main HTTP server.

use crate::ai::interaction::chat_completion::ChatState;
use crate::mcp::server::handler::McpProxyHandler;
use rmcp::transport::sse_server::{SseServer, SseServerConfig};
use rust_i18n::t;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Starts a standalone MCP proxy server
///
/// # Arguments
/// * `chat_state` - Chat state instance for accessing the tool manager
/// * `listener` - Pre-bound TCP listener
/// * `cancellation_token` - Token to cancel the server
///
/// # Returns
/// Returns Ok(()) on success, or an error on failure
pub async fn start_standalone_mcp_server(
    chat_state: Arc<ChatState>,
    listener: tokio::net::TcpListener,
    cancellation_token: CancellationToken,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Get the bound address from the listener
    let bind_addr = listener.local_addr()?;

    // Create SSE server configuration
    let sse_config = SseServerConfig {
        bind: bind_addr,
        sse_path: "/sse".to_string(),
        post_path: "/message".to_string(),
        ct: cancellation_token.clone(),
        sse_keep_alive: None,
    };

    // Create SSE server
    let (sse_server, router) = SseServer::new(sse_config);

    // Start the service with our handler
    let service_ct = sse_server.with_service(move || McpProxyHandler::new(chat_state.clone()));

    log::info!(
        "{}",
        t!("mcp.proxy.server_started", address = bind_addr.to_string())
    );

    // Start the HTTP server in a separate task
    let server_ct = cancellation_token.clone();
    tokio::spawn(async move {
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            server_ct.cancelled().await;
            log::info!("{}", t!("mcp.proxy.server_stopped"));
        });

        if let Err(e) = server.await {
            log::error!("MCP server error: {}", e);
        }
    });

    // Wait for service cancellation
    service_ct.cancelled().await;

    Ok(())
}
