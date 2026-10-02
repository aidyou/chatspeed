//! Desktop-free assembly of the canonical ChatSpeed runtime backend.
//!
//! This crate exists to *prove* the runtime boundary rather than to re-implement
//! it. Every module below is included from the canonical sources under
//! `src-tauri/src/` with `#[path]`, so there is exactly one implementation of
//! the database, AI, tool, MCP, capability, workflow and ccproxy layers. The
//! desktop crate compiles the same files with the `desktop` feature enabled;
//! this crate never enables it, which is what keeps Tauri, Wry and GTK out of
//! the runtime dependency graph.
//!
//! Nothing in this crate may add a second schema, a second executor or a second
//! application service. Desktop-only code inside the shared sources is expected
//! to be marked `#[cfg(feature = "desktop")]` instead of being copied here.

// The shared sources contain generous documentation and a few deliberate
// no-ops; keep this crate warning-clean without silencing real defects.
#![allow(clippy::too_many_arguments)]

use rust_i18n::i18n;

// The canonical translation catalog lives next to the desktop crate. The macro
// path is relative to this crate's manifest directory.
i18n!("../../i18n", fallback = "en");

#[path = "../../src/constants.rs"]
pub mod constants;

#[allow(unused_imports)]
use constants::*;

#[path = "../../src/error.rs"]
pub mod error;

#[path = "../../src/libs/mod.rs"]
pub mod libs;

#[path = "../../src/db/mod.rs"]
pub mod db;

#[path = "../../src/sensitive/mod.rs"]
pub mod sensitive;

#[path = "../../src/mcp/mod.rs"]
pub mod mcp;

#[path = "../../src/ai/mod.rs"]
pub mod ai;

#[path = "../../src/tools/mod.rs"]
pub mod tools;

#[path = "../../src/capability/mod.rs"]
pub mod capability;

#[path = "../../src/ccproxy/mod.rs"]
pub mod ccproxy;

#[path = "../../src/workflow/mod.rs"]
pub mod workflow;

// The canonical `commands` module also holds Tauri adapters that the runtime
// cannot link (window, clipboard, updater, ...). This crate compiles only the
// shared workflow command core, which the workflow application service and the
// automation service call directly.
pub mod commands;

/// The single canonical runtime owner assembly.
pub mod owner;

/// The long-lived background tasks the runtime starts after assembly.
pub mod background;

/// The canonical runtime data-command dispatcher, shared with the desktop crate
/// through `#[path]`. Re-exported at the crate root so the shared control-plane
/// server refers to `crate::runtime_data` identically in both crates.
pub mod data;
pub use data::runtime_data;

/// The allowlisted `/control/v1/data-commands/{command}` router extension the
/// canonical control-plane server merges into its state-typed router.
pub mod data_commands;

pub use background::RuntimeBackground;
pub use db::MainStore;
pub use owner::{RuntimeOwner, RuntimeOwnerConfig, RuntimeOwnerError};
