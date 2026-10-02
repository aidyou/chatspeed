mod constants;
mod error;
mod fs;
mod git_diff;
mod git_inspect;
pub(crate) mod helper;
mod history;
mod interaction;
mod llm_output;
mod mcp_loader;
mod sandbox;
mod search;
mod shell;
mod shell_output;
mod skill;
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
pub use fs::*;
pub use git_diff::GitDiff;
pub use git_inspect::GitInspect;
pub use history::ReadHistoryMessage;
pub use interaction::*;
pub use mcp_loader::{McpToolExecute, McpToolExpand};
pub use sandbox::*;
pub use search::*;
pub use shell::*;
pub use skill::*;
pub use todo_manager::*;
pub use tool_manager::{NativeToolResult, ToolDefinition, ToolManager};
pub use types::ToolScope;
pub use types::{ToolCallResult, ToolCategory};
#[cfg(feature = "desktop")]
pub use web_fetch::WebFetch;
#[cfg(feature = "desktop")]
pub use web_search::WebSearch;
