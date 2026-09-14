//! Independent loopback HTTP/JSON + SSE control plane for workflow runtime
//! access. See `server.rs` for the listener lifecycle and `discovery.rs` for
//! how local clients find and authenticate against it.

pub mod auth;
pub mod discovery;
pub mod dto;
pub mod server;
pub mod sse;
