//! Removed: this module used to open a second SQLite database with its own
//! `runtime_snapshots` / `runtime_events` tables.
//!
//! That was a parallel schema for state the runtime already owns through the
//! shared `MainStore`: the canonical durable projections are
//! `src-tauri/src/db/*` (`workflow_snapshots`, `workflow_events`), and the
//! database constitution requires every access to go through
//! `MainStore`/`DbRuntime` rather than a second connection. The module was
//! never declared in `lib.rs` and was never built, so it is being dropped
//! instead of reimplemented.
//!
//! There is deliberately no replacement here. If the runtime boundary needs a
//! durable `RuntimeStore` implementation, it must be an adapter over the
//! canonical `MainStore` tables and it belongs with the runtime owner
//! (`chatspeed-runtime-backend`), not in this ports-only crate.
//!
//! This file is intentionally inert so the tree stays honest until it can be
//! deleted with `git rm`.

#![allow(dead_code)]
