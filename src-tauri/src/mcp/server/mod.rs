//! MCP Server Module
//!
//! This module implements the MCP server functionality based on the SSE protocol, allowing external
//! clients to connect and invoke enabled MCP tools.

mod handler;
mod persistent_session;
mod standalone;

pub use standalone::{create_http_service, create_sse_router};

