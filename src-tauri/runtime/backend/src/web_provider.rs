//! Single-slot desktop Web MCP provider registry (AC-8).
//!
//! The desktop hosts the two fixed web tools (`web_fetch`, `web_search`) as a
//! dedicated MCP server on an ephemeral `127.0.0.1` port and registers it here.
//! The runtime then reaches it as an ordinary streamable-HTTP MCP server, so the
//! workflow engine, replay and security see the two tools exactly like any other
//! MCP tool — there is no second executor, database or generic RPC.
//!
//! The registry enforces the security-relevant invariants:
//!
//! - **one slot**: a second live desktop is rejected with a structured
//!   `conflict`, never silently rerouted or replaced;
//! - **lease-bound**: the slot is dropped as soon as its registering lease is no
//!   longer valid;
//! - **idempotent for the same lease**: re-registering the same identity is a
//!   no-op, while a new port on the same lease is an explicit generation
//!   replacement;
//! - **fixed endpoint**: the runtime always dials
//!   `http://127.0.0.1:<port>/mcp`;
//! - **no reconnect**: the provider MCP client is built with a single bounded
//!   attempt, so a dead or released provider fails its in-flight call as
//!   `unavailable` instead of silently reconnecting.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chatspeed_contracts::{
    validate_web_mcp_port, web_mcp_endpoint, ClientLease, WebMcpProviderError,
    WebMcpProviderRegistration, WebMcpProviderRegistrationResponse, WebMcpProviderStatus,
    WEB_MCP_CODE_CONFLICT, WEB_MCP_CODE_FORBIDDEN, WEB_MCP_CODE_UNAVAILABLE, WEB_MCP_SERVER_NAME,
};
use tokio::sync::Mutex;

use crate::ai::interaction::chat_completion::ChatState;
use crate::mcp::client::{McpProtocolType, McpServerConfig};
use crate::workflow::react::client::http::server::{RuntimeWebMcpPlane, WebProviderLeaseCheck};

/// Bound on the provider MCP connect/read timeout, in seconds.
const PROVIDER_TIMEOUT_SECS: u64 = 20;

/// Installs and removes the provider MCP server the runtime dials.
///
/// This is the only seam between the registry's slot bookkeeping and the
/// canonical `ToolManager` MCP path, so the registry can be unit-tested without
/// a database, a `ChatState`, or a live socket.
#[async_trait]
pub trait WebProviderInstaller: Send + Sync + 'static {
    /// Registers the reserved provider MCP server bound to `(port, proof)`.
    async fn install_provider(&self, port: u16, proof: &str) -> Result<(), String>;

    /// Removes the reserved provider MCP server; a missing server is a no-op.
    async fn remove_provider(&self) -> Result<(), String>;
}

/// The canonical installer: it registers the provider through `ToolManager`.
pub struct ToolManagerProviderInstaller {
    chat_state: Arc<ChatState>,
}

impl ToolManagerProviderInstaller {
    /// Wraps the runtime's single chat/tool state.
    pub fn new(chat_state: Arc<ChatState>) -> Self {
        Self { chat_state }
    }

    /// Builds the fixed provider MCP configuration the runtime dials.
    ///
    /// The URL is derived from the port with a fixed loopback host and path; the
    /// proof token is the provider's header credential, held in memory only.
    fn provider_config(port: u16, proof: &str) -> McpServerConfig {
        McpServerConfig {
            name: WEB_MCP_SERVER_NAME.to_string(),
            protocol_type: McpProtocolType::StreamableHttp,
            url: Some(web_mcp_endpoint(port)),
            bearer_token: Some(proof.to_string()),
            timeout: Some(PROVIDER_TIMEOUT_SECS),
            ..Default::default()
        }
    }
}

#[async_trait]
impl WebProviderInstaller for ToolManagerProviderInstaller {
    async fn install_provider(&self, port: u16, proof: &str) -> Result<(), String> {
        self.chat_state
            .tool_manager
            .clone()
            .register_web_mcp_provider(Self::provider_config(port, proof))
            .await
            .map_err(|error| format!("cannot reach the desktop Web MCP provider: {error}"))
    }

    async fn remove_provider(&self) -> Result<(), String> {
        self.chat_state
            .tool_manager
            .unregister_mcp_server(WEB_MCP_SERVER_NAME)
            .await
            .map_err(|error| format!("cannot remove the Web MCP provider: {error}"))
    }
}

/// One installed provider slot.
struct ActiveProvider {
    generation: u64,
    port: u16,
    instance_id: String,
    client_id: String,
    lease_id: String,
    /// Opaque provider proof token. Held in memory only and never logged.
    proof: String,
    expires_at: String,
}

impl ActiveProvider {
    fn response(&self) -> WebMcpProviderRegistrationResponse {
        WebMcpProviderRegistrationResponse {
            server_name: WEB_MCP_SERVER_NAME.to_string(),
            generation: self.generation,
            expires_at: self.expires_at.clone(),
        }
    }

    fn status(&self) -> WebMcpProviderStatus {
        WebMcpProviderStatus {
            server_name: WEB_MCP_SERVER_NAME.to_string(),
            generation: self.generation,
            instance_id: self.instance_id.clone(),
            client_id: self.client_id.clone(),
            lease_id: self.lease_id.clone(),
            port: self.port,
            expires_at: self.expires_at.clone(),
        }
    }
}

impl fmt::Debug for ActiveProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActiveProvider")
            .field("generation", &self.generation)
            .field("port", &self.port)
            .field("instance_id", &self.instance_id)
            .field("client_id", &self.client_id)
            .field("lease_id", &self.lease_id)
            .field("proof", &"<redacted>")
            .finish()
    }
}

/// The single provider slot, bound to one live client lease.
pub struct WebProviderRegistry {
    installer: Arc<dyn WebProviderInstaller>,
    active: std::sync::RwLock<Option<ActiveProvider>>,
    /// Serializes slot mutations (including their async install/remove) so two
    /// concurrent registrations can never both win the single slot.
    op_lock: Mutex<()>,
    generation: AtomicU64,
}

impl WebProviderRegistry {
    /// Builds a registry that installs the provider through `installer`.
    pub fn new(installer: Arc<dyn WebProviderInstaller>) -> Self {
        Self {
            installer,
            active: std::sync::RwLock::new(None),
            op_lock: Mutex::new(()),
            generation: AtomicU64::new(0),
        }
    }

    /// Removes the slot and uninstalls the provider MCP server.
    async fn drop_slot(&self, active: ActiveProvider) {
        log::info!(
            "[Runtime][web-mcp] dropping provider slot (generation {}, port {})",
            active.generation,
            active.port
        );
        if let Err(message) = self.installer.remove_provider().await {
            log::warn!("[Runtime][web-mcp] failed to remove the provider MCP server: {message}");
        }
    }
}

#[async_trait]
impl RuntimeWebMcpPlane for WebProviderRegistry {
    async fn register_provider(
        &self,
        registration: &WebMcpProviderRegistration,
        lease: &ClientLease,
        proof: &str,
        instance_id: &str,
    ) -> Result<WebMcpProviderRegistrationResponse, WebMcpProviderError> {
        validate_web_mcp_port(registration.port)?;
        let _op = self.op_lock.lock().await;

        // Decide under the read lock, then release it before awaiting install.
        {
            let guard = self.active.read().expect("provider lock poisoned");
            if let Some(active) = guard.as_ref() {
                let same_lease =
                    active.client_id == lease.client_id && active.lease_id == lease.lease_id;
                if same_lease
                    && active.proof == proof
                    && active.instance_id == instance_id
                    && active.port == registration.port
                {
                    // Idempotent re-confirmation of the same identity.
                    return Ok(active.response());
                }
                if !same_lease {
                    return Err(WebMcpProviderError::new(
                        WEB_MCP_CODE_CONFLICT,
                        format!(
                            "the Web MCP provider slot is held by client `{}`; a second desktop is not rerouted onto it",
                            active.client_id
                        ),
                    ));
                }
                // Same lease, different identity: an explicit generation
                // replacement, installed below.
            }
        }

        self.installer
            .install_provider(registration.port, proof)
            .await
            .map_err(|message| {
                WebMcpProviderError::new(
                    WEB_MCP_CODE_UNAVAILABLE,
                    format!("cannot install the desktop Web MCP provider: {message}"),
                )
            })?;

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let active = ActiveProvider {
            generation,
            port: registration.port,
            instance_id: instance_id.to_string(),
            client_id: lease.client_id.clone(),
            lease_id: lease.lease_id.clone(),
            proof: proof.to_string(),
            expires_at: lease.expires_at.clone(),
        };
        let response = active.response();
        *self.active.write().expect("provider lock poisoned") = Some(active);
        log::info!(
            "[Runtime][web-mcp] installed provider slot (generation {generation}, port {})",
            registration.port
        );
        Ok(response)
    }

    async fn unregister_provider(
        &self,
        lease: &ClientLease,
        proof: &str,
    ) -> Result<(), WebMcpProviderError> {
        let _op = self.op_lock.lock().await;
        let removed = {
            let mut guard = self.active.write().expect("provider lock poisoned");
            let matches = guard.as_ref().map(|active| {
                active.client_id == lease.client_id
                    && active.lease_id == lease.lease_id
                    && active.proof == proof
            });
            match matches {
                Some(true) => guard.take(),
                Some(false) => {
                    return Err(WebMcpProviderError::new(
                        WEB_MCP_CODE_FORBIDDEN,
                        "the registered Web MCP provider does not match this proof",
                    ))
                }
                // Removing an absent slot is a harmless no-op.
                None => None,
            }
        };
        if let Some(active) = removed {
            self.drop_slot(active).await;
        }
        Ok(())
    }

    fn provider_status(&self) -> Option<WebMcpProviderStatus> {
        self.active
            .read()
            .expect("provider lock poisoned")
            .as_ref()
            .map(ActiveProvider::status)
    }

    async fn sweep_invalid_leases(&self, is_valid: &dyn WebProviderLeaseCheck) {
        let _op = self.op_lock.lock().await;
        // Read the identity first, then validate it, then drop it. `op_lock`
        // serializes every slot mutation, so the slot cannot change between the
        // read and the write.
        let identity = {
            let guard = self.active.read().expect("provider lock poisoned");
            guard
                .as_ref()
                .map(|active| (active.client_id.clone(), active.lease_id.clone()))
        };
        let Some((client_id, lease_id)) = identity else {
            return;
        };
        if is_valid.is_valid(&client_id, &lease_id) {
            return;
        }
        let removed = self.active.write().expect("provider lock poisoned").take();
        if let Some(active) = removed {
            self.drop_slot(active).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex as StdMutex;

    /// Records installs/removals and can be told to fail the install.
    struct FakeInstaller {
        installs: AtomicUsize,
        removals: AtomicUsize,
        installed: StdMutex<Option<(u16, String)>>,
        fail_install: bool,
        /// Leases the fake knows are live; everything else is treated as stale.
        live_leases: StdMutex<Vec<(String, String)>>,
    }

    impl FakeInstaller {
        fn new(fail_install: bool) -> Arc<Self> {
            Arc::new(Self {
                installs: AtomicUsize::new(0),
                removals: AtomicUsize::new(0),
                installed: StdMutex::new(None),
                fail_install,
                live_leases: StdMutex::new(Vec::new()),
            })
        }

        fn is_valid(&self, client_id: &str, lease_id: &str) -> bool {
            self.live_leases
                .lock()
                .expect("live leases")
                .iter()
                .any(|(client, lease)| client == client_id && lease == lease_id)
        }
    }

    #[async_trait]
    impl WebProviderInstaller for FakeInstaller {
        async fn install_provider(&self, port: u16, proof: &str) -> Result<(), String> {
            if self.fail_install {
                return Err("connection refused".to_string());
            }
            self.installs.fetch_add(1, Ordering::SeqCst);
            *self.installed.lock().expect("installed") = Some((port, proof.to_string()));
            Ok(())
        }

        async fn remove_provider(&self) -> Result<(), String> {
            self.removals.fetch_add(1, Ordering::SeqCst);
            *self.installed.lock().expect("installed") = None;
            Ok(())
        }
    }

    fn lease(client_id: &str, lease_id: &str) -> ClientLease {
        ClientLease {
            client_id: client_id.to_string(),
            lease_id: lease_id.to_string(),
            client_kind: "tauri".to_string(),
            expires_at: "unix-100".to_string(),
        }
    }

    fn registration(port: u16) -> WebMcpProviderRegistration {
        WebMcpProviderRegistration { port }
    }

    /// Adapts a plain closure to the sweep oracle for tests.
    struct FnCheck<'a>(&'a (dyn Fn(&str, &str) -> bool + Send + Sync));

    impl WebProviderLeaseCheck for FnCheck<'_> {
        fn is_valid(&self, client_id: &str, lease_id: &str) -> bool {
            (self.0)(client_id, lease_id)
        }
    }

    #[tokio::test]
    async fn register_installs_once_and_reports_the_slot() {
        let installer = FakeInstaller::new(false);
        let registry = WebProviderRegistry::new(installer.clone());

        let response = registry
            .register_provider(
                &registration(41234),
                &lease("tauri-main", "lease-1"),
                "proof-a",
                "desktop-a",
            )
            .await
            .expect("register");
        assert_eq!(response.server_name, WEB_MCP_SERVER_NAME);
        assert_eq!(response.generation, 1);
        assert_eq!(installer.installs.load(Ordering::SeqCst), 1);

        let status = registry.provider_status().expect("status");
        assert_eq!(status.port, 41234);
        assert_eq!(status.client_id, "tauri-main");
        assert_eq!(status.lease_id, "lease-1");
        assert_eq!(status.instance_id, "desktop-a");
        assert_eq!(status.expires_at, "unix-100");
    }

    #[tokio::test]
    async fn same_identity_re_registration_is_idempotent() {
        let installer = FakeInstaller::new(false);
        let registry = WebProviderRegistry::new(installer.clone());
        let lease = lease("tauri-main", "lease-1");

        let first = registry
            .register_provider(&registration(41234), &lease, "proof-a", "desktop-a")
            .await
            .expect("first");
        let second = registry
            .register_provider(&registration(41234), &lease, "proof-a", "desktop-a")
            .await
            .expect("second");
        assert_eq!(first.generation, second.generation);
        assert_eq!(installer.installs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn same_lease_new_port_is_an_explicit_generation_replacement() {
        let installer = FakeInstaller::new(false);
        let registry = WebProviderRegistry::new(installer.clone());
        let lease = lease("tauri-main", "lease-1");

        let first = registry
            .register_provider(&registration(41234), &lease, "proof-a", "desktop-a")
            .await
            .expect("first");
        let second = registry
            .register_provider(&registration(41235), &lease, "proof-a", "desktop-a")
            .await
            .expect("replacement");
        assert!(second.generation > first.generation);
        assert_eq!(registry.provider_status().expect("status").port, 41235);
        assert_eq!(installer.installs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_second_live_desktop_is_rejected_with_conflict() {
        let installer = FakeInstaller::new(false);
        let registry = WebProviderRegistry::new(installer.clone());
        registry
            .register_provider(
                &registration(41234),
                &lease("tauri-main", "lease-1"),
                "proof-a",
                "desktop-a",
            )
            .await
            .expect("first");

        let error = registry
            .register_provider(
                &registration(41234),
                &lease("tauri-other", "lease-2"),
                "proof-b",
                "desktop-b",
            )
            .await
            .expect_err("second desktop must conflict");
        assert_eq!(error.code, WEB_MCP_CODE_CONFLICT);
        // The incumbent is untouched and no second install happened.
        assert_eq!(
            registry.provider_status().expect("status").client_id,
            "tauri-main"
        );
        assert_eq!(installer.installs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn install_failure_leaves_no_slot() {
        let installer = FakeInstaller::new(true);
        let registry = WebProviderRegistry::new(installer);
        let error = registry
            .register_provider(
                &registration(41234),
                &lease("tauri-main", "lease-1"),
                "proof-a",
                "desktop-a",
            )
            .await
            .expect_err("install must fail");
        assert_eq!(error.code, WEB_MCP_CODE_UNAVAILABLE);
        assert!(registry.provider_status().is_none());
    }

    #[tokio::test]
    async fn unregister_requires_a_matching_proof() {
        let installer = FakeInstaller::new(false);
        let registry = WebProviderRegistry::new(installer.clone());
        let lease = lease("tauri-main", "lease-1");
        registry
            .register_provider(&registration(41234), &lease, "proof-a", "desktop-a")
            .await
            .expect("register");

        let error = registry
            .unregister_provider(&lease, "wrong-proof")
            .await
            .expect_err("wrong proof is forbidden");
        assert_eq!(error.code, WEB_MCP_CODE_FORBIDDEN);
        assert!(registry.provider_status().is_some());

        registry
            .unregister_provider(&lease, "proof-a")
            .await
            .expect("matching proof removes the slot");
        assert!(registry.provider_status().is_none());
        assert_eq!(installer.removals.load(Ordering::SeqCst), 1);

        // Repeating the unregister is a harmless no-op.
        registry
            .unregister_provider(&lease, "proof-a")
            .await
            .expect("second unregister is a no-op");
    }

    #[tokio::test]
    async fn sweep_drops_the_slot_when_its_lease_is_gone() {
        let installer = FakeInstaller::new(false);
        let registry = WebProviderRegistry::new(installer.clone());
        registry
            .register_provider(
                &registration(41234),
                &lease("tauri-main", "lease-1"),
                "proof-a",
                "desktop-a",
            )
            .await
            .expect("register");
        installer
            .live_leases
            .lock()
            .expect("live leases")
            .push(("tauri-main".to_string(), "lease-1".to_string()));

        // Still-valid lease: the slot survives a sweep.
        let live = |c: &str, l: &str| installer.is_valid(c, l);
        registry.sweep_invalid_leases(&FnCheck(&live)).await;
        assert!(registry.provider_status().is_some());

        // Released lease: the slot is dropped and the provider removed.
        installer.live_leases.lock().expect("live leases").clear();
        registry.sweep_invalid_leases(&FnCheck(&live)).await;
        assert!(registry.provider_status().is_none());
        assert_eq!(installer.removals.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn zero_port_is_rejected() {
        let registry = WebProviderRegistry::new(FakeInstaller::new(false));
        let error = registry
            .register_provider(
                &registration(0),
                &lease("tauri-main", "lease-1"),
                "proof-a",
                "desktop-a",
            )
            .await
            .expect_err("port 0 must be rejected");
        assert_eq!(error.code, chatspeed_contracts::WEB_MCP_CODE_INVALID_PORT);
    }
}
