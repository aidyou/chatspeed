// The MCP client implementations own server registration and the connection
// lifecycle, which only the runtime does. The desktop reaches MCP through the
// control plane and links just the shared config/status types below.
#[cfg(not(feature = "desktop"))]
mod core;
#[cfg(not(feature = "desktop"))]
mod stdio;
#[cfg(not(feature = "desktop"))]
mod streamable_http;
mod types;
#[cfg(not(feature = "desktop"))]
mod util;

#[cfg(not(feature = "desktop"))]
pub use stdio::StdioClient;
#[cfg(not(feature = "desktop"))]
pub use streamable_http::StreamableHttpClient;
#[cfg(not(feature = "desktop"))]
pub(crate) use types::{McpClient, McpClientResult, StatusChangeCallback};
pub(crate) use types::{McpProtocolType, McpServerConfig, McpStatus};
