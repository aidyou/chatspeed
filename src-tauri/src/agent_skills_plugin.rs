//! Plugin lifecycle is owned by `chatspeed_runtime_backend::plugin`.
//!
//! Desktop commands live in `commands/plugin.rs` and forward through
//! `runtime_plugin.rs`; there is deliberately no desktop-local bundle host.
