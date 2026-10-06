//! MCP Server Module
//!
//! This module implements the MCP server functionality, allowing external
//! clients to connect and invoke enabled MCP tools via Streamable HTTP.

// The MCP server handler, persistent sessions and the Streamable HTTP service
// are compiled only by the runtime. The desktop mounts no MCP server: it reaches
// tool execution through the control plane, and the ccproxy router that used to
// mount this service is runtime-only as well.
#[cfg(not(feature = "desktop"))]
mod handler;
#[cfg(not(feature = "desktop"))]
pub mod persistent_session;
#[cfg(not(feature = "desktop"))]
mod standalone;

#[cfg(not(feature = "desktop"))]
pub use standalone::create_http_service;
