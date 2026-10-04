mod constants;
// The fixed `web_fetch`/`web_search` tools that reach the desktop WebView only
// through the runtime-owned client capability bridge.
#[cfg(all(test, not(feature = "desktop")))]
mod client_bridge_web;
mod error;
mod fs;
// Runtime-only tool implementations: the desktop reaches them through the
// control plane and never links the execution machinery, so only the runtime
// crate compiles them. The runtime-backend crate never enables `desktop`.
#[cfg(not(feature = "desktop"))]
mod git_diff;
#[cfg(not(feature = "desktop"))]
mod git_inspect;
#[cfg(not(feature = "desktop"))]
pub(crate) mod helper;
#[cfg(not(feature = "desktop"))]
mod history;
#[cfg(not(feature = "desktop"))]
mod interaction;
mod llm_output;
// The MCP expander/executor tools are runtime-only; the desktop detects their
// names but does not register or run them.
#[cfg(not(feature = "desktop"))]
mod mcp_loader;
mod sandbox;
mod search;
// Shared shell approval DTOs. Both crates compile these even though only the
// runtime compiles the shell policy engine and executor.
#[cfg(not(feature = "desktop"))]
mod shell;
#[cfg(not(feature = "desktop"))]
mod shell_output;
mod shell_policy;
#[cfg(not(feature = "desktop"))]
mod skill;
#[cfg(not(feature = "desktop"))]
mod todo_manager;
mod tool_manager;
mod types;
// Desktop-only WebView-backed tools. The runtime reaches web access through a
// client capability bridge instead, so it must not link the scraper/search
// stacks that back these two implementations.
#[cfg(feature = "desktop")]
pub mod web_config;
#[cfg(feature = "desktop")]
mod web_fetch;
#[cfg(feature = "desktop")]
mod web_search;

pub use constants::*;
pub use error::ToolError;
// The filesystem and search tools are registered by the runtime's `ToolManager`
// only; the desktop has no live path that names them.
#[cfg(all(test, not(feature = "desktop")))]
pub use client_bridge_web::ClientBridgeWebTool;
#[cfg(not(feature = "desktop"))]
pub use fs::*;
#[cfg(not(feature = "desktop"))]
pub use git_diff::GitDiff;
#[cfg(not(feature = "desktop"))]
pub use git_inspect::GitInspect;
#[cfg(not(feature = "desktop"))]
pub use history::ReadHistoryMessage;
#[cfg(not(feature = "desktop"))]
pub use interaction::*;
#[cfg(not(feature = "desktop"))]
pub use mcp_loader::{McpToolExecute, McpToolExpand};
pub use sandbox::*;
#[cfg(not(feature = "desktop"))]
pub use search::*;
#[cfg(not(feature = "desktop"))]
pub use shell::*;
pub use shell_policy::*;
#[cfg(not(feature = "desktop"))]
pub use skill::*;
#[cfg(not(feature = "desktop"))]
pub use todo_manager::*;
pub use tool_manager::{NativeToolResult, ToolDefinition, ToolManager};
pub use types::ToolScope;
pub use types::{ToolCallResult, ToolCategory};
#[cfg(feature = "desktop")]
pub use web_fetch::WebFetch;
#[cfg(feature = "desktop")]
pub use web_search::WebSearch;
