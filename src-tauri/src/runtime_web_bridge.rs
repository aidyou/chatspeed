//! Desktop dispatcher for the runtime's client WebView capability bridge.
//!
//! The runtime owns the workflow and reaches a WebView-backed capability
//! (`web_fetch`, `web_search`) only by dispatching a typed invocation to this
//! dispatcher over the client-pull bridge. This module is deliberately narrow:
//!
//! - it accepts only the allowlisted web capabilities at the current schema;
//! - it rejects unknown capabilities, mismatched schema versions, and any
//!   argument outside the capability's declared schema;
//! - it reads the proxy/search configuration from the **runtime** configuration
//!   over the control plane (never a local database), and
//! - it returns only a typed result or a structured error.
//!
//! There is no generic RPC, arbitrary Tauri command, or SQL/filesystem
//! passthrough here. `web_fetch` still uses the Tauri WebView for the browser
//! path; `web_search` still uses the WebView for built-in providers. The only
//! thing that changed is where their configuration comes from.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chatspeed_contracts::{
    ClientCapabilityError, ClientCapabilityInvocation, ClientCapabilityResult,
    ClientCapabilityStatus, BRIDGE_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Wry};

use crate::runtime_client::{
    web_bridge_declaration, BridgeDispatchFuture, BridgeDispatcher, RuntimeSupervisor,
    RuntimeUnavailable,
};
use crate::tools::web_config::{MapWebToolConfig, WebToolConfig};
use crate::tools::{ToolDefinition, WebFetch, WebSearch};

/// Canonical control-plane route returning the runtime configuration map.
const GET_ALL_CONFIG_ROUTE: &str = "/control/v1/data-commands/get_all_config";

/// Bound on one client-side capability execution.
///
/// The runtime bounds the whole invocation more loosely; staying under it means
/// the client reports its own timeout as a typed error instead of the runtime
/// reporting a bare deadline.
const DISPATCH_TIMEOUT: Duration = Duration::from_secs(55);

/// The declared argument keys of each bridged capability.
const WEB_FETCH_ARGUMENTS: [&str; 4] = ["url", "format", "keep_link", "keep_image"];
const WEB_SEARCH_ARGUMENTS: [&str; 6] = [
    "query",
    "page",
    "number",
    "time_period",
    "response_format",
    "provider",
];

/// Executes allowlisted web capabilities on behalf of the runtime bridge.
#[derive(Clone)]
pub struct WebBridgeDispatcher {
    app_handle: AppHandle<Wry>,
    client: chatspeed_runtime_client::RuntimeClient,
}

impl WebBridgeDispatcher {
    /// Builds a dispatcher bound to one connected runtime client.
    pub fn new(
        app_handle: AppHandle<Wry>,
        client: chatspeed_runtime_client::RuntimeClient,
    ) -> Self {
        Self { app_handle, client }
    }

    /// Executes one invocation, mapping every failure to a typed result.
    async fn run(&self, invocation: ClientCapabilityInvocation) -> ClientCapabilityResult {
        let request_id = invocation.request_id.clone();
        match self.execute(invocation).await {
            Ok(value) => ClientCapabilityResult {
                request_id,
                status: ClientCapabilityStatus::Ok,
                result: Some(value),
                error: None,
            },
            Err(error) => ClientCapabilityResult {
                request_id,
                status: ClientCapabilityStatus::Error,
                result: None,
                error: Some(error),
            },
        }
    }

    /// Maps one typed invocation onto exactly one WebView-backed tool call.
    async fn execute(
        &self,
        invocation: ClientCapabilityInvocation,
    ) -> Result<Value, ClientCapabilityError> {
        if invocation.schema_version != BRIDGE_SCHEMA_VERSION {
            return Err(capability_error(
                "unsupported_schema",
                format!(
                    "unsupported capability schema version `{}`",
                    invocation.schema_version
                ),
            ));
        }
        let arguments = invocation
            .arguments
            .as_object()
            .ok_or_else(|| capability_error("invalid_arguments", "arguments must be an object"))?;

        // Read the runtime configuration once per invocation so the tools see
        // the values the runtime owner actually holds.
        let config = self.runtime_config().await?;
        let config: Arc<dyn WebToolConfig> = Arc::new(MapWebToolConfig::new(config));

        // Only the declared arguments are accepted; the runtime already
        // validated the schema, but this is defense in depth against a
        // compromised or buggy runtime.
        let params = match invocation.capability.as_str() {
            "web_fetch" => {
                reject_unknown(arguments, &WEB_FETCH_ARGUMENTS)?;
                Value::Object(arguments.clone())
            }
            "web_search" => {
                reject_unknown(arguments, &WEB_SEARCH_ARGUMENTS)?;
                Value::Object(arguments.clone())
            }
            other => {
                return Err(capability_error(
                    "unsupported_capability",
                    format!("`{other}` is not an allowlisted client capability"),
                ))
            }
        };

        let tool: Arc<dyn ToolDefinition> = match invocation.capability.as_str() {
            "web_fetch" => Arc::new(WebFetch::new(self.app_handle.clone(), config)),
            _ => WebSearch::new(self.app_handle.clone(), config),
        };

        match tokio::time::timeout(DISPATCH_TIMEOUT, tool.call(params)).await {
            Ok(Ok(call_result)) => serde_json::to_value(&call_result).map_err(|error| {
                capability_error(
                    "serialization",
                    format!("cannot encode the result: {error}"),
                )
            }),
            Ok(Err(tool_error)) => Err(capability_error("tool_error", tool_error.to_string())),
            Err(_elapsed) => Err(capability_error(
                "timeout",
                "the capability did not finish before its client-side deadline",
            )),
        }
    }

    /// Fetches the runtime configuration map over the control plane.
    async fn runtime_config(&self) -> Result<HashMap<String, Value>, ClientCapabilityError> {
        let value = self
            .client
            .post(GET_ALL_CONFIG_ROUTE, &json!({}))
            .await
            .map_err(|error| {
                capability_error(
                    "config_unavailable",
                    format!("cannot read the runtime configuration: {error}"),
                )
            })?;
        serde_json::from_value(value).map_err(|error| {
            capability_error(
                "config_invalid",
                format!("the runtime configuration is not a map: {error}"),
            )
        })
    }
}

impl BridgeDispatcher for WebBridgeDispatcher {
    fn dispatch(&self, invocation: ClientCapabilityInvocation) -> BridgeDispatchFuture {
        let this = self.clone();
        Box::pin(async move { this.run(invocation).await })
    }
}

/// Connects the desktop bridge to a supervisor that already holds a lease.
///
/// This is the one entry point `setup` calls after connecting; it registers the
/// fixed web declaration and starts the reader.
pub async fn start(
    app_handle: AppHandle<Wry>,
    supervisor: &Arc<RuntimeSupervisor>,
) -> Result<(), RuntimeUnavailable> {
    let client = supervisor.client().await?;
    let dispatcher = Arc::new(WebBridgeDispatcher::new(app_handle, client));
    supervisor
        .start_bridge(web_bridge_declaration(), dispatcher)
        .await
}

fn reject_unknown(
    arguments: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), ClientCapabilityError> {
    if let Some(unknown) = arguments
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(capability_error(
            "invalid_arguments",
            format!("unexpected argument `{unknown}`"),
        ));
    }
    Ok(())
}

fn capability_error(code: &str, message: impl Into<String>) -> ClientCapabilityError {
    ClientCapabilityError {
        code: code.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The dispatcher's argument gate is the security-relevant part; it is
    // exercised without a Tauri app handle.

    #[test]
    fn unknown_and_out_of_allowlist_capabilities_are_rejected() {
        let arguments = serde_json::Map::new();
        assert!(reject_unknown(&arguments, &WEB_FETCH_ARGUMENTS).is_ok());
        let mut with_extra = serde_json::Map::new();
        with_extra.insert("url".to_string(), json!("https://example.com"));
        with_extra.insert("selector".to_string(), json!("body"));
        let error = reject_unknown(&with_extra, &WEB_FETCH_ARGUMENTS)
            .expect_err("an undeclared argument is rejected");
        assert_eq!(error.code, "invalid_arguments");
    }

    #[test]
    fn web_search_accepts_only_its_declared_arguments() {
        let mut ok = serde_json::Map::new();
        ok.insert("query".to_string(), json!("rust"));
        ok.insert("number".to_string(), json!(5));
        ok.insert("provider".to_string(), json!("bing"));
        assert!(reject_unknown(&ok, &WEB_SEARCH_ARGUMENTS).is_ok());

        let mut bad = serde_json::Map::new();
        bad.insert("query".to_string(), json!("rust"));
        bad.insert("limit".to_string(), json!(5));
        assert!(reject_unknown(&bad, &WEB_SEARCH_ARGUMENTS).is_err());
    }
}
