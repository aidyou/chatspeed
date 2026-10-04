//! Desktop-hosted loopback Web MCP provider (AC-8).
//!
//! The desktop is the only process that links a Tauri WebView, so it hosts the
//! two fixed web tools (`web_fetch`, `web_search`) as a dedicated MCP server on
//! an ephemeral `127.0.0.1` port. It then registers that provider with the
//! runtime over the control plane, and the runtime reaches it as an ordinary
//! streamable-HTTP MCP server, so the fixed web tools stay on the canonical
//! `ToolManager` path.
//!
//! Security invariants:
//!
//! - the server binds `127.0.0.1:0` and is reachable only from this machine;
//! - every request must present the in-memory provider proof token in the
//!   `Authorization` header (never a URL, query or body);
//! - a browser `Origin` that is not this provider's own loopback origin is
//!   rejected, so a web page can never drive the provider;
//! - the proof token is random and short-lived, held in memory only, and bound
//!   to the desktop instance, the client id and the live lease;
//! - tool execution reuses the exact `WebBridgeDispatcher` body (runtime-config
//!   source plus the strict shared capability schema), so there is no second
//!   execution or schema path.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;
use chatspeed_contracts::{
    validate_capability_arguments, ClientCapabilityInvocation, ClientCapabilityStatus,
    WebMcpProviderRegistration, BRIDGE_SCHEMA_VERSION,
};
use rand::Rng;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, IntoContents,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
    Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler};
use serde_json::{json, Map, Value};
use tauri::{AppHandle, Wry};
use tokio::sync::watch;

use crate::runtime_client::{BridgeDispatcher, RuntimeSupervisor, RuntimeUnavailable};
use crate::runtime_web_bridge::WebBridgeDispatcher;

/// The MCP endpoint path the runtime dials.
const MCP_PATH: &str = "/mcp";

/// Bound on one provider-side tool execution (the dispatcher bounds its own
/// work more loosely, so the provider can report a typed timeout first).
const TOOL_TIMEOUT: Duration = Duration::from_secs(60);

/// One allowlisted provider tool and its JSON schema.
struct ProviderTool {
    name: &'static str,
    description: &'static str,
    schema: Value,
}

fn provider_tools() -> Vec<ProviderTool> {
    vec![
        ProviderTool {
            name: "web_fetch",
            description: "Extracts the full content or links from a single web page URL.",
            schema: json!({
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "URL to scrape content from"},
                    "format": {"type": "string", "enum": ["markdown", "text", "links"], "default": "markdown"},
                    "keep_link": {"type": "boolean", "default": false},
                    "keep_image": {"type": "boolean", "default": false}
                },
                "required": ["url"]
            }),
        },
        ProviderTool {
            name: "web_search",
            description: "Search the web for up-to-date information beyond the model knowledge cutoff.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"oneOf": [{"type": "string"}, {"type": "array", "items": {"type": "string"}}]},
                    "page": {"type": "integer", "minimum": 1, "default": 1},
                    "number": {"type": "integer", "minimum": 1, "maximum": 30, "default": 5},
                    "time_period": {"type": "string", "enum": ["day", "week", "month", "year"]},
                    "response_format": {"type": "string", "enum": ["json", "xml"], "default": "json"},
                    "provider": {"type": "string"}
                },
                "required": ["query"]
            }),
        },
    ]
}

/// The dedicated MCP server handler for the two fixed web tools.
#[derive(Clone)]
struct WebMcpHandler {
    dispatcher: Arc<WebBridgeDispatcher>,
}

impl WebMcpHandler {
    /// Maps one allowlisted tool call onto the shared capability executor.
    async fn execute(&self, capability: &str, arguments: Map<String, Value>) -> CallToolResult {
        // The shared strict schema is validated before anything else, so an
        // undeclared argument is rejected without touching the runtime config.
        if let Err(error) = validate_capability_arguments(capability, &arguments) {
            return CallToolResult::structured_error(
                json!({"code": error.code, "message": error.message}),
            );
        }
        let invocation = ClientCapabilityInvocation {
            request_id: generate_token(),
            capability: capability.to_string(),
            schema_version: BRIDGE_SCHEMA_VERSION.to_string(),
            arguments: Value::Object(arguments),
            deadline: String::new(),
        };
        let result = tokio::time::timeout(
            TOOL_TIMEOUT,
            BridgeDispatcher::dispatch(self.dispatcher.as_ref(), invocation),
        )
        .await;
        match result {
            Ok(result) => match result.status {
                ClientCapabilityStatus::Ok => {
                    let payload = result.result.unwrap_or(Value::Null);
                    if payload.is_object() {
                        CallToolResult::structured(payload)
                    } else {
                        CallToolResult::success(
                            payload.as_str().unwrap_or_default().to_string().into_contents(),
                        )
                    }
                }
                ClientCapabilityStatus::Error | ClientCapabilityStatus::Cancelled => {
                    let error = result.error.unwrap_or(chatspeed_contracts::ClientCapabilityError {
                        code: "capability_error".to_string(),
                        message: "the client capability failed".to_string(),
                    });
                    CallToolResult::structured_error(json!({"code": error.code, "message": error.message}))
                }
            },
            Err(_) => CallToolResult::structured_error(json!({
                "code": "timeout",
                "message": "the web capability did not finish before its deadline"
            })),
        }
    }
}

impl ServerHandler for WebMcpHandler {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::new("Chatspeed Web MCP Provider", env!("CARGO_PKG_VERSION"));
        info.instructions =
            Some("Dedicated loopback provider exposing exactly web_fetch and web_search.".to_string());
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = provider_tools()
            .into_iter()
            .map(|tool| {
                let schema = match tool.schema {
                    Value::Object(map) => Arc::new(map),
                    _ => Arc::new(Map::new()),
                };
                Tool::new(tool.name, tool.description, schema)
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.as_ref();
        let capability = match name {
            "web_fetch" | "web_search" => name,
            other => {
                return Ok(CallToolResponse::Complete(CallToolResult::structured_error(
                    json!({"code": "unknown_tool", "message": format!("`{other}` is not a provider tool")}),
                )))
            }
        };
        let arguments = request.arguments.unwrap_or_default();
        Ok(CallToolResponse::Complete(
            self.execute(capability, arguments).await,
        ))
    }
}

/// Authentication state for the provider's own loopback origin.
#[derive(Clone)]
struct ProviderAuth {
    token: String,
    /// The provider's own loopback origin; a browser `Origin` must match it.
    origin: String,
}

/// Rejects any request that does not present the proof token, or that carries a
/// browser `Origin` other than the provider's own loopback origin.
async fn require_proof(
    State(auth): State<ProviderAuth>,
    request: Request,
    next: Next,
) -> Response {
    if let Some(origin) = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        if !constant_time_eq(origin, &auth.origin) {
            return reject(StatusCode::FORBIDDEN, "origin_forbidden");
        }
    }
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.strip_prefix("Bearer ").unwrap_or(value))
        .map(|value| constant_time_eq(value, &auth.token))
        .unwrap_or(false);
    if !presented {
        return reject(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    next.run(request).await
}

fn reject(status: StatusCode, code: &'static str) -> Response {
    (
        status,
        axum::Json(json!({"error": {"code": code, "message": "the Web MCP provider rejected the request"}})),
    )
        .into_response()
}

/// Constant-time equality comparison for secret material.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A running desktop Web MCP provider bound to one live lease.
pub struct WebMcpProviderHandle {
    port: u16,
    token: String,
    instance_id: String,
    client_id: String,
    lease_id: String,
    client: chatspeed_runtime_client::RuntimeClient,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl WebMcpProviderHandle {
    /// The loopback port the provider bound.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stops the server and unregisters the provider from the runtime.
    ///
    /// Called before the lease is released, so the runtime never keeps dialing a
    /// provider whose lease is already gone.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
        if let Err(error) = self
            .client
            .unregister_web_mcp_provider(&self.client_id, &self.lease_id, &self.instance_id, &self.token)
            .await
        {
            log::debug!("[WebMcpProvider] unregistering the provider failed: {error}");
        }
    }
}

impl std::fmt::Debug for WebMcpProviderHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebMcpProviderHandle")
            .field("port", &self.port)
            .field("instance_id", &self.instance_id)
            .field("token", &"<redacted>")
            .finish()
    }
}

/// Starts the desktop Web MCP provider and registers it with the runtime.
///
/// The provider binds an ephemeral loopback port, serves only `web_fetch` and
/// `web_search`, and registers the port with the runtime under the caller's live
/// lease. The returned handle is stored on the supervisor so shutdown stops the
/// server before the lease is released.
pub async fn start(
    app_handle: AppHandle<Wry>,
    supervisor: &Arc<RuntimeSupervisor>,
) -> Result<(), RuntimeUnavailable> {
    let (client, client_id, lease_id) = supervisor.terminal_connection().await?;

    let dispatcher = Arc::new(WebBridgeDispatcher::new(app_handle, client.clone()));
    let handler = WebMcpHandler { dispatcher };
    let factory = {
        let handler = handler.clone();
        move || Ok(handler.clone())
    };
    let service = StreamableHttpService::new(
        factory,
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let token = generate_token();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| {
            RuntimeUnavailable::Unreachable(format!("cannot bind the provider listener: {error}"))
        })?;
    let port = listener
        .local_addr()
        .map_err(|error| RuntimeUnavailable::Unreachable(error.to_string()))?
        .port();

    let auth = ProviderAuth {
        token: token.clone(),
        origin: format!("http://127.0.0.1:{port}"),
    };
    let router = Router::new()
        .nest_service(MCP_PATH, service)
        .layer(axum::middleware::from_fn_with_state(auth, require_proof));

    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(async move {
        let served = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
        });
        if let Err(error) = served.await {
            log::warn!("[WebMcpProvider] server stopped with error: {error}");
        }
    });

    let instance_id = generate_token();
    let registration = WebMcpProviderRegistration { port };
    if let Err(error) = client
        .register_web_mcp_provider(&registration, &client_id, &lease_id, &instance_id, &token)
        .await
    {
        let _ = shutdown.send(true);
        task.abort();
        return Err(RuntimeUnavailable::from_client_error(error));
    }
    log::info!("[WebMcpProvider] registered loopback provider on 127.0.0.1:{port}");

    supervisor
        .set_web_provider(WebMcpProviderHandle {
            port,
            token,
            instance_id,
            client_id,
            lease_id,
            client,
            shutdown,
            task,
        })
        .await
}

/// Builds the provider's request Authorization header value for tests.
#[cfg(test)]
fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_exposes_exactly_the_two_fixed_web_tools() {
        let tools = provider_tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["web_fetch", "web_search"]);
    }

    #[test]
    fn constant_time_eq_matches_only_equal_inputs() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
    }

    #[test]
    fn proof_bearer_value_matches_the_middleware_expectation() {
        assert_eq!(bearer("secret"), "Bearer secret");
    }
}