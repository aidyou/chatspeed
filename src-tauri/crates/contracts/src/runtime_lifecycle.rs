//! Runtime lifecycle wire contracts (U-4).
//!
//! The standalone `chatspeed-runtime` process owns a small, typed lifecycle
//! surface on the same canonical `/control/v1` control plane that already
//! serves the workflow, capability and automation routes:
//!
//! - `GET /control/v1/runtime/status` reports the authoritative lifecycle phase,
//!   the admission gate's epoch/active count, the live lease count and the
//!   automation keep-alive summary. It is read-only.
//! - `POST /control/v1/runtime/shutdown` requests a coordinated shutdown. It
//!   requires the bearer token (the shared middleware), a live lease proof, the
//!   expected instance id, an explicit `confirm`, an explicit `force` flag and a
//!   mandatory `Idempotency-Key`. It answers `202 Accepted` with the operation it
//!   started; a client must treat that as *accepted*, never as *stopped*, and
//!   re-read the status route until the phase reaches `stopped`.
//!
//! These types are the single source of truth for the JSON names shared by the
//! runtime, the desktop client adapters and `cscli`. Axum responses and Tauri
//! types deliberately stay out of this crate.

use serde::{Deserialize, Serialize};

/// Envelope schema version for the lifecycle DTOs.
pub const RUNTIME_LIFECYCLE_SCHEMA_VERSION: &str = "1";

/// Read-only lifecycle status route.
pub const RUNTIME_STATUS_PATH: &str = "/control/v1/runtime/status";

/// Coordinated shutdown request route.
pub const RUNTIME_SHUTDOWN_PATH: &str = "/control/v1/runtime/shutdown";

/// Header carrying the requesting client id for the shutdown lease proof.
pub const RUNTIME_CLIENT_HEADER: &str = "X-Runtime-Client";

/// Header carrying the requesting lease id for the shutdown lease proof.
pub const RUNTIME_LEASE_HEADER: &str = "X-Runtime-Lease";

/// Stable code: the request did not target the live instance.
pub const RUNTIME_CODE_INSTANCE_MISMATCH: &str = "instance_mismatch";

/// Stable code: the shutdown request was not explicitly confirmed.
pub const RUNTIME_CODE_NOT_CONFIRMED: &str = "not_confirmed";

/// Stable code: the caller has no live lease.
pub const RUNTIME_CODE_LEASE_INVALID: &str = "lease_invalid";

/// Stable code: a shutdown is already in progress.
pub const RUNTIME_CODE_ALREADY_STOPPING: &str = "already_stopping";

/// Stable code: the runtime is quiescing and refuses new work.
pub const RUNTIME_CODE_RUNTIME_STOPPING: &str = "runtime_stopping";

/// Stable code: the routing/management surface is unavailable in this process.
pub const RUNTIME_CODE_UNAVAILABLE: &str = "unavailable";

/// Phase of the runtime lifecycle state machine.
///
/// Forward-only: `running -> quiescing -> cancelling -> draining -> stopped`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLifecyclePhase {
    /// Accepting new work.
    Running,
    /// Refusing new work; the admission gate is closed.
    Quiescing,
    /// Cancelling in-flight work. U-4 provides the seam only; owned-resource
    /// cancellation lands with U-5.
    Cancelling,
    /// Waiting for admitted work to drain.
    Draining,
    /// The control plane has stopped.
    Stopped,
}

/// Stage of an accepted shutdown operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeOperationStage {
    /// Accepted by the control plane; not yet stopped.
    Accepted,
    /// Gate closed, refusals in effect.
    Quiescing,
    /// Cancellation phase (U-4 seam).
    Cancelling,
    /// Draining admitted work.
    Draining,
    /// Fully stopped.
    Stopped,
    /// Terminal failure before the shutdown could be completed.
    Failed,
}

/// Observation health of the automation keep-alive summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAutomationHealth {
    Healthy,
    Degraded,
    /// This process cannot observe live automation state; it must keep alive.
    Unknown,
}

/// Authoritative automation keep-alive summary mirrored onto the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeAutomationSummaryDto {
    pub effective_schedules: i64,
    pub live_automation_runs: i64,
    pub in_flight_admissions: i64,
    pub blocked_reconcile: i64,
    pub health: RuntimeAutomationHealth,
    /// Whether at least one reason above requires the runtime to stay alive.
    pub keep_alive: bool,
}

/// Terminal result of a shutdown operation, once one is known.
///
/// Distinguishes a clean coordinated exit from a forced one and from a failure,
/// so a client never mistakes a forced or failed outcome for a graceful stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeShutdownOutcome {
    /// The instance stopped.
    Graceful,
    /// The shutdown was driven by an explicit force request, so the extra forced
    /// budget (and, if needed, the last-resort termination fallback) applied.
    Forced,
    /// The shutdown could not complete within both budgets; the instance is not
    /// reported as stopped.
    Failed,
}

/// In-flight shutdown operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeOperationDto {
    pub operation_id: String,
    pub stage: RuntimeOperationStage,
    pub force: bool,
    pub accepted_at: String,
    /// The terminal outcome once one is known; absent while the operation is in
    /// flight and for a client that decodes an older payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<RuntimeShutdownOutcome>,
}

/// Typed runtime lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatusDto {
    pub protocol_version: String,
    pub schema_version: String,
    pub service: String,
    pub instance_id: String,
    pub pid: u32,
    pub phase: RuntimeLifecyclePhase,
    /// Admission-gate epoch; bumped once when the gate first closes.
    pub epoch: u64,
    /// Work admissions currently held across the runtime.
    pub active_admissions: usize,
    /// Live client leases.
    pub active_leases: usize,
    /// Whether the admission gate is closed (any phase after `running`).
    pub quiesced: bool,
    pub automation: RuntimeAutomationSummaryDto,
    /// The in-flight shutdown operation, when one exists.
    pub operation: Option<RuntimeOperationDto>,
}

/// `POST /control/v1/runtime/shutdown` request body.
///
/// `expected_instance_id` makes the operation safe against a stale discovery
/// document; `confirm` and `force` must both be explicit, and the lease proof
/// travels in the [`RUNTIME_CLIENT_HEADER`] / [`RUNTIME_LEASE_HEADER`] headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeShutdownRequest {
    /// The instance the caller believes it is shutting down.
    pub expected_instance_id: String,
    /// Explicit confirmation; the route refuses an unconfirmed request.
    #[serde(default)]
    pub confirm: bool,
    /// Explicit force flag. It is recorded on the operation; U-4 never claims
    /// that forced resource cleanup completed.
    #[serde(default)]
    pub force: bool,
}

/// `POST /control/v1/runtime/shutdown` `202 Accepted` body.
///
/// `accepted` is true when the operation was started or already in flight; the
/// `stage` is the current operation stage, never a completion claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeShutdownAcceptedDto {
    pub schema_version: String,
    pub service: String,
    pub instance_id: String,
    pub operation_id: String,
    pub stage: RuntimeOperationStage,
    pub accepted: bool,
    pub force: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_uses_stable_snake_case_wire_names() {
        let status = RuntimeStatusDto {
            protocol_version: "1".to_string(),
            schema_version: RUNTIME_LIFECYCLE_SCHEMA_VERSION.to_string(),
            service: "chatspeed-runtime".to_string(),
            instance_id: "instance-a".to_string(),
            pid: 42,
            phase: RuntimeLifecyclePhase::Draining,
            epoch: 3,
            active_admissions: 2,
            active_leases: 1,
            quiesced: true,
            automation: RuntimeAutomationSummaryDto {
                effective_schedules: 1,
                live_automation_runs: 0,
                in_flight_admissions: 0,
                blocked_reconcile: 0,
                health: RuntimeAutomationHealth::Healthy,
                keep_alive: true,
            },
            operation: Some(RuntimeOperationDto {
                operation_id: "op-1".to_string(),
                stage: RuntimeOperationStage::Draining,
                force: true,
                accepted_at: "unix-1".to_string(),
                outcome: None,
            }),
        };
        assert_eq!(
            serde_json::to_value(status).expect("serialize status"),
            json!({
                "protocol_version": "1",
                "schema_version": "1",
                "service": "chatspeed-runtime",
                "instance_id": "instance-a",
                "pid": 42,
                "phase": "draining",
                "epoch": 3,
                "active_admissions": 2,
                "active_leases": 1,
                "quiesced": true,
                "automation": {
                    "effective_schedules": 1,
                    "live_automation_runs": 0,
                    "in_flight_admissions": 0,
                    "blocked_reconcile": 0,
                    "health": "healthy",
                    "keep_alive": true
                },
                "operation": {
                    "operation_id": "op-1",
                    "stage": "draining",
                    "force": true,
                    "accepted_at": "unix-1"
                }
            })
        );
    }

    #[test]
    fn shutdown_request_defaults_confirm_and_force_to_false() {
        let request: RuntimeShutdownRequest =
            serde_json::from_value(json!({"expected_instance_id": "instance-a"}))
                .expect("deserialize shutdown request");
        assert_eq!(request.expected_instance_id, "instance-a");
        assert!(!request.confirm);
        assert!(!request.force);

        // Unknown fields are rejected so a typo cannot silently become a no-op.
        assert!(serde_json::from_value::<RuntimeShutdownRequest>(json!({
            "expected_instance_id": "instance-a",
            "confirmed": true
        }))
        .is_err());
    }
}
