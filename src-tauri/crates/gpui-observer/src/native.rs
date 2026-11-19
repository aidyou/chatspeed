//! Native command transport for the workflow window.
//!
//! The native window must not borrow the desktop's Tauri command layer: doing so
//! would keep it tied to the Tauri shell. Everything it needs goes through three
//! explicit lanes instead:
//!
//! - runtime-owned data commands over `POST /control/v1/data-commands/{command}`,
//!   the same allowlist the desktop adapter uses (`get_all_config`, today's cost
//!   statistics, ChatHub entries, ...);
//! - control-plane routes under `/control/v1/...` for workflow reads and signals;
//! - host-side work in this process (directory listing, opening a path), which
//!   is what a native window does natively and a webview needed a command for.
//!
//! Only the allowlisted command names are ever appended to the data-command
//! path; there is no generic remote-procedure surface here.

use chatspeed_runtime_client::{data_command_path, load_discovery, RuntimeClient};
use serde_json::{json, Value};
use std::path::Path;

use crate::types::{DirEntry, Options};

/// Opens a client against the runtime the desktop discovered.
pub async fn connect(options: &Options) -> Option<RuntimeClient> {
    let discovery = load_discovery(options.discovery.as_deref()).ok()?;
    RuntimeClient::connect(&discovery).await.ok()
}

/// Runs one allowlisted runtime data command.
pub async fn data_command(
    options: &Options,
    command: &str,
    params: Value,
) -> Result<Value, String> {
    let client = connect(options)
        .await
        .ok_or_else(|| "runtime unavailable".to_string())?;
    client
        .post(&data_command_path(command), &params)
        .await
        .map_err(|error| error.to_string())
}

/// Runs a control-plane read (`/control/v1/...`).
pub async fn control_get(options: &Options, path: &str) -> Result<Value, String> {
    let client = connect(options)
        .await
        .ok_or_else(|| "runtime unavailable".to_string())?;
    client.get(path).await.map_err(|error| error.to_string())
}

/// Runs a control-plane mutation with an idempotency key.
pub async fn control_post(options: &Options, path: &str, body: Value) -> Result<Value, String> {
    let client = connect(options)
        .await
        .ok_or_else(|| "runtime unavailable".to_string())?;
    client
        .post(path, &body)
        .await
        .map_err(|error| error.to_string())
}

/// The runtime configuration the desktop settings UI writes.
pub async fn config(options: &Options) -> Result<Value, String> {
    data_command(options, "get_all_config", json!({})).await
}

/// Directory names the workflow treats as workspace noise, mirroring
/// `is_workspace_noise_name` in the runtime so the tree shows the same entries
/// the agent's `list_dir` tool reports.
fn is_noise(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "target"
            | "__pycache__"
            | ".venv"
            | ".turbo"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | "dist"
            | "coverage"
    ) || lower.ends_with(".pyc")
        || matches!(lower.as_str(), ".ds_store" | "thumbs.db" | "desktop.ini")
}

/// Lists one directory for the authorized-path tree: noise entries removed,
/// directories first, then names, both case-sensitively like the runtime walker.
///
/// Blocking filesystem work; the worker runs it on a blocking task, never on the
/// UI thread.
pub fn list_dir(path: &str) -> Result<Vec<DirEntry>, String> {
    let root = Path::new(path);
    let metadata = std::fs::metadata(root).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err(format!("{path} is not a directory"));
    }
    let entries = std::fs::read_dir(root).map_err(|error| error.to_string())?;
    let mut list = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if is_noise(&name) {
            continue;
        }
        let path = entry.path().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .map(|kind| kind.is_dir())
            // A symlink to a directory follows the link so it can be expanded.
            .unwrap_or_else(|_| entry.path().is_dir());
        list.push(DirEntry { name, path, is_dir });
    }
    list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `gpui_kit` is not imported here, so the built-in attribute is unambiguous.
    #[test]
    fn listing_sorts_directories_first_and_hides_noise() {
        let root = std::env::temp_dir().join(format!("cs-list-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).expect("src");
        std::fs::create_dir_all(root.join("node_modules")).expect("noise");
        std::fs::write(root.join("b.txt"), b"b").expect("b");
        std::fs::write(root.join("A.txt"), b"a").expect("a");

        let entries = list_dir(&root.to_string_lossy()).expect("listing");
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["src", "A.txt", "b.txt"]);
        assert!(entries[0].is_dir);

        assert!(list_dir(root.join("b.txt").to_string_lossy().as_ref()).is_err());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn noise_filter_matches_the_runtime() {
        for name in [".git", "node_modules", "target", "dist", "x.pyc"] {
            assert!(is_noise(name), "{name}");
        }
        for name in ["src", "Cargo.toml", "README.md"] {
            assert!(!is_noise(name), "{name}");
        }
    }
}
