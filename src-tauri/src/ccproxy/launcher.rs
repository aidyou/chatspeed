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

/// The desktop client shares the bind helper with the legacy static HTTP server.
#[cfg(feature = "desktop")]
use crate::http::server::try_available_port;

/// Binds the first available port at or after `preferred`.
///
/// The desktop client and a runtime owner serve the proxy on the same listen
/// address convention. Desktop delegates to `crate::http::server`, which also
/// owns the legacy static server; the desktop-free runtime cannot link that
/// module, so it probes the same order locally: skip a port that answers on
/// loopback, then require the wildcard bind to succeed before taking it.
///
/// On macOS, listeners bound to `0.0.0.0:PORT` and `127.0.0.1:PORT` can
/// otherwise coexist, causing requests for the same loopback port to cross
/// processes.
#[cfg(not(feature = "desktop"))]
async fn try_available_port(
    listen: &str,
    preferred: u16,
) -> Result<tokio::net::TcpListener, String> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

    let bind_ip: IpAddr = listen
        .parse()
        .map_err(|error| format!("failed to parse the proxy listen address {listen}: {error}"))?;
    let (loopback_ip, wildcard_ip) = match bind_ip {
        IpAddr::V4(_) => (
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        ),
        IpAddr::V6(_) => (
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        ),
    };

    for port in preferred..=u16::MAX {
        if std::net::TcpStream::connect_timeout(
            &SocketAddr::new(loopback_ip, port),
            Duration::from_millis(10),
        )
        .is_ok()
        {
            continue;
        }
        if std::net::TcpListener::bind(SocketAddr::new(wildcard_ip, port)).is_err() {
            continue;
        }
        if let Ok(listener) = tokio::net::TcpListener::bind(SocketAddr::new(bind_ip, port)).await {
            return Ok(listener);
        }
    }
    Err(format!(
        "no port at or after {preferred} is available for the chat completion proxy"
    ))
}

/// How many times a bind is retried before the proxy reports failure.
const MAX_ATTEMPTS: u32 = 5;

/// A running loopback chat-completion proxy.
pub struct CcproxyServer {
    shutdown: broadcast::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl CcproxyServer {
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
        match try_available_port(&listen, port).await {
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
                return Ok(CcproxyServer { shutdown, task });
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
