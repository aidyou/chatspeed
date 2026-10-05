//! Runtime observation port for MCP servers.
//!
//! The capability service must be able to *report* runtime state without
//! owning the runtime: `ToolManager` and the MCP child processes stay with the
//! desktop main process (INV-1). This port is the only way the capability
//! domain observes them, which keeps the read model testable with a fake and
//! keeps `desired` strictly separate from `observed` (INV-7).
//!
//! Every answer is a one-shot observation with a timestamp. Nothing here is
//! persisted as a permanent fact.

use std::collections::BTreeMap;
#[cfg(not(feature = "desktop"))]
use std::sync::Arc;

use serde_json::Value;

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::capability::error::CapabilityError;
#[cfg(not(feature = "desktop"))]
use crate::capability::redaction;
use crate::mcp::client::McpServerConfig;
#[cfg(not(feature = "desktop"))]
use crate::mcp::client::McpStatus;

/// One observed runtime answer for a single server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedMcpRuntime {
    /// The runtime state name, e.g. `connected`, `stopped`, `error`.
    pub state: String,
    /// How many tools the runtime currently has cached for this server.
    pub cached_tool_count: usize,
}

/// Read-only observation of the MCP runtime.
#[async_trait::async_trait]
pub trait McpRuntimePort: Send + Sync {
    /// The observed runtime state of every registered MCP server, by name.
    async fn observed_runtime(
        &self,
    ) -> Result<BTreeMap<String, ObservedMcpRuntime>, CapabilityError>;
}

/// The shared refusal of a process that owns no MCP runtime.
fn runtime_unavailable() -> CapabilityError {
    CapabilityError::new(
        crate::capability::error::code::RUNTIME_UNAVAILABLE,
        "no MCP runtime is available in this process",
    )
}

/// A port that observes nothing, used where no runtime is available.
///
/// It refuses rather than answering with an empty map: an empty map means "the
/// runtime answered and nothing is registered", which is a *fact* a caller could
/// report as drift. "There is no runtime here" is a different statement, and
/// conflating them would let a headless process assert that a server the user
/// wants enabled is provably not running (INV-7).
pub struct UnavailableRuntimePort;

#[async_trait::async_trait]
impl McpRuntimePort for UnavailableRuntimePort {
    async fn observed_runtime(
        &self,
    ) -> Result<BTreeMap<String, ObservedMcpRuntime>, CapabilityError> {
        Err(runtime_unavailable())
    }
}

/// The real observation port, reading the desktop `ToolManager`.
///
/// It only reads: it asks for the current status of every registered server and
/// for the tool cache size. It never registers, starts, stops or calls a
/// server, and it never forwards an error message that could carry a secret.
#[cfg(not(feature = "desktop"))]
pub struct ToolManagerRuntimePort {
    chat_state: Arc<crate::ai::interaction::chat_completion::ChatState>,
}

#[cfg(not(feature = "desktop"))]
impl ToolManagerRuntimePort {
    pub fn new(chat_state: Arc<crate::ai::interaction::chat_completion::ChatState>) -> Self {
        Self { chat_state }
    }
}

#[cfg(not(feature = "desktop"))]
#[async_trait::async_trait]
impl McpRuntimePort for ToolManagerRuntimePort {
    async fn observed_runtime(
        &self,
    ) -> Result<BTreeMap<String, ObservedMcpRuntime>, CapabilityError> {
        let tool_manager = self.chat_state.tool_manager.clone();
        let statuses = tool_manager
            .get_mcp_serves_status()
            .await
            .map_err(|error| {
                CapabilityError::internal(redaction::redact_text(&error.to_string()))
            })?;

        let mut observed = BTreeMap::new();
        for (name, status) in statuses {
            let cached_tool_count = tool_manager
                .get_mcp_server_tools(&name)
                .await
                .map(|tools| tools.len())
                .unwrap_or(0);
            observed.insert(
                name,
                ObservedMcpRuntime {
                    state: status_name(&status).to_string(),
                    cached_tool_count,
                },
            );
        }
        Ok(observed)
    }
}

/// Maps a runtime status onto a stable state name.
///
/// An `Error` variant is reported as `error` only: the underlying message is
/// free text that may quote a command line, so it never enters a read model.
#[cfg(not(feature = "desktop"))]
fn status_name(status: &McpStatus) -> &'static str {
    match status {
        McpStatus::Starting => "starting",
        McpStatus::Connected => "connected",
        McpStatus::Running => "running",
        McpStatus::Stopped => "stopped",
        McpStatus::Error(_) => "error",
    }
}

/// A single runtime effect the capability service may request.
///
/// Every method maps to exactly one `ToolManager` primitive, and the service
/// always records the result in the operation journal. That replaces the
/// previous pattern of a command spawning an untracked task after its database
/// write, where a failed start was only ever visible in a log line (INV-7/INV-8).
///
/// Nothing here owns a process: `ToolManager` and its MCP children stay with the
/// desktop main process (INV-1).
#[async_trait::async_trait]
pub trait McpRuntimeEffects: Send + Sync {
    /// Connects and registers one server.
    async fn start(&self, config: McpServerConfig) -> Result<(), CapabilityError>;

    /// Stops and unregisters one server.
    async fn stop(&self, name: &str) -> Result<(), CapabilityError>;

    /// Re-reads a running server's tool list into the runtime cache.
    async fn refresh_tools(&self, name: &str) -> Result<(), CapabilityError>;

    /// The runtime's cached tool declarations. Listing never invokes a tool.
    async fn list_tools(&self, name: &str) -> Result<Vec<MCPToolDeclaration>, CapabilityError>;

    /// Enables or disables one cached tool.
    async fn set_tool_disabled(
        &self,
        server: &str,
        tool: &str,
        disabled: bool,
    ) -> Result<(), CapabilityError>;

    /// Invokes one enabled tool that belongs to a running server and returns the
    /// runtime's serialized tool result unchanged.
    ///
    /// The runtime owner resolves the requested name (an internal tool name or
    /// one of the aliases it assigned), proves the tool belongs to `server` and
    /// is enabled, and only then reaches the MCP client. A name that belongs to
    /// another server, an alias with no owner, and a disabled tool are all
    /// refused before any invocation, so a manual call can never cross a server
    /// boundary or run a disabled tool (INV-1/AC-11).
    ///
    /// The result is never re-encoded: the caller sees exactly what the MCP
    /// server produced.
    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, CapabilityError>;

    /// One-shot observation of a single server, used to confirm an effect.
    async fn observe(&self, name: &str) -> Result<Option<ObservedMcpRuntime>, CapabilityError>;
}

/// A port with no runtime behind it, used where the process owns no
/// `ToolManager`.
///
/// It refuses every effect instead of pretending success, so a headless or
/// test-only service can never report a started server that was never started.
pub struct UnavailableRuntimeEffects;

#[async_trait::async_trait]
impl McpRuntimeEffects for UnavailableRuntimeEffects {
    async fn start(&self, _config: McpServerConfig) -> Result<(), CapabilityError> {
        Err(runtime_unavailable())
    }

    async fn stop(&self, _name: &str) -> Result<(), CapabilityError> {
        Err(runtime_unavailable())
    }

    async fn refresh_tools(&self, _name: &str) -> Result<(), CapabilityError> {
        Err(runtime_unavailable())
    }

    async fn list_tools(&self, _name: &str) -> Result<Vec<MCPToolDeclaration>, CapabilityError> {
        Err(runtime_unavailable())
    }

    async fn set_tool_disabled(
        &self,
        _server: &str,
        _tool: &str,
        _disabled: bool,
    ) -> Result<(), CapabilityError> {
        Err(runtime_unavailable())
    }

    async fn call_tool(
        &self,
        _server: &str,
        _tool: &str,
        _arguments: Value,
    ) -> Result<Value, CapabilityError> {
        Err(runtime_unavailable())
    }

    async fn observe(&self, _name: &str) -> Result<Option<ObservedMcpRuntime>, CapabilityError> {
        // Unknown, not absent: absence would be proof that nothing is running.
        Err(runtime_unavailable())
    }
}

/// The real effect port, driving the desktop `ToolManager`.
#[cfg(not(feature = "desktop"))]
pub struct ToolManagerRuntimeEffects {
    chat_state: Arc<crate::ai::interaction::chat_completion::ChatState>,
}

#[cfg(not(feature = "desktop"))]
impl ToolManagerRuntimeEffects {
    pub fn new(chat_state: Arc<crate::ai::interaction::chat_completion::ChatState>) -> Self {
        Self { chat_state }
    }
}

/// Converts a runtime failure into a bounded, redacted summary.
///
/// A connect failure message can quote a command line or a URL with embedded
/// credentials, so it is redacted before it can reach an operation record (AC-13).
#[cfg(not(feature = "desktop"))]
fn runtime_failure(action: &str, error: crate::tools::ToolError) -> CapabilityError {
    CapabilityError::new(
        crate::capability::error::code::INTERNAL,
        format!(
            "the MCP runtime could not {action}: {}",
            redaction::redact_text(&error.to_string())
        ),
    )
}

/// The MCP client folds a tool's `is_error` result and a transport failure into
/// a bounded detail before this mapper is called. That detail contains only the
/// generic message or an allowlisted provider code, never raw provider text.
#[cfg(not(feature = "desktop"))]
fn call_tool_failure(error: crate::tools::ToolError) -> CapabilityError {
    use crate::capability::error::code;
    use crate::tools::ToolError;
    match error {
        ToolError::FunctionNotFound(name) => CapabilityError::new(
            code::NOT_FOUND,
            format!("MCP tool '{name}' does not exist on this server"),
        ),
        ToolError::Config(message) | ToolError::Security(message) => {
            CapabilityError::new(code::REFUSED, redaction::redact_text(&message))
        }
        ToolError::McpServerNotFound(name) => CapabilityError::new(
            code::RUNTIME_UNAVAILABLE,
            format!("the MCP server '{name}' is not registered with the runtime"),
        ),
        // `invoke_mcp_tool` has already reduced this detail to the generic
        // message plus an optional allowlisted provider code. Preserve that
        // diagnostic signal without forwarding arbitrary MCP error content.
        ToolError::ExecutionFailed(detail) => CapabilityError::new(code::INTERNAL, detail),
        _ => CapabilityError::new(code::INTERNAL, "the MCP tool call failed"),
    }
}

#[cfg(not(feature = "desktop"))]
#[async_trait::async_trait]
impl McpRuntimeEffects for ToolManagerRuntimeEffects {
    async fn start(&self, config: McpServerConfig) -> Result<(), CapabilityError> {
        self.chat_state
            .tool_manager
            .clone()
            .register_mcp_server(config)
            .await
            .map_err(|error| runtime_failure("start the server", error))
    }

    async fn stop(&self, name: &str) -> Result<(), CapabilityError> {
        self.chat_state
            .tool_manager
            .unregister_mcp_server(name)
            .await
            .map_err(|error| runtime_failure("stop the server", error))
    }

    async fn refresh_tools(&self, name: &str) -> Result<(), CapabilityError> {
        self.chat_state
            .tool_manager
            .refresh_mcp_server_tools(name)
            .await
            .map_err(|error| runtime_failure("refresh the tool list", error))
    }

    async fn list_tools(&self, name: &str) -> Result<Vec<MCPToolDeclaration>, CapabilityError> {
        self.chat_state
            .tool_manager
            .get_mcp_server_tools(name)
            .await
            .map_err(|error| runtime_failure("read the tool list", error))
    }

    async fn set_tool_disabled(
        &self,
        server: &str,
        tool: &str,
        disabled: bool,
    ) -> Result<(), CapabilityError> {
        self.chat_state
            .tool_manager
            .disable_mcp_tool(server, tool, disabled)
            .await
            .map_err(|error| runtime_failure("change the tool state", error))
    }

    async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<Value, CapabilityError> {
        self.chat_state
            .tool_manager
            .clone()
            .invoke_mcp_tool(server, tool, arguments)
            .await
            .map_err(call_tool_failure)
    }

    async fn observe(&self, name: &str) -> Result<Option<ObservedMcpRuntime>, CapabilityError> {
        let tool_manager = self.chat_state.tool_manager.clone();
        let statuses = tool_manager
            .get_mcp_serves_status()
            .await
            .map_err(|error| {
                CapabilityError::internal(redaction::redact_text(&error.to_string()))
            })?;
        let Some(status) = statuses.get(name) else {
            return Ok(None);
        };
        let cached_tool_count = tool_manager
            .get_mcp_server_tools(name)
            .await
            .map(|tools| tools.len())
            .unwrap_or(0);
        Ok(Some(ObservedMcpRuntime {
            state: status_name(status).to_string(),
            cached_tool_count,
        }))
    }
}

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;

    #[test]
    fn execution_failure_preserves_the_bounded_provider_code() {
        let error = call_tool_failure(crate::tools::ToolError::ExecutionFailed(
            "the MCP tool call failed (config_unavailable)".to_string(),
        ));
        assert_eq!(error.code(), crate::capability::error::code::INTERNAL);
        assert_eq!(
            error.message,
            "the MCP tool call failed (config_unavailable)"
        );
    }

    #[test]
    fn an_error_status_never_reports_its_message() {
        let status = McpStatus::Error("command failed with token=canary".to_string());
        assert_eq!(status_name(&status), "error");
    }
}
