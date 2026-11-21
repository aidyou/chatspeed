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

/// Runs one allowlisted workflow command.
///
/// Mutating commands require an idempotency key the server checks before it
/// replays anything, so the caller supplies a stable one per user action.
pub async fn workflow_command(
    options: &Options,
    command: &str,
    params: Value,
    idempotency_key: Option<&str>,
) -> Result<Value, String> {
    let client = connect(options)
        .await
        .ok_or_else(|| "runtime unavailable".to_string())?;
    let path = format!("/control/v1/workflow-commands/{command}");
    match idempotency_key {
        Some(key) => client
            .post_with_idempotency(&path, &params, key)
            .await
            .map_err(|error| error.to_string()),
        None => client
            .post(&path, &params)
            .await
            .map_err(|error| error.to_string()),
    }
}

/// The runtime configuration the desktop settings UI writes.
pub async fn config(options: &Options) -> Result<Value, String> {
    data_command(options, "get_all_config", json!({})).await
}

/// Match the desktop's estimate from current pricing, not stored historical costs.
pub fn today_cost(rows: &Value, providers: &Value) -> f64 {
    use std::collections::HashMap;
    let mut by_id = HashMap::new();
    let mut by_name = HashMap::new();
    let rate = |p: &Value, field: &str| p[field].as_f64().unwrap_or(0.).max(0.);
    fn insert<'a, K: Eq + std::hash::Hash>(
        map: &mut HashMap<K, &'a Value>,
        key: K,
        pricing: &'a Value,
    ) {
        let priced = |p: &Value| {
            ["inputPerMillion", "outputPerMillion", "cachePerMillion"]
                .iter()
                .any(|field| p[*field].as_f64().is_some_and(|v| v > 0.))
        };
        let entry = map.entry(key).or_insert(pricing);
        if !priced(entry) && priced(pricing) {
            *entry = pricing;
        }
    }
    for provider in providers.as_array().into_iter().flatten() {
        for model in provider["models"].as_array().into_iter().flatten() {
            if let Some(id) = model["id"].as_str() {
                if let Some(provider_id) = provider["id"].as_i64() {
                    insert(&mut by_id, (provider_id, id), &model["pricing"]);
                }
            }
            if let Some(name) = provider["name"].as_str() {
                for key in ["id", "name"]
                    .into_iter()
                    .filter_map(|key| model[key].as_str())
                {
                    insert(&mut by_name, (name, key), &model["pricing"]);
                }
            }
        }
    }
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let model = row["backendModel"].as_str()?;
            let pricing = row["providerId"]
                .as_i64()
                .and_then(|id| by_id.get(&(id, model)))
                .or_else(|| by_name.get(&(row["provider"].as_str()?, model)))?;
            let input = rate(row, "totalInputTokens");
            let cache = rate(row, "totalCacheTokens");
            Some(
                ((input - cache).max(0.) * rate(pricing, "inputPerMillion")
                    + cache * rate(pricing, "cachePerMillion")
                    + rate(row, "totalOutputTokens") * rate(pricing, "outputPerMillion"))
                    / 1_000_000.
                    * pricing["multiplier"].as_f64().unwrap_or(1.).max(0.),
            )
        })
        .sum()
}

pub fn compact_cost(cost: f64) -> String {
    if !cost.is_finite() || cost == 0. {
        "$0.00".into()
    } else if cost >= 1000. {
        format!("${:.2}K", cost / 1000.)
    } else if cost >= 1. {
        format!("${cost:.2}")
    } else if cost >= 0.01 {
        format!("${cost:.4}")
    } else {
        format!("${cost:.6}")
    }
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
    let statuses = directory_git_status(root);
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
        let git_status = statuses.get(&name).cloned();
        list.push(DirEntry { name, path, is_dir, git_status });
    }
    list.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(list)
}

fn directory_git_status(root: &Path) -> std::collections::BTreeMap<String, String> {
    let Ok(output) = std::process::Command::new("git")
        .current_dir(root)
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "status.relativePaths=true", "status", "--short", "-z", "--no-renames", "--untracked-files=normal", "--ignore-submodules=all", "--", "."])
        .output()
    else {
        return Default::default();
    };
    if !output.status.success() {
        return Default::default();
    }
    parse_git_status(&output.stdout)
}

fn parse_git_status(output: &[u8]) -> std::collections::BTreeMap<String, String> {
    let mut statuses = std::collections::BTreeMap::new();
    for record in output.split(|byte| *byte == 0) {
        if record.get(2) != Some(&b' ') {
            continue;
        }
        let path = String::from_utf8_lossy(&record[3..]);
        let Some(name) = path.split('/').next().filter(|name| !name.is_empty()) else {
            continue;
        };
        let status = String::from_utf8_lossy(&record[..2]).trim().to_owned();
        statuses.entry(name.to_owned()).or_insert(status);
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_status_preserves_spaces_and_groups_child_paths() {
        let statuses = parse_git_status(b" M src/main.rs\0?? new file.txt\0A  added\0bad\0");
        assert_eq!(statuses.get("src").map(String::as_str), Some("M"));
        assert_eq!(statuses.get("new file.txt").map(String::as_str), Some("??"));
        assert_eq!(statuses.get("added").map(String::as_str), Some("A"));
        assert_eq!(statuses.len(), 3);
    }

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
    fn today_cost_uses_provider_id_and_cached_input_pricing() {
        let rows = json!([{"providerId": 9, "provider": "old name", "backendModel": "m",
            "totalInputTokens": 1000000, "totalCacheTokens": 500000,
            "totalOutputTokens": 250000, "totalEstimatedCost": 1000}]);
        let providers = json!([{"id": 9, "name": "renamed", "models": [{"id": "m",
            "pricing": {"inputPerMillion": 2, "cachePerMillion": 1, "outputPerMillion": 4, "multiplier": 2}}]}]);
        assert_eq!(compact_cost(today_cost(&rows, &providers)), "$5.00");
        assert_eq!(compact_cost(today_cost(&rows, &json!([]))), "$0.00");
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
