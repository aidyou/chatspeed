// mod chat_completion;
// mod deep_search;
// mod search_dedup;
mod error;
mod tool_manager;
mod types;
mod web_fetch;
mod web_search;

// pub use chat_completion::ChatCompletion;
// pub use deep_search::core::DeepSearch;
// pub use search_dedup::SearchDedup;
pub use error::ToolError;
pub use tool_manager::{NativeToolResult, ToolDefinition, ToolManager, MCP_TOOL_NAME_SPLIT};
pub use types::{ToolCallResult, ToolCategory};
pub use web_fetch::WebFetch;
pub use web_search::WebSearch;
