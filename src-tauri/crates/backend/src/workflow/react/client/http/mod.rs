//! Independent loopback HTTP/JSON + SSE control plane for workflow runtime
//! access. See `server.rs` for the listener lifecycle and `discovery.rs` for
//! how local clients find and authenticate against it.

pub mod auth;
#[cfg(not(feature = "desktop"))]
pub mod chat_commands;
#[cfg(not(feature = "desktop"))]
pub mod client_bridge;
pub mod data_commands;
pub mod discovery;
pub mod dto;
pub mod server;
pub mod sse;
#[cfg(not(feature = "desktop"))]
pub mod terminal_commands;
#[cfg(not(feature = "desktop"))]
pub mod web_mcp_commands;
pub mod workflow_commands;
