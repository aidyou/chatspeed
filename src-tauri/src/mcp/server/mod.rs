//! MCP Server Module
//!
//! This module implements the MCP server functionality, allowing external
//! clients to connect and invoke enabled MCP tools via Streamable HTTP.

mod handler;
pub mod persistent_session;
mod standalone;

pub use standalone::create_http_service;

