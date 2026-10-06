//! Desktop `tools` adapter.
//!
//! Re-exports the runtime backend's transport-neutral tool contracts, registry
//! types and execution cores, and adds the WebView-backed web tools
//! (`web_fetch`, `web_search`) plus their configuration port. The runtime never
//! links those two implementations: it reaches web access through the runtime's
//! client capability bridge (`runtime_web_bridge`), which is the only place that
//! runs a bridged web tool on the runtime's behalf.

pub use chatspeed_runtime_backend::tools::*;

pub mod web_config;
mod web_fetch;
mod web_search;

pub use web_fetch::WebFetch;
pub use web_search::WebSearch;
