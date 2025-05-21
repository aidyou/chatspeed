mod core;
mod sse;
mod stdio;
mod types;
mod util;

pub use sse::SseClient;
pub use stdio::StdioClient;
pub use types::{
    McpClient, McpClientError, McpClientResult, McpProtocolType, McpServerConfig, McpStatus,
};
