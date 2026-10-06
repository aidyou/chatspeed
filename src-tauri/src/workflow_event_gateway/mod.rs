//! Tauri-specific workflow event transport (desktop-retained adapter).
//!
//! This is the desktop home of the former in-process `TauriGateway` output
//! adapter. It is not wired into the current desktop build: the desktop streams
//! workflow events from the runtime over the control plane, so nothing emits
//! them in-process any more. It is retained here — inside the desktop crate,
//! which is allowed to link Tauri — so the adapter can be rewired without the
//! desktop-free runtime backend ever carrying it.
//!
//! `gateway.rs` binds `tauri::AppHandle` plus the runtime wire types; it is kept
//! as an un-wired source file, exactly as it was when it lived in the backend.

pub mod gateway;