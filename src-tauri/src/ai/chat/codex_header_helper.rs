//! Codex-compatible HTTP headers for OpenAI Responses API requests.
//!
//! ChatSpeed sends its Responses API requests through ccproxy to upstream
//! providers. When the upstream is OpenAI, the request should be
//! indistinguishable from a request sent by the official Codex CLI client.
//! This module builds the headers Codex always sends on the HTTP
//! `/responses` path, mirroring the algorithm in
//! `codex-rs/login/src/auth/default_client.rs` (`get_codex_user_agent`) and
//! `codex-rs/codex-api/src/requests/headers.rs` (`build_session_headers`).
//!
//! Priority contract: headers built here are defaults only. Custom headers
//! configured by the user (provider metadata `customHeaders`, or client-side
//! `ChatMetadata.customHeaders`) always win and must never be overwritten.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use serde_json::{json, Value};

/// Originator advertised by the Codex CLI Rust client.
const CODEX_ORIGINATOR: &str = "codex_cli_rs";

/// Codex CLI version advertised in the User-Agent. Defaults to a current
/// release version; update when the mimicked client moves forward.
const CODEX_VERSION: &str = "0.150.1";

/// Terminal token used by Codex when no terminal can be detected. ChatSpeed
/// is a desktop app, so Codex's terminal detection would not find a terminal
/// either.
const CODEX_TERMINAL_TOKEN: &str = "unknown";

/// Stable UUID v5 derived from the conversation id.
///
/// Codex generates random UUIDs per session/thread, but ChatSpeed must keep
/// them stable per conversation: OpenAI uses `session_id`-derived values for
/// prompt cache routing, and unstable ids would collapse the cache hit rate.
/// This mirrors the existing `{CONV_ID}` placeholder scheme in
/// `crate::ai::util::process_custom_headers_value`.
fn stable_conversation_uuid(chat_id: &str, salt: &str) -> String {
    let seed = format!("chatspeed:codex:{salt}:{chat_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, seed.as_bytes()).to_string()
}

/// Best-effort OS version without extra dependencies (cached).
fn os_version() -> &'static str {
    static OS_VERSION: OnceLock<String> = OnceLock::new();
    OS_VERSION.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("sw_vers")
                .arg("-productVersion")
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|version| version.trim().to_string())
                .filter(|version| !version.is_empty())
                .unwrap_or_else(|| "unknown".to_string())
        }
        #[cfg(not(target_os = "macos"))]
        {
            "unknown".to_string()
        }
    })
}

/// User-Agent matching Codex's format:
/// `{originator}/{version} ({os_type} {os_version}; {arch}) {terminal_token}`.
fn codex_user_agent() -> String {
    format!(
        "{}/{CODEX_VERSION} ({} {}; {}) {}",
        CODEX_ORIGINATOR,
        std::env::consts::OS,
        os_version(),
        std::env::consts::ARCH,
        CODEX_TERMINAL_TOKEN
    )
}

/// Builds the Codex-compatible headers for a Responses API request.
///
/// `identity_seed` must be the prompt-cache identity (the `prompt_cache_key`
/// value). The official client sends the same session UUID as
/// `session-id`, `thread-id`, `x-client-request-id` and `prompt_cache_key`,
/// so the seed is used verbatim here; this keeps the header identities
/// stable per conversation/workflow session and aligned with cache routing.
pub(crate) fn codex_responses_headers(identity_seed: &str) -> Vec<(String, String)> {
    vec![
        ("originator".to_string(), CODEX_ORIGINATOR.to_string()),
        ("user-agent".to_string(), codex_user_agent()),
        ("session-id".to_string(), identity_seed.to_string()),
        ("thread-id".to_string(), identity_seed.to_string()),
        // Codex sets x-client-request-id to the thread id on the HTTP path.
        ("x-client-request-id".to_string(), identity_seed.to_string()),
        // Codex formats the window id as "{thread_id}:{counter}".
        (
            "x-codex-window-id".to_string(),
            format!("{identity_seed}:1"),
        ),
    ]
}

/// Current unix timestamp in milliseconds.
fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// Builds the `client_metadata` body field Codex always sends on Responses
/// API requests (see `codex-rs` `responses_metadata.rs`).
///
/// `identity_seed` must be the prompt-cache identity: the official client
/// sends the same session UUID as `prompt_cache_key` and
/// `client_metadata.session_id`, so the seed is used verbatim for the
/// session/thread fields. This keeps the metadata stable within one
/// conversation or workflow session, while `turn_id` / `root_turn_id` are
/// fresh v7 UUIDs per request. The official client also rotates turn ids on
/// every turn, so they cannot participate in prompt-cache routing; the
/// stable session/thread/window/installation ids are what matters for cache
/// affinity.
///
/// The `workspaces` entry of the turn metadata is intentionally omitted: it
/// contains the client's local paths and git remotes, which ChatSpeed cannot
/// report truthfully and should not fabricate.
pub(crate) fn codex_client_metadata(identity_seed: &str) -> Value {
    let installation_id = stable_conversation_uuid(identity_seed, "installation");
    let context_window_id = stable_conversation_uuid(identity_seed, "context-window");
    let window_id = format!("{identity_seed}:1");
    let turn_id = Uuid::now_v7().to_string();

    let turn_metadata = json!({
        "installation_id": installation_id,
        "session_id": identity_seed,
        "thread_id": identity_seed,
        "agent_name": "/root",
        "turn_id": turn_id,
        "window_id": window_id,
        "context_window_id": context_window_id,
        "request_kind": "turn",
        "root_turn_id": turn_id,
        "thread_source": "user",
        "sandbox": "seatbelt",
        "sandbox_mode": "workspace-write",
        "auto_review_enabled": true,
        "node_repl_auto_review_required": false,
        "node_repl_disabled": false,
        "turn_started_at_unix_ms": unix_ms(),
        "workspace_kind": "project",
    });

    json!({
        "root_turn_id": turn_id,
        "session_id": identity_seed,
        "thread_id": identity_seed,
        "turn_id": turn_id,
        "x-codex-installation-id": installation_id,
        "x-codex-turn-metadata": turn_metadata.to_string(),
        "x-codex-window-id": window_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_stable_per_conversation() {
        let first = codex_responses_headers("42");
        let second = codex_responses_headers("42");
        assert_eq!(first, second);

        let other = codex_responses_headers("43");
        assert_ne!(first, other);
    }

    #[test]
    fn headers_contain_codex_surface() {
        // The seed is used verbatim as session/thread id, so it must already
        // be a UUID (callers pass the prompt_cache_key UUID).
        let seed = Uuid::now_v7().to_string();
        let headers = codex_responses_headers(&seed);
        let names: Vec<&str> = headers.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "originator",
                "user-agent",
                "session-id",
                "thread-id",
                "x-client-request-id",
                "x-codex-window-id"
            ]
        );

        let originator = headers
            .iter()
            .find(|(name, _)| name == "originator")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert_eq!(originator, "codex_cli_rs");

        let user_agent = headers
            .iter()
            .find(|(name, _)| name == "user-agent")
            .map(|(_, value)| value.as_str())
            .unwrap();
        assert!(user_agent.starts_with("codex_cli_rs/0.150.1 ("));
        assert!(user_agent.contains(std::env::consts::ARCH));

        // session-id and thread-id must equal the seed (prompt_cache_key).
        let value_of = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        let session_id = value_of("session-id");
        let thread_id = value_of("thread-id");
        // Codex main-conversation requests use one session UUID for
        // session-id, thread-id and the prompt cache key.
        assert_eq!(session_id, seed);
        assert_eq!(thread_id, seed);
        assert_eq!(value_of("x-client-request-id"), thread_id);
        // Codex formats the window id as "{thread_id}:{counter}".
        assert_eq!(value_of("x-codex-window-id"), format!("{thread_id}:1"));
    }

    #[test]
    fn client_metadata_matches_codex_surface() {
        let seed = Uuid::now_v7().to_string();
        let metadata = codex_client_metadata(&seed);
        let headers = codex_responses_headers(&seed);
        let header_value = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap()
        };

        // Identity fields must agree with the headers.
        assert_eq!(
            metadata["session_id"].as_str().unwrap(),
            header_value("session-id")
        );
        assert_eq!(
            metadata["thread_id"].as_str().unwrap(),
            header_value("thread-id")
        );
        assert_eq!(
            metadata["x-codex-window-id"].as_str().unwrap(),
            header_value("x-codex-window-id")
        );

        // Turn ids are fresh v7 UUIDs, equal to their root.
        let turn_id = metadata["turn_id"].as_str().unwrap();
        let parsed = Uuid::parse_str(turn_id).unwrap();
        assert_eq!(parsed.get_version_num(), 7);
        assert_eq!(metadata["root_turn_id"].as_str().unwrap(), turn_id);

        // The turn metadata is a JSON-encoded string with the observed keys.
        let turn_metadata: Value =
            serde_json::from_str(metadata["x-codex-turn-metadata"].as_str().unwrap()).unwrap();
        for key in [
            "installation_id",
            "session_id",
            "thread_id",
            "agent_name",
            "turn_id",
            "window_id",
            "context_window_id",
            "request_kind",
            "root_turn_id",
            "thread_source",
            "sandbox",
            "sandbox_mode",
            "auto_review_enabled",
            "node_repl_auto_review_required",
            "node_repl_disabled",
            "turn_started_at_unix_ms",
            "workspace_kind",
        ] {
            assert!(turn_metadata.get(key).is_some(), "missing key: {key}");
        }
        assert_eq!(turn_metadata["turn_id"].as_str().unwrap(), turn_id);
        assert!(!turn_metadata.get("workspaces").is_some());
    }
}
