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
pub(crate) use types::{McpClient, McpClientResult, McpStopOutcome, StatusChangeCallback};
// The internal trait is re-exported only for the runtime tests that build a mock
// canonical client; production code reaches it through `types` directly.
#[cfg(all(test, not(feature = "desktop")))]
pub(crate) use types::McpClientInternal;
pub use types::{McpProtocolType, McpServerConfig, McpStatus};
