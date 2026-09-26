//! The loopback chat-completion proxy listener (shared by every runtime owner).
//!
//! The desktop application and a headless instance each serve the *same* proxy
//! surface on their own loopback port, because the proxy address and the
//! process-local internal key are published through process-global state
//! (`CHAT_COMPLETION_PROXY`). A runtime that resolves a `group@alias` model
//! therefore has to own its proxy: pointing at another instance's listener would
//! authenticate with a key that instance never issued.
//!
//! This module owns only *starting and stopping* the listener. Route
//! composition, ordering, authentication, model resolution and header handling
//! stay in [`crate::ccproxy::router`] and are untouched
//! (`src-tauri/src/ccproxy/CONSTITUTION.md` §4, §5, §6).

use crate::ai::interaction::chat_completion::ChatState;
use crate::constants::{
    CFG_CCPROXY_LISTEN, CFG_CCPROXY_LISTEN_DEFAULT, CFG_CCPROXY_PORT, CFG_CCPROXY_PORT_DEFAULT,
    CHAT_COMPLETION_PROXY,
};
use crate::db::MainStore;
use axum::extract::DefaultBodyLimit;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};

/// How many times a bind is retried before the proxy reports failure.
const MAX_ATTEMPTS: u32 = 5;

/// A running loopback chat-completion proxy.
pub struct CcproxyServer {
    addr: SocketAddr,
    shutdown: broadcast::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl CcproxyServer {
    /// The address the proxy is actually listening on.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The loopback base URL published to the in-process AI client.
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    /// Stops the listener and waits for it to close.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.task.await;
    }
}

/// Binds the configured loopback address and serves the proxy routes.
///
/// `CHAT_COMPLETION_PROXY` is published only after the listener exists, so the
/// in-process client can never be pointed at a port this instance does not own.
pub async fn start(
    main_store: Arc<MainStore>,
    chat_state: Arc<ChatState>,
    package_version: String,
) -> Result<CcproxyServer, String> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    let app = crate::ccproxy::routes(package_version, main_store.clone(), chat_state)
        .await
        .layer(DefaultBodyLimit::max(50 * 1024 * 1024)) // 50MB limit for AI requests
        .layer(cors);

    let (port, listen) = (
        main_store.get_config(CFG_CCPROXY_PORT, CFG_CCPROXY_PORT_DEFAULT),
        main_store.get_config(CFG_CCPROXY_LISTEN, CFG_CCPROXY_LISTEN_DEFAULT.to_string()),
    );

    let mut attempts = 0;
    loop {
        attempts += 1;
        match crate::http::server::try_available_port(&listen, port).await {
            Ok(listener) => {
                let addr = listener
                    .local_addr()
                    .map_err(|error| format!("failed to read the ccproxy address: {error}"))?;
                *CHAT_COMPLETION_PROXY.write() = format!("http://127.0.0.1:{}", addr.port());
                log::info!("Serving chat completion proxy on http://{addr}");

                let (shutdown, mut receiver) = broadcast::channel::<()>(1);
                let task = tokio::spawn(async move {
                    let server = axum::serve(
                        listener,
                        app.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .with_graceful_shutdown(async move {
                        let _ = receiver.recv().await;
                        log::info!("CCProxy server received shutdown signal");
                    });
                    match server.await {
                        Ok(()) => log::info!("CCProxy server shut down gracefully"),
                        Err(error) => log::error!("CCProxy server error: {error}"),
                    }
                });
                return Ok(CcproxyServer {
                    addr,
                    shutdown,
                    task,
                });
            }
            Err(error) => {
                log::error!("Failed to start ccproxy server (attempt {attempts}): {error}");
                if attempts >= MAX_ATTEMPTS {
                    return Err(format!(
                        "failed to start the chat completion proxy after {MAX_ATTEMPTS} attempts"
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}
