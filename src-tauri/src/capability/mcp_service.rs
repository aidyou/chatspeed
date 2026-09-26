//! MCP read projection.
//!
//! The projection keeps three facts strictly separate (INV-7):
//!
//! - **registered / desired** — what the durable record says the user wants;
//! - **runtime** — what the runtime observed just now, with a timestamp;
//! - **tools** — how many tools the runtime currently has cached, and whether
//!   that observation succeeded at all.
//!
//! No field in this DTO carries a secret value: a token is reported as a
//! boolean presence bit and the config itself only as a fingerprint of its
//! redacted form.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::capability::mcp::runtime::ObservedMcpRuntime;
use crate::capability::operation::{canonical_request_hash, now_ms};
use crate::capability::redaction;
use crate::db::Mcp;
use crate::mcp::client::{McpProtocolType, McpStatus};

/// Removes secret values from a stored record while keeping the legacy
/// editable wire shape.
///
/// `list_mcp_servers` must hand the desktop page a config it can put back into
/// the edit form (command/args/url/proxy/timeout/disabled_tools), so unlike
/// [`McpServerView`] this keeps the `Mcp` shape. It is still secret-free: the
/// bearer token and every environment value are dropped here, and presence is
/// reported by [`McpServerView::secret_present`] / [`McpServerView::env_present`]
/// from the capability projection the page already loads. An omitted secret on
/// a later update is then treated as "keep what is stored" (see
/// `CapabilityApplicationService::mcp_update`), never as an accidental delete.
pub fn redact_record_secrets(record: &Mcp) -> Mcp {
    let mut redacted = record.clone();
    redacted.config.bearer_token = None;
    redacted.config.env = None;
    redacted
}

/// Makes a live runtime `McpStatus` safe for the public / legacy desktop DTO.
///
/// A non-error status carries no secret. `McpStatus::Error`, however, embeds the
/// raw runtime message, and the MCP client formats connection/handshake failures
/// with values drawn from the config (a bearer token, an env value, or a URL
/// with userinfo). The desktop `list_mcp_servers` overlays that live status onto
/// every record, so without this the public IPC response could carry a secret in
/// the `status.error` field even though the config is redacted (AC-13).
///
/// The error message is therefore replaced with a stable non-sensitive code.
/// The `{"error": ...}` wire shape the page reads is preserved, and detailed
/// diagnostics remain in the log-redacted server logs and the capability
/// projection, which reports only the state name (INV-7/AC-13).
pub fn public_runtime_status(status: &McpStatus) -> McpStatus {
    match status {
        McpStatus::Error(_) => McpStatus::Error(redaction::REDACTED.to_string()),
        other => other.clone(),
    }
}

/// What the persisted record says, as opposed to what the runtime does.
#[derive(Debug, Clone, Serialize)]
pub struct McpDesiredView {
    /// `true` when the record is not disabled (the user wants it running).
    pub enabled: bool,
    pub registered: bool,
}

/// One observed runtime answer.
#[derive(Debug, Clone, Serialize)]
pub struct McpRuntimeView {
    /// The observed state name, or `unknown` when nothing was observed.
    pub state: String,
    /// Whether a runtime observation was actually available.
    pub observed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
}

/// Tool-list freshness for one server.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolsView {
    /// How many tools the runtime currently exposes, when observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// `observed` when the runtime answered in this read, `unavailable`
    /// otherwise. A refresh (a separate mutation) may later report
    /// `stale`/`failed` from a persisted revision.
    pub freshness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<i64>,
}

/// The full read model of one MCP server.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerView {
    pub id: i64,
    pub name: String,
    pub description: String,
    /// `stdio` or `streamable_http`; `sse` is reported but never installable.
    pub protocol_type: String,
    /// Whether this transport has a supported adapter.
    pub transport_supported: bool,
    pub desired: McpDesiredView,
    pub runtime: McpRuntimeView,
    pub tools: McpToolsView,
    /// A fingerprint of the redacted configuration, for drift comparison.
    pub config_fingerprint: String,
    /// Whether a bearer token is stored; never the token itself.
    pub secret_present: bool,
    /// Whether environment variables are stored; never their values.
    pub env_present: bool,
    /// A stable drift code when desired and observed disagree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<String>,
}

/// Drift code: the record wants the server running but nothing is observed.
pub const DRIFT_DESIRED_BUT_NOT_RUNNING: &str = "desired_enabled_not_running";
/// Drift code: the runtime is running while the record says disabled.
pub const DRIFT_RUNNING_WHILE_DISABLED: &str = "running_while_disabled";
/// Drift code: the record is enabled but the transport has no adapter.
pub const DRIFT_UNSUPPORTED_TRANSPORT: &str = "unsupported_transport";

/// Projects the persisted records and one runtime observation into read DTOs.
///
/// `observation` is `None` when the runtime could not be asked at all, which
/// is reported as `observed: false` rather than as a stopped server.
pub fn project_mcp_servers(
    servers: &[Mcp],
    observation: Option<&BTreeMap<String, ObservedMcpRuntime>>,
) -> Vec<McpServerView> {
    let observed_at_ms = observation.map(|_| now_ms());
    let empty = BTreeMap::new();
    let observed = observation.unwrap_or(&empty);
    // A runtime that answered but has no entry proves the server is not
    // registered, which is reported as stopped rather than as unobserved.
    let absent = ObservedMcpRuntime {
        state: "stopped".to_string(),
        cached_tool_count: 0,
    };

    let mut views: Vec<McpServerView> = servers
        .iter()
        .map(|server| {
            let runtime_answer = observation.map(|_| observed.get(&server.name).unwrap_or(&absent));
            project_mcp_server(server, runtime_answer, observed_at_ms)
        })
        .collect();

    views.sort_by(|left, right| left.name.cmp(&right.name));
    views
}

/// Projects one record with its runtime answer already resolved.
///
/// `runtime_answer` is `Some(..)` whenever the runtime answered, including the
/// synthesized entry that proves absence, and `None` only when nothing was
/// observed at all. Keeping this total means a single-server projection never
/// needs to reach back into a collection.
pub fn project_mcp_server(
    server: &Mcp,
    runtime_answer: Option<&ObservedMcpRuntime>,
    observed_at_ms: Option<i64>,
) -> McpServerView {
    let config_fingerprint = fingerprint(&server.config);
    let secret_present = server
        .config
        .bearer_token
        .as_deref()
        .map(|token| !token.is_empty())
        .unwrap_or(false);
    let env_present = server
        .config
        .env
        .as_ref()
        .map(|env| !env.is_empty())
        .unwrap_or(false);
    let transport_supported = matches!(
        server.config.protocol_type,
        McpProtocolType::Stdio | McpProtocolType::StreamableHttp
    );

    let runtime = McpRuntimeView {
        state: runtime_answer
            .map(|answer| answer.state.clone())
            .unwrap_or_else(|| "unknown".to_string()),
        observed: runtime_answer.is_some(),
        observed_at_ms: if runtime_answer.is_some() {
            observed_at_ms
        } else {
            None
        },
    };
    let tools = McpToolsView {
        count: runtime_answer.map(|answer| answer.cached_tool_count),
        freshness: if runtime_answer.is_some() {
            "observed".to_string()
        } else {
            "unavailable".to_string()
        },
        observed_at_ms: if runtime_answer.is_some() {
            observed_at_ms
        } else {
            None
        },
    };

    let desired = McpDesiredView {
        enabled: !server.disabled,
        registered: true,
    };
    let drift = classify_drift(&desired, &runtime, transport_supported);

    McpServerView {
        id: server.id,
        name: server.name.clone(),
        description: server.description.clone(),
        protocol_type: server.config.protocol_type.to_string(),
        transport_supported,
        desired,
        runtime,
        tools,
        config_fingerprint,
        secret_present,
        env_present,
        drift,
    }
}

fn classify_drift(
    desired: &McpDesiredView,
    runtime: &McpRuntimeView,
    transport_supported: bool,
) -> Option<String> {
    if desired.enabled && !transport_supported {
        return Some(DRIFT_UNSUPPORTED_TRANSPORT.to_string());
    }
    if !runtime.observed {
        // Nothing was observed: the projection must not claim drift it cannot
        // prove, and must not claim agreement either.
        return None;
    }
    let running = matches!(runtime.state.as_str(), "connected" | "running" | "starting");
    if desired.enabled && !running {
        return Some(DRIFT_DESIRED_BUT_NOT_RUNNING.to_string());
    }
    if !desired.enabled && running {
        return Some(DRIFT_RUNNING_WHILE_DISABLED.to_string());
    }
    None
}

/// A stable fingerprint of the redacted configuration.
///
/// Two records with the same effective configuration produce the same
/// fingerprint; two records that differ only in secret bytes do not, because
/// the secrets are redacted first. That is deliberate: drift detection is about
/// the configuration ChatSpeed can describe, and secrets never enter a
/// reported hash input verbatim.
fn fingerprint(config: &crate::mcp::client::McpServerConfig) -> String {
    match serde_json::to_value(config) {
        Ok(value) => canonical_request_hash(&redaction::redact_json(&value)),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::client::McpServerConfig;
    use std::collections::HashSet;

    fn server(id: i64, name: &str, disabled: bool, protocol: McpProtocolType) -> Mcp {
        Mcp {
            id,
            name: name.to_string(),
            description: "test".to_string(),
            config: McpServerConfig {
                name: name.to_string(),
                protocol_type: protocol,
                bearer_token: Some("canary-token".to_string()),
                env: Some(vec![("SECRET_ENV".to_string(), "canary-env".to_string())]),
                disabled_tools: Some(HashSet::new()),
                ..Default::default()
            },
            disabled,
            status: None,
        }
    }

    #[test]
    fn the_projection_never_carries_a_secret_value() {
        let servers = vec![server(1, "weather", false, McpProtocolType::Stdio)];
        let views = project_mcp_servers(&servers, None);
        let serialized = serde_json::to_string(&views).expect("serialize");

        assert!(!serialized.contains("canary"), "got {serialized}");
        assert!(views[0].secret_present);
        assert!(views[0].env_present);
        assert!(!views[0].config_fingerprint.is_empty());
    }

    #[test]
    fn desired_and_observed_are_reported_separately() {
        let servers = vec![server(1, "weather", false, McpProtocolType::Stdio)];
        let views = project_mcp_servers(&servers, None);
        assert!(views[0].desired.enabled);
        assert!(!views[0].runtime.observed);
        assert_eq!(views[0].runtime.state, "unknown");
        assert_eq!(views[0].tools.freshness, "unavailable");
        // No observation means no proven drift.
        assert!(views[0].drift.is_none());
    }

    #[test]
    fn drift_is_derived_from_desired_and_observed() {
        let servers = vec![
            server(1, "weather", false, McpProtocolType::Stdio),
            server(2, "notes", true, McpProtocolType::Stdio),
        ];
        let mut observation = BTreeMap::new();
        observation.insert(
            "weather".to_string(),
            ObservedMcpRuntime {
                state: "stopped".to_string(),
                cached_tool_count: 0,
            },
        );
        observation.insert(
            "notes".to_string(),
            ObservedMcpRuntime {
                state: "connected".to_string(),
                cached_tool_count: 3,
            },
        );

        let views = project_mcp_servers(&servers, Some(&observation));
        let weather = views.iter().find(|view| view.name == "weather").expect("weather");
        assert_eq!(weather.drift.as_deref(), Some(DRIFT_DESIRED_BUT_NOT_RUNNING));
        assert_eq!(weather.tools.count, Some(0));
        assert_eq!(weather.tools.freshness, "observed");

        let notes = views.iter().find(|view| view.name == "notes").expect("notes");
        assert_eq!(notes.drift.as_deref(), Some(DRIFT_RUNNING_WHILE_DISABLED));
    }

    /// An answered observation is a complete answer, so a server the runtime has
    /// no entry for is provably not running. Only "no answer at all" stays
    /// unobserved, and conflating the two is what would hide real drift.
    #[test]
    fn an_answered_observation_proves_a_missing_server_is_stopped() {
        let servers = vec![server(1, "weather", false, McpProtocolType::Stdio)];
        let views = project_mcp_servers(&servers, Some(&BTreeMap::new()));
        assert!(views[0].runtime.observed, "the runtime answered");
        assert_eq!(views[0].runtime.state, "stopped");
        assert_eq!(views[0].drift.as_deref(), Some(DRIFT_DESIRED_BUT_NOT_RUNNING));

        // The same record with no observation at all claims nothing either way.
        let unobserved = project_mcp_servers(&servers, None);
        assert!(!unobserved[0].runtime.observed);
        assert_eq!(unobserved[0].runtime.state, "unknown");
        assert!(unobserved[0].drift.is_none());
    }

    #[test]
    fn sse_is_reported_but_never_supported() {
        let servers = vec![server(1, "legacy", true, McpProtocolType::Sse)];
        let views = project_mcp_servers(&servers, None);
        assert_eq!(views[0].protocol_type, "sse");
        assert!(!views[0].transport_supported);
    }
}
