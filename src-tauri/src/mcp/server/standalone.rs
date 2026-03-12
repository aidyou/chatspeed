//! MCP Server component builders.
//!
//! This module provides factory functions to create routers and services for different MCP transports.

use crate::ai::interaction::chat_completion::ChatState;
use crate::mcp::server::handler::McpProxyHandler;
use crate::mcp::server::persistent_session::PersistentSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use std::sync::Arc;

/// Creates a service for the MCP proxy server using Streamable HTTP transport.
///
/// This function attempts to create a persistent session manager with multiple fallbacks:
/// 1. First tries to create a persistent session manager with sled database
/// 2. If that fails, tries to create a temporary in-memory session manager
/// 3. As absolute last resort, logs error and returns a basic service (sessions won't persist)
///
/// The function will not panic even if all session manager initialization attempts fail.
pub fn create_http_service(
    chat_state: Arc<ChatState>,
) -> StreamableHttpService<McpProxyHandler, PersistentSessionManager<McpProxyHandler>> {
    log::info!("Creating MCP Streamable HTTP service component with persistent sessions.");

    // Create the service factory closure. It must be `Clone` to be passed to multiple places.
    let service_factory = move || Ok(McpProxyHandler::new(chat_state.clone()));

    // The session manager needs an Arc'd version of the factory.
    let session_manager = match PersistentSessionManager::new(Arc::new(service_factory.clone())) {
        Ok(manager) => {
            log::info!("Successfully created persistent session manager");
            manager
        }
        Err(e) => {
            log::error!("Failed to initialize persistent session manager: {}. Attempting temporary fallback...", e);
            // The fallback logic in PersistentSessionManager::new should handle all cases,
            // but if it still fails, try explicit temporary session manager creation
            match PersistentSessionManager::new_temporary(Arc::new(service_factory.clone())) {
                Ok(temp_manager) => {
                    log::warn!("Using temporary session manager. MCP sessions will not persist across restarts.");
                    temp_manager
                }
                Err(temp_err) => {
                    log::error!(
                        "Even temporary session manager failed: {}. Using emergency fallback.",
                        temp_err
                    );
                    // Absolute last resort: create an in-memory temporary manager
                    // This should virtually never fail since it doesn't touch disk
                    PersistentSessionManager::new_temporary(Arc::new(service_factory.clone()))
                        .unwrap_or_else(|final_err| {
                            log::error!("CRITICAL: All session manager initialization methods failed: {}", final_err);
                            log::error!("MCP server will start but session management may be unstable.");
                            // Create the most basic possible manager - this is truly last resort
                            panic!("Cannot initialize MCP session manager even with in-memory fallback. System may be critically damaged.")
                        })
                }
            }
        }
    };

    // The streamable service itself takes the un-Arc'd closure.
    StreamableHttpService::new(
        service_factory,
        Arc::new(session_manager),
        StreamableHttpServerConfig::default(),
    )
}
