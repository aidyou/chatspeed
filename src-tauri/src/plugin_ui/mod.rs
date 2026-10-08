//! Host-side loopback HTTP gateway for the built-in plugin's static UI.
//!
//! The runtime's control-plane route is bearer-protected and meant for trusted
//! processes, so a WebView tab cannot load it directly. This module hosts a
//! separate, narrowly scoped server on `127.0.0.1` that serves one verified
//! plugin UI bundle to one tab at a time, gated by a random per-tab capability
//! and by Host/Origin checks. `gateway.rs` owns the running server, the
//! capability table and the runtime asset reads; `types.rs` holds the Rust-only
//! wire types the host consumes; `state.rs` owns the tab sessions and the
//! runtime gate. Nothing here is reachable from the frontend.
//!
//! This file is a pure module list: it declares the submodules and re-exports
//! the host-facing surface, so no behavior lives in a `mod.rs`.

mod gateway;
mod host;
mod state;
mod types;

pub use gateway::PluginUiGateway;
pub use host::{PluginUiBounds, PluginUiHost};
pub use state::{
    is_valid_tab_id, plan_tab_open, PluginUiRuntime, PluginUiTabPlan, PluginUiTabSession,
    PluginUiTabs, MAX_PLUGIN_UI_TABS,
};
pub use types::PluginUiGrant;