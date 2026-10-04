//! Tauri-specific workflow event transport.
//!
//! `gateway.rs` (the former in-process `TauriGateway`/`EventBatcher` output
//! adapter) is intentionally not compiled in any current configuration. The
//! desktop streams workflow events from the runtime over the control plane
//! instead of emitting them in-process, so the adapter has no consumer in the
//! desktop build; and it binds `tauri::AppHandle` plus the runtime-only
//! `GatewayPayload`/`WorkflowSignal` wire types, so it cannot compile in the
//! desktop-free runtime either. The source is retained for a future desktop
//! rewire.

// pub mod gateway;