//! Transport adapters for the workflow runtime.
//!
//! Phase 1 keeps the Tauri event transport here. Later units add the shared
//! runtime hub (`hub`) and the loopback HTTP control plane (`http`) next to it.

// The loopback HTTP control plane and the shared runtime hub are runtime-only:
// the desktop reaches the runtime through the `RuntimeSupervisor` client, not
// through these in-process adapters.
#[cfg(not(feature = "desktop"))]
pub mod http;
#[cfg(not(feature = "desktop"))]
pub mod hub;
// Desktop-only output adapter: it emits Tauri window events. The runtime talks
// to clients through the HTTP/SSE control plane instead.
#[cfg(feature = "desktop")]
pub mod tauri;
