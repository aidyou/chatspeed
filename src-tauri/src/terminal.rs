//! Desktop-side terminal wire types and compatibility markers.
//!
//! Interactive PTY resources are runtime-owned. This module intentionally contains
//! no `portable_pty`, process handles, database access or Tauri event emitter; the
//! Tauri command adapter calls `crate::runtime_terminal` instead.
//!
//! `TerminalManager` remains as a source-level compatibility marker for the
//! workflow-terminal contract. The actual owner lives in
//! `chatspeed-runtime-backend::terminal::TerminalManager`.

pub(crate) type TerminalShell = chatspeed_contracts::TerminalShellDto;
pub(crate) type TerminalSessionMetadata = chatspeed_contracts::TerminalSessionMetadataDto;

/// Compatibility marker: desktop does not construct or manage a PTY owner.
#[cfg(test)]
pub(crate) struct TerminalManager;

// The runtime preserves the old distinction: a reader I/O error or output-event delivery failure
// aborts a direct user terminal, not AI shell-tool executions. The canonical runtime
// implementation owns `abort_terminal_session` and the process cleanup.

// Kept as compatibility vocabulary for the terminal contract tests. Shell prompt
// handling still preserves CurrentFileSystemLocation.ProviderPath and the ESC
// (`[char]27`) OSC 7 sequence in the runtime-owned implementation.
