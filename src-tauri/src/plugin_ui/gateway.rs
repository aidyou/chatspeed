//! Host-side loopback HTTP gateway for one plugin's verified static UI.
//!
//! The runtime's `/control/v1/plugins/{id}/ui/{path}` route is bearer-protected
//! and reserved for trusted processes, so a WebView tab cannot load it. This
//! server gives each tab its own random capability and serves the verified
//! bundle through that capability only:
//!
//! - it binds `127.0.0.1:0`, so it is reachable only from this machine;
//! - every request must carry the exact `Host` of this listener, which stops a
//!   DNS-rebinding page from reaching the gateway through another name;
//! - a browser `Origin`, when present, must be this gateway's own origin, and no
//!   CORS header is ever emitted;
//! - the route is `GET`-only and matches `capability` + `plugin_id`; an unknown,
//!   revoked or cross-tab request is a bare 404 that never names a filesystem
//!   path;
//! - each asset is read through the supervisor's [`RuntimeClient`], so the
//!   bearer, `no_proxy` and no-redirect rules are the client's, and the bytes
//!   are bounded by the client's per-chunk limit;
//! - a capability revoked while its asset was in flight is re-checked before the
//!   response is built, so a racing `revoke` still wins.
//!
//! The capability token, the resolved URLs and the runtime bearer never appear
//! in logs, errors or the frontend.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chatspeed_runtime_client::{is_safe_ui_asset_path, PluginUiAssetResponse};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::plugin_types::{BUILTIN_PLUGIN_KIND, PLUGIN_ID};
use crate::runtime_client::RuntimeSupervisor;

use super::types::{PluginUiGrant, PluginUiInventory, PluginUiState};

/// The only route this gateway serves: one tab's capability, one plugin, one
/// relative asset path. It is `GET`-only and never carries a query.
const PLUGIN_UI_ROUTE: &str = "/ui/{capability}/{plugin_id}/{*asset_path}";

/// The Content-Security-Policy applied to every served asset.
///
/// `sandbox allow-scripts` (without `allow-same-origin`) gives the document an
/// opaque origin, so one tab can never reach another tab's storage. The
/// `default-src 'none'` list keeps the bundle on `'self'` and blocks every
/// remote load, framing, embedding and form post.
const PLUGIN_UI_CSP: &str = "sandbox allow-scripts; default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

/// Reads a plugin's runtime inventory and verified UI assets.
///
/// The production implementation reaches the runtime through the supervisor; the
/// seam exists so the gateway's routing and capability logic can be exercised
/// without a live runtime.
#[async_trait::async_trait]
trait PluginUiSource: Send + Sync + 'static {
    /// The runtime's current plugin inventory.
    async fn inventory(&self) -> Result<PluginUiInventory, String>;
    /// One verified plugin UI asset, read through the runtime's bytes API.
    async fn fetch(
        &self,
        plugin_id: &str,
        asset_path: &str,
    ) -> Result<PluginUiAssetResponse, String>;
}

/// The production source: every call goes through the supervisor's live client.
struct SupervisorSource {
    supervisor: Arc<RuntimeSupervisor>,
}

#[async_trait::async_trait]
impl PluginUiSource for SupervisorSource {
    async fn inventory(&self) -> Result<PluginUiInventory, String> {
        let client = self
            .supervisor
            .client()
            .await
            .map_err(|error| error.to_string())?;
        let value = client
            .get(&inventory_path())
            .await
            .map_err(|error| error.to_string())?;
        serde_json::from_value(value).map_err(|error| {
            format!("the runtime plugin inventory is not a recognized document: {error}")
        })
    }

    async fn fetch(
        &self,
        plugin_id: &str,
        asset_path: &str,
    ) -> Result<PluginUiAssetResponse, String> {
        let client = self
            .supervisor
            .client()
            .await
            .map_err(|error| error.to_string())?;
        client
            .get_plugin_ui_asset(plugin_id, asset_path)
            .await
            .map_err(|error| error.to_string())
    }
}

/// The read-only inventory route the runtime exposes for this plugin.
fn inventory_path() -> String {
    format!("/control/v1/plugins/{PLUGIN_ID}")
}

/// One active capability: the plugin whose assets it may read.
///
/// The entry and its URLs live in the [`PluginUiGrant`] the host already holds;
/// routing only needs the plugin binding, and an unknown or removed capability
/// is indistinguishable from a plugin mismatch.
#[derive(Clone, PartialEq, Eq)]
struct GrantEntry {
    plugin_id: String,
}

/// Shared axum state: the listener identity, the capability table and the source.
#[derive(Clone)]
struct GatewayState {
    /// This gateway's own origin, e.g. `http://127.0.0.1:41234`.
    origin: String,
    /// This gateway's own `Host` value, e.g. `127.0.0.1:41234`.
    host: String,
    grants: Arc<Mutex<HashMap<String, GrantEntry>>>,
    source: Arc<dyn PluginUiSource>,
}

impl GatewayState {
    /// Snapshots one capability so a later request cannot serve a revoked grant.
    fn grant(&self, capability: &str) -> Option<GrantEntry> {
        lock(&self.grants).get(capability).cloned()
    }
}

/// A running loopback plugin UI gateway.
///
/// `Debug` deliberately prints neither the capability tokens nor the URLs.
pub struct PluginUiGateway {
    addr: SocketAddr,
    source: Arc<dyn PluginUiSource>,
    grants: Arc<Mutex<HashMap<String, GrantEntry>>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl std::fmt::Debug for PluginUiGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginUiGateway")
            .field("addr", &self.addr)
            .field("capability_count", &lock(&self.grants).len())
            .finish()
    }
}

impl PluginUiGateway {
    /// Starts the gateway on an ephemeral loopback port.
    ///
    /// The returned handle owns the server task; dropping it stops the server.
    pub async fn start(supervisor: Arc<RuntimeSupervisor>) -> Result<Self, String> {
        let source: Arc<dyn PluginUiSource> = Arc::new(SupervisorSource { supervisor });
        Self::start_with_source(source).await
    }

    /// Starts the gateway against an explicit asset source.
    async fn start_with_source(source: Arc<dyn PluginUiSource>) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| format!("cannot bind the plugin UI gateway listener: {error}"))?;
        let addr = listener
            .local_addr()
            .map_err(|error| format!("cannot resolve the plugin UI gateway address: {error}"))?;
        let grants = Arc::new(Mutex::new(HashMap::new()));
        let state = GatewayState {
            origin: format!("http://{addr}"),
            host: addr.to_string(),
            grants: grants.clone(),
            source: source.clone(),
        };
        let router = Router::new()
            .route(PLUGIN_UI_ROUTE, get(handle_asset))
            .with_state(state);

        let (shutdown, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let served = axum::serve(listener, router).with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            });
            if let Err(error) = served.await {
                log::warn!("[PluginUiGateway] the plugin UI gateway stopped with an error: {error}");
            }
        });
        log::info!("[PluginUiGateway] serving loopback plugin UI assets on {addr}");

        Ok(Self {
            addr,
            source,
            grants,
            shutdown: Some(shutdown),
            task,
        })
    }

    /// Issues a per-tab capability grant after proving the plugin's state.
    ///
    /// The runtime inventory must list the plugin as a built-in, enabled,
    /// verified bundle whose declared entry matches `entry`. On success the
    /// caller receives one random capability and the URLs it resolves to; the
    /// capability is kept in Rust only and never returned to the frontend.
    pub async fn grant(&self, plugin_id: &str, entry: &str) -> Result<PluginUiGrant, String> {
        if !is_safe_ui_asset_path(entry) {
            return Err("plugin UI entry is not a safe relative path".to_string());
        }
        let inventory = self.source.inventory().await?;
        verify_inventory(&inventory, plugin_id, entry)?;

        let capability = Uuid::new_v4().simple().to_string();
        let prefix = format!("http://{}/ui/{capability}/{plugin_id}/", self.addr);
        let url = format!("{prefix}{entry}");
        lock(&self.grants).insert(
            capability.clone(),
            GrantEntry {
                plugin_id: plugin_id.to_string(),
            },
        );
        Ok(PluginUiGrant {
            capability,
            url,
            prefix,
        })
    }

    /// Revokes one capability; a later request for it is a bare 404.
    pub async fn revoke(&self, capability: &str) {
        lock(&self.grants).remove(capability);
    }

    /// Revokes every capability, dropping a whole tab set at once.
    pub async fn revoke_all(&self) {
        lock(&self.grants).clear();
    }
}

impl Drop for PluginUiGateway {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        // Cancelling the task also drops the listener, so no request can race
        // the graceful shutdown once the handle is gone.
        self.task.abort();
    }
}

/// Serves one request gated by Host, Origin, capability, plugin and path.
///
/// Every rejection is intentionally opaque: a 404 carries no body and never a
/// physical path, so an attacker cannot tell "unknown" from "revoked" from
/// "wrong plugin".
async fn handle_asset(
    State(state): State<GatewayState>,
    headers: HeaderMap,
    Path((capability, plugin_id, asset_path)): Path<(String, String, String)>,
) -> Response {
    if !host_allowed(&state, &headers) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !origin_allowed(&state, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(grant) = state.grant(&capability) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if grant.plugin_id != plugin_id || !is_safe_ui_asset_path(&asset_path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let asset = match state.source.fetch(&plugin_id, &asset_path).await {
        Ok(asset) => asset,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    // A revoke racing an in-flight read must still win: only serve when the
    // capability is still mapped exactly as it was before the read.
    if state.grant(&capability).as_ref() != Some(&grant) {
        return StatusCode::NOT_FOUND.into_response();
    }
    asset_response(asset)
}

/// Builds the success response, forwarding only the asset bytes and a fixed,
/// hard-coded header set (no runtime transport header is passed through).
fn asset_response(asset: PluginUiAssetResponse) -> Response {
    let mut response = Response::new(Body::from(asset.bytes));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    let content_type = HeaderValue::from_str(&asset.content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PLUGIN_UI_CSP),
    );
    response
}

/// Accepts a request only when it carries this listener's exact `Host`.
fn host_allowed(state: &GatewayState, headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        == Some(state.host.as_str())
}

/// Accepts an absent `Origin`, or one equal to this gateway's own origin.
fn origin_allowed(state: &GatewayState, headers: &HeaderMap) -> bool {
    match headers.get(header::ORIGIN) {
        None => true,
        Some(value) => value.to_str().ok() == Some(state.origin.as_str()),
    }
}

/// Proves the runtime inventory describes the requested built-in UI bundle.
fn verify_inventory(
    inventory: &PluginUiInventory,
    plugin_id: &str,
    entry: &str,
) -> Result<(), String> {
    if inventory.schema_version != crate::plugin_types::PLUGIN_INVENTORY_SCHEMA_VERSION {
        return Err("the plugin inventory version is unsupported".to_string());
    }
    let record = inventory
        .plugins
        .iter()
        .find(|record| record.id == plugin_id)
        .ok_or_else(|| "the runtime inventory does not list the requested plugin".to_string())?;
    if record.kind != BUILTIN_PLUGIN_KIND {
        return Err("the requested plugin is not a built-in bundle".to_string());
    }
    if record.state != PluginUiState::Enabled {
        return Err("the requested plugin is not enabled".to_string());
    }
    let ui = record
        .ui
        .as_ref()
        .ok_or_else(|| "the requested plugin exposes no UI".to_string())?;
    if !ui.verified {
        return Err("the plugin UI bundle is not verified".to_string());
    }
    if ui.entry != entry {
        return Err("the requested entry does not match the plugin UI entry".to_string());
    }
    Ok(())
}

/// Locks the capability table, recovering from a poisoned mutex instead of
/// panicking the whole gateway.
fn lock(
    grants: &Mutex<HashMap<String, GrantEntry>>,
) -> MutexGuard<'_, HashMap<String, GrantEntry>> {
    match grants.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::Notify;

    const ENTRY: &str = "index.html";
    const BODY: &[u8] = b"<html></html>";

    /// A scripted asset source, so the gateway's routing and capability logic
    /// is exercised without a live runtime.
    struct FakeSource {
        inventory: serde_json::Value,
        delay: Duration,
        started: Option<Arc<Notify>>,
        fetch_calls: AtomicUsize,
    }

    impl FakeSource {
        fn new(inventory: serde_json::Value) -> Self {
            Self {
                inventory,
                delay: Duration::ZERO,
                started: None,
                fetch_calls: AtomicUsize::new(0),
            }
        }

        fn enabled() -> Self {
            Self::new(serde_json::json!({
                "schema_version": 1,
                "plugins": [{
                    "id": PLUGIN_ID,
                    "kind": BUILTIN_PLUGIN_KIND,
                    "state": "enabled",
                    "ui": { "entry": ENTRY, "verified": true }
                }]
            }))
        }
    }

    #[async_trait::async_trait]
    impl PluginUiSource for FakeSource {
        async fn inventory(&self) -> Result<PluginUiInventory, String> {
            serde_json::from_value(self.inventory.clone()).map_err(|error| error.to_string())
        }

        async fn fetch(
            &self,
            _plugin_id: &str,
            _asset_path: &str,
        ) -> Result<PluginUiAssetResponse, String> {
            self.fetch_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(started) = &self.started {
                started.notify_one();
            }
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            Ok(PluginUiAssetResponse {
                bytes: BODY.to_vec(),
                content_type: "text/html".to_string(),
            })
        }
    }

    async fn gateway(source: FakeSource) -> PluginUiGateway {
        PluginUiGateway::start_with_source(Arc::new(source))
            .await
            .expect("gateway")
    }

    fn base(gateway: &PluginUiGateway) -> String {
        format!("http://{}", gateway.addr)
    }

    fn host(gateway: &PluginUiGateway) -> String {
        gateway.addr.to_string()
    }

    fn origin(gateway: &PluginUiGateway) -> String {
        format!("http://{}", gateway.addr)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client")
    }

    fn status_of(text: &str) -> u16 {
        text.lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .expect("status code")
    }

    /// Sends a hand-written request and reads only the response head.
    ///
    /// Used where a URL client would normalize the target or rewrite `Host`.
    async fn raw_request(
        gateway: &PluginUiGateway,
        request_line: &str,
        headers: &[(&str, &str)],
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(gateway.addr).await.expect("connect");
        let mut request = format!("{request_line}\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("Connection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.expect("write");
        stream.flush().await.ok();

        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut chunk))
                .await
            {
                Ok(Ok(read)) => read,
                _ => break,
            };
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buffer).to_string();
        (status_of(&text), text)
    }

    #[tokio::test]
    async fn grant_proves_the_runtime_inventory_before_issuing_a_capability() {
        let gateway = gateway(FakeSource::enabled()).await;
        let grant = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        assert_eq!(grant.capability.len(), 32);
        assert!(grant.url.ends_with("/index.html"));
        assert!(grant.url.starts_with(&base(&gateway)));
        assert!(grant.prefix.ends_with(&format!("/{PLUGIN_ID}/")));
        assert!(grant.prefix.starts_with(&base(&gateway)));

        let second = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        assert_ne!(grant.capability, second.capability, "each grant is unique");

        // A safe but undeclared entry, and a foreign plugin id, are refused.
        assert!(gateway.grant(PLUGIN_ID, "other.html").await.is_err());
        assert!(gateway.grant("other-plugin", ENTRY).await.is_err());
        assert!(gateway.grant(PLUGIN_ID, "../plugin.json").await.is_err());
    }

    #[tokio::test]
    async fn grant_rejects_inventory_that_does_not_prove_the_bundle() {
        let cases = [
            serde_json::json!({"schema_version":1,"plugins":[{"id":PLUGIN_ID,"kind":BUILTIN_PLUGIN_KIND,"state":"disabled","ui":{"entry":ENTRY,"verified":true}}]}),
            serde_json::json!({"schema_version":1,"plugins":[{"id":PLUGIN_ID,"kind":"external","state":"enabled","ui":{"entry":ENTRY,"verified":true}}]}),
            serde_json::json!({"schema_version":1,"plugins":[{"id":PLUGIN_ID,"kind":BUILTIN_PLUGIN_KIND,"state":"enabled","ui":{"entry":ENTRY,"verified":false}}]}),
            serde_json::json!({"schema_version":1,"plugins":[{"id":PLUGIN_ID,"kind":BUILTIN_PLUGIN_KIND,"state":"enabled"}]}),
            serde_json::json!({"schema_version":1,"plugins":[]}),
        ];
        for inventory in cases {
            let gateway = gateway(FakeSource::new(inventory)).await;
            assert!(
                gateway.grant(PLUGIN_ID, ENTRY).await.is_err(),
                "an unproven inventory must be refused"
            );
        }
    }

    #[tokio::test]
    async fn grant_refuses_missing_or_unknown_inventory_versions() {
        let mut unsupported = FakeSource::enabled().inventory;
        unsupported["schema_version"] = serde_json::json!(2);
        let server = gateway(FakeSource::new(unsupported)).await;
        assert!(server.grant(PLUGIN_ID, ENTRY).await.is_err());
        let mut missing = FakeSource::enabled().inventory;
        missing.as_object_mut().expect("inventory").remove("schema_version");
        let server = gateway(FakeSource::new(missing)).await;
        assert!(server.grant(PLUGIN_ID, ENTRY).await.is_err());
    }

    #[tokio::test]
    async fn inventory_parsing_ignores_the_extra_wire_fields() {
        // Mirrors the real inventory, which carries many more fields.
        let inventory = serde_json::json!({
            "schema_version": 1,
            "plugins": [{
                "id": PLUGIN_ID,
                "kind": BUILTIN_PLUGIN_KIND,
                "version": "0.1.0",
                "state": "enabled",
                "capabilities": ["skills:read"],
                "ui": {"entry": ENTRY, "assets": [ENTRY], "verified": true, "content_digest": "abc", "route_version": 1},
                "root": "/tmp/plugin",
                "bundle_digest": "abc"
            }],
            "uninstall_scope": "plugin-bundle-only",
            "managed_skills_dir": "/tmp/skills",
            "host": {"host": "static-bundle-no-exec"}
        });
        let gateway = gateway(FakeSource::new(inventory)).await;
        assert!(gateway.grant(PLUGIN_ID, ENTRY).await.is_ok());
    }

    #[tokio::test]
    async fn serves_the_entry_with_the_fixed_security_headers() {
        let gateway = gateway(FakeSource::enabled()).await;
        let grant = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let target = format!("/ui/{}/{}/{}", grant.capability, PLUGIN_ID, ENTRY);
        let url = format!("{}{target}", base(&gateway));

        let response = client()
            .get(&url)
            .header("Origin", origin(&gateway))
            .send()
            .await
            .expect("send");
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("text/html")
        );
        assert_eq!(
            response
                .headers()
                .get("cache-control")
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(
            response
                .headers()
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            response
                .headers()
                .get("referrer-policy")
                .and_then(|value| value.to_str().ok()),
            Some("no-referrer")
        );
        assert_eq!(
            response
                .headers()
                .get("content-security-policy")
                .and_then(|value| value.to_str().ok()),
            Some(PLUGIN_UI_CSP)
        );
        assert!(
            response.headers().get("access-control-allow-origin").is_none(),
            "the gateway emits no CORS header"
        );
        assert_eq!(response.text().await.expect("body").as_bytes(), BODY);
    }

    #[tokio::test]
    async fn unknown_cross_plugin_and_traversal_paths_are_a_bare_404() {
        let gateway = gateway(FakeSource::enabled()).await;
        let grant = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let capability = &grant.capability;
        let host = host(&gateway);

        let traversal = format!("/ui/{capability}/{PLUGIN_ID}/../plugin.json");
        let (status, text) = raw_request(&gateway, &format!("GET {traversal} HTTP/1.1"), &[("Host", &host)]).await;
        assert_eq!(status, 404);
        assert!(!text.contains("plugin.json"), "no physical path may leak");

        let encoded = format!("/ui/{capability}/{PLUGIN_ID}/%2e%2e/plugin.json");
        let (status, _) = raw_request(&gateway, &format!("GET {encoded} HTTP/1.1"), &[("Host", &host)]).await;
        assert_eq!(status, 404);

        let url = format!("{}/ui/deadbeef/{PLUGIN_ID}/{ENTRY}", base(&gateway));
        assert_eq!(
            client().get(&url).send().await.expect("send").status().as_u16(),
            404
        );

        let url = format!("{}/ui/{capability}/other-plugin/{ENTRY}", base(&gateway));
        assert_eq!(
            client().get(&url).send().await.expect("send").status().as_u16(),
            404
        );
    }

    #[tokio::test]
    async fn host_and_origin_are_enforced() {
        let gateway = gateway(FakeSource::enabled()).await;
        let grant = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let target = format!("/ui/{}/{}/{}", grant.capability, PLUGIN_ID, ENTRY);

        // A rebound Host is refused before routing.
        let (status, _) = raw_request(
            &gateway,
            &format!("GET {target} HTTP/1.1"),
            &[("Host", "evil.example")],
        )
        .await;
        assert_eq!(status, 404);

        let url = format!("{}{target}", base(&gateway));
        let status = client()
            .get(&url)
            .header("Origin", "http://evil.example")
            .send()
            .await
            .expect("send")
            .status()
            .as_u16();
        assert_eq!(status, 403);

        // An absent Origin is allowed.
        assert_eq!(
            client().get(&url).send().await.expect("send").status().as_u16(),
            200
        );
    }

    #[tokio::test]
    async fn non_get_methods_are_refused() {
        let gateway = gateway(FakeSource::enabled()).await;
        let grant = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let url = format!("{}/ui/{}/{}/{}", base(&gateway), grant.capability, PLUGIN_ID, ENTRY);
        assert_eq!(
            client().post(&url).send().await.expect("send").status().as_u16(),
            405
        );
    }

    #[tokio::test]
    async fn revoked_capabilities_are_indistinguishable_from_unknown_ones() {
        let gateway = gateway(FakeSource::enabled()).await;
        let first = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let second = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let first_url = format!("{}/ui/{}/{}/{}", base(&gateway), first.capability, PLUGIN_ID, ENTRY);
        let second_url = format!("{}/ui/{}/{}/{}", base(&gateway), second.capability, PLUGIN_ID, ENTRY);

        gateway.revoke(&first.capability).await;
        assert_eq!(client().get(&first_url).send().await.expect("send").status().as_u16(), 404);
        assert_eq!(client().get(&second_url).send().await.expect("send").status().as_u16(), 200);

        gateway.revoke_all().await;
        assert_eq!(client().get(&second_url).send().await.expect("send").status().as_u16(), 404);
    }

    #[tokio::test]
    async fn a_revoke_racing_an_in_flight_read_still_wins() {
        let started = Arc::new(Notify::new());
        let mut source = FakeSource::enabled();
        source.started = Some(started.clone());
        source.delay = Duration::from_millis(200);
        let gateway = Arc::new(gateway(source).await);
        let grant = gateway.grant(PLUGIN_ID, ENTRY).await.expect("grant");
        let url = format!("{}/ui/{}/{}/{}", base(&gateway), grant.capability, PLUGIN_ID, ENTRY);

        let requester = tokio::spawn(async move {
            client().get(&url).send().await.expect("send").status().as_u16()
        });

        started.notified().await;
        gateway.revoke(&grant.capability).await;
        assert_eq!(requester.await.expect("join"), 404);
    }

    #[tokio::test]
    async fn dropping_the_gateway_stops_the_server() {
        let gateway = gateway(FakeSource::enabled()).await;
        let addr = gateway.addr;
        drop(gateway);

        let mut refused = false;
        for _ in 0..50 {
            if TcpStream::connect(addr).await.is_err() {
                refused = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(refused, "dropping the gateway must close the listener");
    }
}