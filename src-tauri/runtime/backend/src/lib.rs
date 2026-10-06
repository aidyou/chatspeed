//! Desktop-free assembly of the canonical ChatSpeed runtime backend.
//!
//! This crate *physically owns* the canonical application modules: the
//! database, AI, tool, MCP, capability, workflow, ccproxy, sensitive and
//! environment layers, the runtime data-command dispatcher and the built-in
//! agent synchronization all live under this crate's `src/` tree, with exactly
//! one implementation of each layer instead of a desktop and a runtime copy.
//!
//! The desktop crate links this package as an ordinary Cargo dependency and
//! re-exports the runtime-owned APIs; it does not compile these sources by path.
//! This crate never enables the `desktop` marker feature, which keeps Tauri, Wry
//! and GTK out of the runtime dependency graph. Every remaining `#[path]` in
//! this crate points at a sibling file inside this crate, never back into
//! `src-tauri/src`.
//!
//! Nothing in this crate may add a second schema, a second executor or a second
//! application service. Desktop-only code lives in the desktop crate's adapters
//! instead of being copied into the runtime backend.

// The shared sources contain generous documentation and a few deliberate
// no-ops; keep this crate warning-clean without silencing real defects.
#![allow(clippy::too_many_arguments)]

use rust_i18n::i18n;

// The canonical translation catalog lives in the desktop crate directory. The
// macro path is relative to this crate's manifest directory.
i18n!("../../i18n", fallback = "en");

pub mod constants;

#[allow(unused_imports)]
use constants::*;

pub mod error;

pub mod libs;

pub mod db;

pub mod sensitive;

pub mod mcp;

pub mod ai;

/// Runtime-owned Models.dev catalog parsing and resolution engine.
///
/// The desktop-free runtime is the only owner of catalog parsing, the
/// models.dev snapshot cache/loader and the endpoint-bound profile/transport
/// resolvers. This sibling file is declared at the crate root because the
/// shared sources name it as `crate::model_catalog_engine`; the `#[path]` is
/// intra-crate and never reaches back into `src-tauri/src`. The shared
/// `ai::model_catalog` module keeps only the DTOs both crates name.
#[path = "ai/model_catalog_engine.rs"]
pub mod model_catalog_engine;

/// Runtime-owned Models.dev catalog service.
///
/// The desktop-free runtime is the only owner of the catalog snapshot, its
/// loading and its refresh. The shared `ai::model_catalog_updater` re-exports
/// it so the runtime sources name exactly one implementation.
#[path = "ai/model_catalog_service.rs"]
pub mod model_catalog_service;

pub mod tools;

pub mod capability;

pub mod ccproxy;

pub mod workflow;

// The canonical `commands` module also holds Tauri adapters that the runtime
// cannot link (window, clipboard, updater, ...). The desktop crate owns those;
// this crate compiles only the shared workflow command core, which the workflow
// application service and the automation service call directly.
pub mod commands;

/// The canonical, transport-neutral environment and shell discovery helpers.
///
/// Pure `std` (plus `dirs`/`log`) and references no Tauri type. The interactive
/// terminal resolves its available shells, child environment and login-PATH
/// merge through it.
pub mod environment;

/// The runtime-owned, Tauri-free interactive user terminal PTY core (U-7).
///
/// The desktop terminal is bound to `tauri::AppHandle` and emits window events,
/// while this module publishes typed events into a bounded per-session
/// broadcast the control plane relays over SSE.
pub mod terminal;

/// The single canonical runtime owner assembly.
pub mod owner;

/// The long-lived background tasks the runtime starts after assembly.
pub mod background;

/// The single-slot desktop Web MCP provider registry (AC-8).
pub mod web_provider;

/// The canonical runtime data-command dispatcher. Re-exported at the crate root
/// so the shared control-plane server refers to `crate::runtime_data`
/// identically in both crates.
pub mod data;
pub use data::runtime_data;

/// The allowlisted `/control/v1/data-commands/{command}` router extension the
/// canonical control-plane server merges into its state-typed router.
pub mod data_commands;

pub use background::RuntimeBackground;
pub use db::MainStore;
pub use owner::{RuntimeOwner, RuntimeOwnerConfig, RuntimeOwnerError};
