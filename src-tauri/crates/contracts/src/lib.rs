//! Transport-neutral control-plane wire contracts.
//!
//! These types are the single source of truth for JSON names and protocol
//! versions shared by the runtime, desktop client adapters, and `cscli`.
//! Application errors, Axum responses, and Tauri types deliberately stay out
//! of this crate.

pub mod bridge;
pub mod chat;
pub mod discovery;
pub mod error;
pub mod lease;
pub mod model_catalog;
pub mod protocol;
pub mod sse;
pub mod terminal;
pub mod web_mcp;
pub mod workflow;

pub use bridge::{
    validate_capability_arguments, ClientBridgeCancelRequest, ClientBridgeCapability,
    ClientBridgeDeclaration, ClientBridgeRegistration, ClientBridgeRegistrationResponse,
    ClientBridgeUnregisterRequest, ClientBridgeWorkEnvelope, ClientCapabilityError,
    ClientCapabilityInvocation, ClientCapabilityResult, ClientCapabilityStatus,
    BRIDGE_CAPABILITY_ALLOWLIST, BRIDGE_PROTOCOL_VERSION, BRIDGE_SCHEMA_VERSION,
    WEB_FETCH_ARGUMENTS, WEB_FETCH_TOOL, WEB_SEARCH_ARGUMENTS, WEB_SEARCH_TOOL,
};
pub use chat::{
    ChatProtocolDto, ChatResponseDto, ChatStartRequest, ChatStartResponse, ChatStopRequest,
    ChatStopResponse, ChatStreamEnvelope, ChatStreamEvent, FinishReasonDto, ListModelsRequest,
    MessageTypeDto, ModelDetailsDto, CHAT_STREAM_SCHEMA_VERSION,
};
pub use discovery::{ControlPlaneDiscovery, CONTROL_PLANE_HOST, DISCOVERY_FILE_NAME};
pub use error::{ErrorDetail, ErrorEnvelope};
pub use lease::{ClientLease, ClientLeaseRequest, ClientLeaseResponse};
pub use model_catalog::{
    ModelsDevPresetProviderDto, ModelsDevProviderModelsRequest, ResolveModelProfileRequest,
};
pub use protocol::{MetaResponse, PROTOCOL_MAJOR, PROTOCOL_VERSION, SCHEMA_VERSION};
pub use sse::{ResetError, ResetRequired, StreamEnvelope, STREAM_SCHEMA_VERSION};
pub use terminal::{
    TerminalCloseRequest, TerminalCreateRequest, TerminalExitEvent, TerminalListSessionsRequest,
    TerminalListShellsRequest, TerminalOutputEvent, TerminalResizeRequest,
    TerminalSessionMetadataDto, TerminalShellDto, TerminalStreamEnvelope, TerminalStreamEvent,
    TerminalWriteRequest, TERMINAL_SCHEMA_VERSION,
};
pub use web_mcp::{
    validate_web_mcp_port, web_mcp_endpoint, WebMcpProviderError, WebMcpProviderRegistration,
    WebMcpProviderRegistrationResponse, WebMcpProviderStatus, WEB_MCP_ALIASES,
    WEB_MCP_CLIENT_HEADER, WEB_MCP_CODE_CONFLICT, WEB_MCP_CODE_FORBIDDEN,
    WEB_MCP_CODE_INVALID_PORT, WEB_MCP_CODE_LEASE_INVALID, WEB_MCP_CODE_UNAVAILABLE,
    WEB_MCP_ENDPOINT_PATH, WEB_MCP_INSTANCE_HEADER, WEB_MCP_LEASE_HEADER, WEB_MCP_LOOPBACK_HOST,
    WEB_MCP_PROOF_HEADER, WEB_MCP_PROTOCOL_VERSION, WEB_MCP_REGISTER_PATH, WEB_MCP_SCHEMA_VERSION,
    WEB_MCP_SERVER_NAME, WEB_MCP_STATUS_PATH, WEB_MCP_UNREGISTER_PATH,
};
pub use workflow::{WorkflowCreateRequest, WorkflowEventsQuery, WorkflowStartRequest};

#[cfg(test)]
mod fixtures {
    use super::*;
    use serde_json::json;

    #[test]
    fn discovery_fixture_uses_stable_snake_case_wire_names() {
        let document = ControlPlaneDiscovery {
            protocol_version: PROTOCOL_VERSION.to_string(),
            server_instance_id: "instance-a".to_string(),
            pid: 42,
            host: CONTROL_PLANE_HOST.to_string(),
            port: 41234,
            token: "secret-is-wire-only".to_string(),
            started_at: "unix-1".to_string(),
        };
        assert_eq!(
            serde_json::to_value(document).expect("serialize discovery"),
            json!({
                "protocol_version": "1",
                "server_instance_id": "instance-a",
                "pid": 42,
                "host": "127.0.0.1",
                "port": 41234,
                "token": "secret-is-wire-only",
                "started_at": "unix-1"
            })
        );
    }

    #[test]
    fn error_fixture_preserves_the_stable_envelope() {
        let envelope = ErrorEnvelope::new("invalid_input", "bad request");
        assert_eq!(
            serde_json::to_value(envelope).expect("serialize error"),
            json!({"error": {"code": "invalid_input", "message": "bad request"}})
        );
    }

    #[test]
    fn lease_fixture_is_shared_by_all_clients() {
        let request = ClientLeaseRequest {
            client_id: "tauri-main".to_string(),
            client_kind: "tauri".to_string(),
        };
        assert_eq!(
            serde_json::to_value(request).expect("serialize lease request"),
            json!({"client_id": "tauri-main", "client_kind": "tauri"})
        );
    }

    #[test]
    fn stream_reset_fixture_is_machine_readable() {
        let reset = ResetRequired::new("cursor_expired");
        assert_eq!(
            serde_json::to_value(reset).expect("serialize reset"),
            json!({
                "schema_version": "1",
                "error": {"code": "reset_required", "reason": "cursor_expired"}
            })
        );
    }
}
