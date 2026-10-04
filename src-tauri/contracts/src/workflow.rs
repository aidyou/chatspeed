//! Workflow application wire contracts for the runtime control plane.
//!
//! The desktop adapters exchange these request DTOs with the standalone runtime
//! over the canonical snake_case `/control/v1/workflows` routes, so this module
//! is the single source of truth for their JSON names. The runtime's application
//! service and its transport adapters consume the same shapes.
//!
//! The workflow domain error (`ApplicationError`/`ApplicationErrorKind`) stays
//! in the runtime backend: it is not a transport-neutral wire type. The request
//! DTOs here carry no [`skip_serializing_if`](serde) attributes, so absent
//! optionals serialize as JSON `null`, exactly as before the extraction.

use serde::{Deserialize, Serialize};

/// Transport-neutral workflow creation request (HTTP canonical snake_case).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowCreateRequest {
    pub user_query: Option<String>,
    pub agent_id: String,
    pub allowed_paths: Option<serde_json::Value>,
    pub auto_approve_plan: Option<bool>,
    pub final_audit: Option<bool>,
    pub inherited_agent_config: Option<String>,
}

/// Transport-neutral workflow start request (HTTP canonical snake_case).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowStartRequest {
    pub session_id: String,
    pub agent_id: String,
    pub initial_prompt: Option<String>,
    pub initial_metadata: Option<serde_json::Value>,
    pub initial_attached_context: Option<String>,
    pub planning_mode: Option<bool>,
}

/// Explicitly bounded durable-events query. `after` is a durable DB event ID
/// (never a live stream cursor); `limit` is capped by the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkflowEventsQuery {
    pub session_id: String,
    pub after: Option<i64>,
    pub limit: Option<u32>,
}

/// Maximum number of durable workflow events returned by one bounded query.
///
/// This is the transport-neutral cap shared by the runtime store (which clamps
/// the `limit` query parameter to it) and by the desktop adapter (which pages
/// the `/control/v1/workflows/{session_id}/events` route with it). It lives
/// here so the HTTP client never depends on a store-owned associated constant.
pub const WORKFLOW_EVENTS_MAX_LIMIT: u32 = 500;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn workflow_create_request_round_trips_snake_case_wire() {
        let request = WorkflowCreateRequest {
            user_query: Some("do the thing".to_string()),
            agent_id: "agent-1".to_string(),
            allowed_paths: Some(json!(["/tmp/a", "/tmp/b"])),
            auto_approve_plan: Some(true),
            final_audit: Some(false),
            inherited_agent_config: Some("{\"model\":\"gpt\"}".to_string()),
        };
        let value = serde_json::to_value(&request).expect("serialize create request");
        assert_eq!(
            value,
            json!({
                "user_query": "do the thing",
                "agent_id": "agent-1",
                "allowed_paths": ["/tmp/a", "/tmp/b"],
                "auto_approve_plan": true,
                "final_audit": false,
                "inherited_agent_config": "{\"model\":\"gpt\"}",
            })
        );
        let round_trip: WorkflowCreateRequest =
            serde_json::from_value(value.clone()).expect("deserialize create request");
        assert_eq!(
            serde_json::to_value(&round_trip).expect("re-serialize create request"),
            value
        );
    }

    #[test]
    fn workflow_create_request_defaults_omitted_or_null_optionals_to_null() {
        let expected = json!({
            "user_query": null,
            "agent_id": "agent-1",
            "allowed_paths": null,
            "auto_approve_plan": null,
            "final_audit": null,
            "inherited_agent_config": null,
        });
        // Omitted optionals and explicit `null` both normalize to `None` and
        // serialize back to `null`, so the wire stays byte-identical.
        for value in [
            json!({"agent_id": "agent-1"}),
            json!({
                "user_query": null,
                "agent_id": "agent-1",
                "allowed_paths": null,
                "auto_approve_plan": null,
                "final_audit": null,
                "inherited_agent_config": null,
            }),
        ] {
            let parsed: WorkflowCreateRequest =
                serde_json::from_value(value).expect("deserialize create request");
            assert_eq!(
                serde_json::to_value(&parsed).expect("serialize create request"),
                expected
            );
        }
    }

    #[test]
    fn workflow_start_request_round_trips_snake_case_wire() {
        let request = WorkflowStartRequest {
            session_id: "session-1".to_string(),
            agent_id: "agent-1".to_string(),
            initial_prompt: Some("go".to_string()),
            initial_metadata: Some(json!({"source": "ui"})),
            initial_attached_context: Some("ctx".to_string()),
            planning_mode: Some(true),
        };
        let value = serde_json::to_value(&request).expect("serialize start request");
        assert_eq!(
            value,
            json!({
                "session_id": "session-1",
                "agent_id": "agent-1",
                "initial_prompt": "go",
                "initial_metadata": {"source": "ui"},
                "initial_attached_context": "ctx",
                "planning_mode": true,
            })
        );
        let round_trip: WorkflowStartRequest =
            serde_json::from_value(value.clone()).expect("deserialize start request");
        assert_eq!(
            serde_json::to_value(&round_trip).expect("re-serialize start request"),
            value
        );
    }

    #[test]
    fn workflow_start_request_defaults_omitted_or_null_optionals_to_null() {
        let expected = json!({
            "session_id": "session-1",
            "agent_id": "agent-1",
            "initial_prompt": null,
            "initial_metadata": null,
            "initial_attached_context": null,
            "planning_mode": null,
        });
        for value in [
            json!({"session_id": "session-1", "agent_id": "agent-1"}),
            json!({
                "session_id": "session-1",
                "agent_id": "agent-1",
                "initial_prompt": null,
                "initial_metadata": null,
                "initial_attached_context": null,
                "planning_mode": null,
            }),
        ] {
            let parsed: WorkflowStartRequest =
                serde_json::from_value(value).expect("deserialize start request");
            assert_eq!(
                serde_json::to_value(&parsed).expect("serialize start request"),
                expected
            );
        }
    }

    #[test]
    fn workflow_events_max_limit_keeps_the_canonical_cap() {
        // The store clamp and the desktop adapter's page size must not diverge,
        // so the single shared cap stays pinned to its historical value.
        assert_eq!(WORKFLOW_EVENTS_MAX_LIMIT, 500);
    }

    #[test]
    fn workflow_events_query_round_trips_snake_case_wire() {
        let query = WorkflowEventsQuery {
            session_id: "session-1".to_string(),
            after: Some(42),
            limit: Some(100),
        };
        let value = serde_json::to_value(&query).expect("serialize events query");
        assert_eq!(
            value,
            json!({"session_id": "session-1", "after": 42, "limit": 100})
        );
        let round_trip: WorkflowEventsQuery =
            serde_json::from_value(value.clone()).expect("deserialize events query");
        assert_eq!(
            serde_json::to_value(&round_trip).expect("re-serialize events query"),
            value
        );
    }

    #[test]
    fn workflow_events_query_defaults_omitted_or_null_optionals_to_null() {
        let expected = json!({"session_id": "session-1", "after": null, "limit": null});
        for value in [
            json!({"session_id": "session-1"}),
            json!({"session_id": "session-1", "after": null, "limit": null}),
        ] {
            let parsed: WorkflowEventsQuery =
                serde_json::from_value(value).expect("deserialize events query");
            assert_eq!(
                serde_json::to_value(&parsed).expect("serialize events query"),
                expected
            );
        }
    }
}
