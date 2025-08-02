//! MCP Server Module
//!
//! This module implements the MCP server functionality based on the SSE protocol, allowing external
//! clients to connect and invoke enabled MCP tools.

mod handler;
mod standalone;

pub use standalone::start_standalone_mcp_server;
