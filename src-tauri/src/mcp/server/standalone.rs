//! MCP Server component builders.
//!
//! This module provides factory functions to create routers and services for different MCP transports.

use crate::ai::interaction::chat_completion::ChatState;
use crate::mcp::server::handler::McpProxyHandler;
use axum::Router;
use rmcp::transport::sse_server::{SseServer, SseServerConfig};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Creates a router for the MCP proxy server using SSE transport.
pub fn create_sse_router(
    chat_state: Arc<ChatState>,
    shutdown_token: Option<CancellationToken>,
) -> Router {
    let cancellation_token = shutdown_token.unwrap_or_else(CancellationToken::new);

    let sse_config = SseServerConfig {
        bind: "0.0.0.0:0".parse().unwrap(), // Dummy address
        sse_path: "/".to_string(),
        post_path: "/message".to_string(),
        ct: cancellation_token.clone(),
        // TODO: Consider increasing the keep-alive duration.
        // Some clients (like Gemini CLI) may not handle session drops on OS sleep gracefully,
        // leading to 410 Gone errors. Increasing this from 30s to a longer duration
        // (e.g., 3600s) is a pragmatic workaround.
        sse_keep_alive: Some(Duration::from_secs(30)),
    };

    log::info!("Creating MCP SSE router component.");

    let (sse_server, router) = SseServer::new(sse_config);
    let _service_ct = sse_server.with_service(move || McpProxyHandler::new(chat_state.clone()));
    router
}

/// Creates a service for the MCP proxy server using Streamable HTTP transport.
pub fn create_http_service(
    chat_state: Arc<ChatState>,
) -> StreamableHttpService<McpProxyHandler> {
    log::info!("Creating MCP Streamable HTTP service component.");
    StreamableHttpService::new(
        move || Ok(McpProxyHandler::new(chat_state.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    )
}