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
use uuid::Uuid;

/// Originator advertised by the Codex CLI Rust client.
const CODEX_ORIGINATOR: &str = "codex_cli_rs";

/// Codex CLI version as reported by the reference source tree
/// (codex-rs workspace `CARGO_PKG_VERSION`; source builds report 0.0.0).
const CODEX_VERSION: &str = "0.0.0";

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
/// Returns `(name, value)` pairs. All values are derived deterministically
/// from `chat_id` so repeated turns of the same conversation reuse the same
/// session/thread identity (prompt-cache friendly).
pub(crate) fn codex_responses_headers(chat_id: &str) -> Vec<(String, String)> {
    let session_id = stable_conversation_uuid(chat_id, "session");
    let thread_id = stable_conversation_uuid(chat_id, "thread");
    vec![
        ("originator".to_string(), CODEX_ORIGINATOR.to_string()),
        ("user-agent".to_string(), codex_user_agent()),
        ("session-id".to_string(), session_id.clone()),
        ("thread-id".to_string(), thread_id.clone()),
        // Codex sets x-client-request-id to the thread id on the HTTP path.
        ("x-client-request-id".to_string(), thread_id),
        ("x-codex-window-id".to_string(), stable_conversation_uuid(chat_id, "window")),
    ]
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
        let headers = codex_responses_headers("42");
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
        assert!(user_agent.starts_with("codex_cli_rs/0.0.0 ("));
        assert!(user_agent.contains(std::env::consts::ARCH));

        // session-id and thread-id must be distinct, valid UUIDs.
        let value_of = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
                .unwrap()
        };
        let session_id = value_of("session-id");
        let thread_id = value_of("thread-id");
        assert_ne!(session_id, thread_id);
        assert!(Uuid::parse_str(&session_id).is_ok());
        assert!(Uuid::parse_str(&thread_id).is_ok());
        assert_eq!(value_of("x-client-request-id"), thread_id);
    }
}
