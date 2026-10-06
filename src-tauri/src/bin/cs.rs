//! Legacy `cs` binary entry point (desktop crate).
//!
//! The maintained CLI is the independent `chatspeed-cli` workspace package at
//! `src-tauri/cli`, which builds the `cscli` binary from `cli/src`. This
//! desktop-crate target is kept only so the non-default `legacy-cli-binary`
//! feature still has a valid path; it is never built by default and no longer
//! contains a second, independently compiled CLI implementation.

fn main() {
    eprintln!("cs: the legacy `cs` binary was replaced by `cscli` (src-tauri/cli)");
    std::process::exit(2);
}
