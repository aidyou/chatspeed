//! MCP lifecycle operations on the shared application service.
//!
//! This is the only place an MCP record or its runtime may change (AC-1). The
//! legacy Tauri commands and the `/control/v1` plane both delegate here, so a
//! desktop click and a CLI call run the same state machine and see the same
//! journal (AC-12).
//!
//! Two rules shape every operation:
//!
//! 1. **desired and observed never merge.** The persisted record says what the
//!    user wants; the runtime says what is true right now. A result carries both
//!    plus the moment the observation was taken (INV-7).
//! 2. **an unproven effect is never retried blindly.** If a start/stop either
//!    timed out or could not be confirmed by observation, the operation ends in
//!    `needs_reconcile` and the record is kept, so the doctor can classify it
//!    (AC-2/AC-10/INV-8).
//!
//! A mutation holds the same per-resource lock `begin` would take, so two
//! concurrent operations on one server serialize instead of interleaving their
//! runtime effects.

use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use crate::capability::error::{code, CapabilityError};
use crate::capability::mcp::descriptor::parse_descriptor;
use crate::capability::mcp::repository::NewMcpRecord;
use crate::capability::mcp::runtime::ObservedMcpRuntime;
use crate::capability::mcp_service::{
    project_mcp_server, redact_record_secrets, McpServerView,
};
use crate::capability::operation;
use crate::capability::types::{
    CapabilityKind, EffectOutcome, OperationBegin, OperationRequest, OperationState,
};
use crate::mcp::client::McpServerConfig;
use crate::capability::CapabilityApplicationService;
use crate::db::Mcp;

/// Effect key: the runtime was asked to start a server.
const EFFECT_START: &str = "mcp.start";
/// Effect key: the runtime was asked to stop a server.
const EFFECT_STOP: &str = "mcp.stop";
/// Effect key: the persisted record was removed.
const EFFECT_DELETE: &str = "mcp.delete";
/// Effect key: the runtime was asked to re-read a tool list.
const EFFECT_REFRESH_TOOLS: &str = "mcp.tools.refresh";
/// Effect key: a new record was registered.
const EFFECT_REGISTER: &str = "mcp.register";
/// Effect key: a record was rewritten.
const EFFECT_UPDATE: &str = "mcp.update";
/// Effect key: the persisted tool set changed.
const EFFECT_TOOL_PERSIST: &str = "mcp.tool.persist";
/// Effect key: the live client's tool set changed.
const EFFECT_TOOL_RUNTIME: &str = "mcp.tool.runtime";


/// How long the service waits for a runtime effect and its confirmation.
///
/// Conservative and injectable: a runtime that stops answering must produce
/// `needs_reconcile`, never an unbounded wait, and a test must not sleep for
/// seconds to exercise the timeout path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpTiming {
    /// Budget for one runtime effect call.
    pub effect_timeout: Duration,
    /// Budget for confirming a stop by observation.
    pub stop_confirm_timeout: Duration,
    /// Budget for waiting for a started server to become observable as running.
    ///
    /// Starting is asynchronous: `register_mcp_server` answers as soon as it has
    /// accepted the request, and a cold stdio process (e.g. an `npx` package that
    /// must boot and complete the MCP handshake) only reaches `running` seconds
    /// later. Confirming the start with a single observation right after the
    /// effect returns therefore races that transition and would misclassify a
    /// genuinely successful start as unobservable. This window bounds the poll
    /// for `running`, and it stays below the CLI's own request timeout so a slow
    /// but healthy start is proven inline instead of timing the caller out.
    pub start_confirm_timeout: Duration,
    /// Budget for a single status observation.
    pub status_timeout: Duration,
    /// Pause between confirmation polls.
    pub poll_interval: Duration,
}

impl Default for McpTiming {
    fn default() -> Self {
        Self {
            effect_timeout: Duration::from_secs(30),
            stop_confirm_timeout: Duration::from_secs(10),
            start_confirm_timeout: Duration::from_secs(15),
            status_timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(150),
        }
    }
}

/// The states that prove a server is not running.
const STOPPED_STATES: &[&str] = &["stopped", "error"];

/// A finished (or replayed) MCP mutation.
#[derive(Debug, Clone, Serialize)]
pub struct McpMutationResult {
    pub operation_id: String,
    /// True when the durable journal answered instead of a new attempt.
    pub replayed: bool,
    /// The redacted projection recorded in the journal.
    pub result: Value,
}

/// One tool-list read, with the freshness facts that make it honest.
#[derive(Debug, Clone, Serialize)]
pub struct McpToolsSnapshot {
    pub name: String,
    /// `runtime` when the live cache answered, `empty` when the server is known
    /// to expose nothing, `unavailable` when the runtime could not answer.
    pub source: String,
    /// `fresh`, `stale` or `unknown`, derived from the last successful refresh.
    pub freshness: String,
    pub tools: Vec<Value>,
    pub observed_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refreshed_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What opening an operation produced: a fresh attempt, or a stored replay.
enum OpenedOperation {
    /// The caller owns the effect and must run it.
    Fresh(String),
    /// The journal already answered; the stored result is returned unchanged.
    Replayed(McpMutationResult),
}

/// The lock key `begin_operation` serializes a capability mutation on.
fn journal_lock_key(resource_key: &str) -> String {
    format!("{}:{}", CapabilityKind::Mcp.as_str(), resource_key)
}

fn resource_key_for(name: &str) -> String {
    format!("mcp:{name}")
}

impl CapabilityApplicationService {
    // ------------------------------------------------------------- install

    /// Registers a server from a strict descriptor, always disabled.
    ///
    /// Installing performs no runtime effect at all: no process, no connection,
    /// no network (AC-9). Enabling is a separate, separately auditable operation.
    pub async fn mcp_install(
        &self,
        descriptor: &Value,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let parsed = parse_descriptor(descriptor)?;
        let name = parsed.config.name.clone();
        self.mcp_install_config(
            &name,
            &parsed.description,
            parsed.config,
            idempotency_key,
            actor_scope,
        )
        .await
    }

    /// Registers an already-validated configuration, always disabled.
    ///
    /// The desktop manual-add form produces a `McpServerConfig` directly, so it
    /// enters through this door; the strict descriptor front door above is for
    /// callers that send raw JSON (HTTP/CLI). Both end up in the same operation.
    pub async fn mcp_install_config(
        &self,
        name: &str,
        description: &str,
        config: McpServerConfig,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let name = name.to_string();
        let description = description.to_string();
        let key = operation::require_idempotency_key(idempotency_key)?;
        let resource_key = resource_key_for(&name);

        // Held for the whole mutation, so the effect below cannot interleave.
        let _guard = self.lock_resource(&resource_key).await;
        let operation_id = match self
            .open_operation(
                "mcp.install",
                &resource_key,
                // Only the redacted shape is journaled: a bearer token or env
                // value never enters the journal (AC-13).
                redacted_descriptor(
                    &serde_json::to_value(&config).unwrap_or(Value::Null),
                ),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };

        // A same-name record is not overwritten: the caller is told it already
        // exists, which keeps an accidental double install from clobbering a
        // working configuration (INV-6 by analogy with the Skill gate).
        let existing = self.mcp_repository().find_by_name(&name)?;
        if let Some(existing) = existing {
            let result = json!({
                "status": "already_registered",
                "name": name,
                "disabled": existing.disabled,
                "server": project_one(&existing, None),
            });
            self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
            return Ok(McpMutationResult {
                operation_id,
                replayed: false,
                result,
            });
        }

        self.set_state(&operation_id, OperationState::Applying, Some("register"))?;
        self.record_effect_intent(&operation_id, EFFECT_REGISTER, Some(&name), None)?;
        let registered = self.mcp_repository().add(NewMcpRecord {
            name: name.clone(),
            description: description.clone(),
            config: config.clone(),
            // Always disabled, whatever the caller's descriptor said.
            disabled: true,
        });

        match registered {
            Ok(record) => {
                let observation = json!({ "id": record.id, "disabled": record.disabled });
                self.record_effect_outcome(&operation_id, EFFECT_REGISTER, crate::capability::types::EffectOutcome::Applied, Some(&observation))?;
                let result = json!({
                    "status": "registered",
                    "id": record.id,
                    "name": record.name.clone(),
                    "disabled": record.disabled,
                    "server": project_one(&record, None),
                });
                self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
                Ok(McpMutationResult {
                    operation_id,
                    replayed: false,
                    result,
                })
            }
            Err(error) => {
                self.record_effect_outcome(&operation_id, EFFECT_REGISTER, crate::capability::types::EffectOutcome::Failed, None)?;
                self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
                Err(error)
            }
        }
    }

    // -------------------------------------------------------------- enable

    /// Sets the desired state to enabled and asks the runtime to start.
    pub async fn mcp_enable(
        &self,
        id: i64,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let record = self.require_server(id).await?;
        let key = operation::require_idempotency_key(idempotency_key)?;
        let name = record.name.clone();
        let resource_key = resource_key_for(&name);
        let _guard = self.lock_resource(&resource_key).await;

        let operation_id = match self
            .open_operation(
                "mcp.enable",
                &resource_key,
                json!({ "id": id, "name": name }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };
        // Phase 1: desired state. A durable write, so it is not retried blindly.
        self.set_state(&operation_id, OperationState::Applying, Some("desired"))?;
        let desired = self
            .mcp_repository()
            .set_disabled(id, false)?
            .ok_or_else(|| CapabilityError::new(code::NOT_FOUND, "the MCP record vanished"))?;

        // Phase 2: runtime effect. The observation decides the final state.
        self.set_state(&operation_id, OperationState::Applying, Some("start"))?;

        // A server the runtime already reports running needs no effect. Without
        // this short-circuit a double click (or a retried HTTP mutation with a
        // fresh key) would start a second process for one record.
        let already_running = match self.observe(&name).await {
            Ok(Some(observed)) => is_running(&observed.state),
            _ => false,
        };
        if already_running {
            let observed = self.observe(&name).await.ok().flatten();
            let result = json!({
                "status": "already_running",
                "id": desired.id,
                "name": name,
                "desired_enabled": true,
                "server": project_one(&desired, observed.as_ref()),
            });
            self.finish_operation(
                &operation_id,
                OperationState::Completed,
                Some(&result),
                None,
            )?;
            return Ok(McpMutationResult {
                operation_id,
                replayed: false,
                result,
            });
        }

        self.record_effect_intent(&operation_id, EFFECT_START, Some(&name), None)?;
        let timing = self.mcp_timing();
        let started = with_timeout(timing.effect_timeout, self.mcp_effects().start(desired.config.clone()))
            .await;

        match started {
            Err(_) => {
                // The call did not answer in time: the process may or may not
                // exist, so only reconciliation can settle it (INV-8).
                self.record_effect_outcome(
                    &operation_id,
                    EFFECT_START,
                    crate::capability::types::EffectOutcome::Unknown,
                    None,
                )?;
                self.require_reconcile(&operation_id, "start_timed_out")?;
                Err(CapabilityError::new(
                    code::NEEDS_RECONCILE,
                    "the MCP start did not answer in time; the runtime state is unknown",
                ))
            }
            Ok(Err(error)) => {
                self.record_effect_outcome(
                    &operation_id,
                    EFFECT_START,
                    crate::capability::types::EffectOutcome::Failed,
                    None,
                )?;
                // A known failure keeps the desired state enabled, which the
                // doctor reports as drift rather than pretending it is running.
                self.finish_operation(
                    &operation_id,
                    OperationState::Failed,
                    Some(&json!({
                        "status": "start_failed",
                        "name": name,
                        "desired_enabled": true,
                        "server": project_one(&desired, None),
                    })),
                    Some(&error),
                )?;
                Err(error)
            }
            Ok(Ok(())) => {
                let (running, observed) = self.wait_until_running(&name).await;
                let outcome = if running {
                    crate::capability::types::EffectOutcome::Applied
                } else {
                    crate::capability::types::EffectOutcome::Unknown
                };
                self.record_effect_outcome(&operation_id, EFFECT_START, outcome, Some(&json!({
                    "observed_state": observed.as_ref().map(|runtime| runtime.state.clone()),
                })))?;

                if !running {
                    self.require_reconcile(&operation_id, "start_not_observable")?;
                    return Err(CapabilityError::new(
                        code::NEEDS_RECONCILE,
                        "the MCP start reported success but the runtime does not show the server",
                    ));
                }

                let result = json!({
                    "status": "enabled",
                    "id": desired.id,
                    "name": name,
                    "desired_enabled": true,
                    "server": project_one(&desired, observed.as_ref()),
                });
                self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
                Ok(McpMutationResult {
                    operation_id,
                    replayed: false,
                    result,
                })
            }
        }
    }

    // ------------------------------------------------------------- disable

    /// Sets the desired state to disabled and confirms the runtime stopped.
    pub async fn mcp_disable(
        &self,
        id: i64,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let record = self.require_server(id).await?;
        let key = operation::require_idempotency_key(idempotency_key)?;
        let name = record.name.clone();
        let resource_key = resource_key_for(&name);
        let _guard = self.lock_resource(&resource_key).await;

        let operation_id = match self
            .open_operation(
                "mcp.disable",
                &resource_key,
                json!({ "id": id, "name": name }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };
        self.set_state(&operation_id, OperationState::Applying, Some("desired"))?;
        let desired = self
            .mcp_repository()
            .set_disabled(id, true)?
            .ok_or_else(|| CapabilityError::new(code::NOT_FOUND, "the MCP record vanished"))?;

        self.set_state(&operation_id, OperationState::Applying, Some("stop"))?;
        let confirmed = self.stop_and_confirm(&operation_id, &name).await;

        let observed = self.observe_for_view(&name).await;
        let result = json!({
            "status": if confirmed { "disabled" } else { "stop_unconfirmed" },
            "id": record.id,
            "name": name,
            "desired_enabled": false,
            "stop_confirmed": confirmed,
            "server": project_one(&desired, observed.as_ref()),
        });

        if confirmed {
            self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
            Ok(McpMutationResult {
                operation_id,
                replayed: false,
                result,
            })
        } else {
            // The record stays (now disabled) so a later reconcile can see it.
            self.require_reconcile(&operation_id, "stop_unconfirmed")?;
            Err(CapabilityError::new(
                code::NEEDS_RECONCILE,
                "the MCP server could not be confirmed stopped; the disabled record was kept",
            ))
        }
    }

    // ----------------------------------------------------------- uninstall

    /// Disables, stops, confirms and only then deletes the record.
    ///
    /// The order is the whole point: deleting persistence while a process may
    /// still be running would create an unowned child with no journal left to
    /// find it (AC-10).
    pub async fn mcp_uninstall(
        &self,
        id: i64,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let record = self.require_server(id).await?;
        let key = operation::require_idempotency_key(idempotency_key)?;
        let name = record.name.clone();
        let resource_key = resource_key_for(&name);
        let _guard = self.lock_resource(&resource_key).await;

        let operation_id = match self
            .open_operation(
                "mcp.uninstall",
                &resource_key,
                json!({ "id": id, "name": name }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };
        // Step 1: desired disabled, so a concurrent startup path will not race us.
        self.set_state(&operation_id, OperationState::Applying, Some("desired"))?;
        if self.mcp_repository().set_disabled(id, true)?.is_none() {
            // Nothing to remove: an earlier attempt already deleted the record,
            // which is the state the caller wanted, so it completes.
            let result = json!({ "status": "already_removed", "name": name });
            self.finish_operation(
                &operation_id,
                OperationState::Completed,
                Some(&result),
                None,
            )?;
            return Ok(McpMutationResult {
                operation_id,
                replayed: false,
                result,
            });
        }

        // Step 2: stop and prove it.
        self.set_state(&operation_id, OperationState::Applying, Some("stop"))?;
        let confirmed = self.stop_and_confirm(&operation_id, &name).await;
        if !confirmed {
            self.require_reconcile(&operation_id, "stop_unconfirmed_before_delete")?;
            return Err(CapabilityError::new(
                code::NEEDS_RECONCILE,
                "the MCP server stop is unconfirmed, so its record was kept",
            ));
        }

        // Step 3: delete persistence, then prove the record is gone.
        self.set_state(&operation_id, OperationState::Applying, Some("delete"))?;
        self.record_effect_intent(&operation_id, EFFECT_DELETE, Some(&name), None)?;
        if let Err(error) = self.mcp_repository().delete(id) {
            self.record_effect_outcome(
                &operation_id,
                EFFECT_DELETE,
                crate::capability::types::EffectOutcome::Failed,
                None,
            )?;
            self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
            return Err(error);
        }
        let still_present = self.mcp_repository().get(id)?.is_some();
        // Only a proven absence lets the delete be reported as applied.
        let runtime_answer = self.observe(&name).await;
        let runtime_proven_absent = match runtime_answer {
            Ok(None) => true,
            Ok(Some(observed)) => STOPPED_STATES.contains(&observed.state.as_str()),
            Err(_) => false,
        };
        let cache_still_has_it = !runtime_proven_absent;
        let outcome = if still_present || cache_still_has_it {
            crate::capability::types::EffectOutcome::Unknown
        } else {
            crate::capability::types::EffectOutcome::Applied
        };
        self.record_effect_outcome(
            &operation_id,
            EFFECT_DELETE,
            outcome,
            Some(&json!({
                "record_present": still_present,
                "runtime_present": cache_still_has_it,
            })),
        )?;

        if still_present || cache_still_has_it {
            self.require_reconcile(&operation_id, "delete_not_observable")?;
            return Err(CapabilityError::new(
                code::NEEDS_RECONCILE,
                "the MCP record or runtime entry is still present after deletion",
            ));
        }

        let result = json!({
            "status": "uninstalled",
            "name": name,
            "id": id,
            "record_present": false,
            "runtime_present": false,
        });
        self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
        Ok(McpMutationResult {
            operation_id,
            replayed: false,
            result,
        })
    }

    // ------------------------------------------------------ tools and status

    /// Re-reads a running server's tool list. Listing never invokes a tool.
    pub async fn mcp_refresh_tools(
        &self,
        id: i64,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let record = self.require_server(id).await?;
        let key = operation::require_idempotency_key(idempotency_key)?;
        let name = record.name.clone();
        let resource_key = resource_key_for(&name);
        let _guard = self.lock_resource(&resource_key).await;

        let operation_id = match self
            .open_operation(
                "mcp.tools.refresh",
                &resource_key,
                json!({ "id": id, "name": name }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };
        if record.disabled {
            let error = CapabilityError::new(
                code::REFUSED,
                "a disabled MCP server cannot be refreshed; enable it first",
            );
            self.finish_operation(&operation_id, OperationState::Blocked, None, Some(&error))?;
            return Err(error);
        }

        self.set_state(&operation_id, OperationState::Applying, Some("refresh"))?;
        self.record_effect_intent(&operation_id, EFFECT_REFRESH_TOOLS, Some(&name), None)?;
        let timing = self.mcp_timing();
        let refreshed = with_timeout(timing.effect_timeout, self.mcp_effects().refresh_tools(&name)).await;

        // Every non-completed refresh still journals the last-known snapshot, so
        // what was cached stays inspectable, but the *caller* gets the same
        // structured refusal as every other unproven MCP effect instead of a
        // success: a stale list reported as 200 hides an unconfirmed runtime
        // effect from anything that branches on status or exit code (AC-2/AC-11/
        // INV-8).
        let snapshot = self.tools_snapshot(&name).await;
        let tool_count = snapshot.tools.len();
        match refreshed {
            Err(_) => {
                let result = json!({
                    "status": "refresh_unconfirmed",
                    "name": name,
                    "kept_last_known": snapshot.tools,
                    "freshness": "stale",
                });
                self.record_effect_outcome(
                    &operation_id,
                    EFFECT_REFRESH_TOOLS,
                    EffectOutcome::Unknown,
                    Some(&json!({ "tool_count": tool_count })),
                )?;
                self.finish_operation(
                    &operation_id,
                    OperationState::NeedsReconcile,
                    Some(&result),
                    None,
                )?;
                // A stable reason is what lets the doctor and reconcile explain
                // why the operation is not converged yet.
                self.require_reconcile(&operation_id, "refresh_timed_out")?;
                Err(CapabilityError::new(
                    code::NEEDS_RECONCILE,
                    "the MCP tool refresh did not answer in time; the tool list is unknown",
                ))
            }
            Ok(Err(error)) => {
                let result = json!({
                    "status": "refresh_failed",
                    "name": name,
                    "kept_last_known": snapshot.tools,
                    "freshness": "failed",
                    "detail": error.redacted_message(),
                });
                self.record_effect_outcome(
                    &operation_id,
                    EFFECT_REFRESH_TOOLS,
                    EffectOutcome::Failed,
                    Some(&json!({ "tool_count": tool_count })),
                )?;
                self.finish_operation(
                    &operation_id,
                    OperationState::Failed,
                    Some(&result),
                    Some(&error),
                )?;
                Err(error)
            }
            Ok(Ok(())) => {
                let result = json!({
                    "status": "refreshed",
                    "name": name,
                    "tool_count": tool_count,
                    "freshness": "fresh",
                    "tools": snapshot.tools,
                });
                self.record_effect_outcome(
                    &operation_id,
                    EFFECT_REFRESH_TOOLS,
                    EffectOutcome::Applied,
                    Some(&json!({ "tool_count": tool_count })),
                )?;
                self.finish_operation(
                    &operation_id,
                    OperationState::Completed,
                    Some(&result),
                    None,
                )?;
                Ok(McpMutationResult {
                    operation_id,
                    replayed: false,
                    result,
                })
            }
        }
    }

    /// The cached tool list of one server, without invoking anything (AC-11).
    pub async fn mcp_tools(&self, id: i64) -> Result<McpToolsSnapshot, CapabilityError> {
        let record = self.require_server(id).await?;
        Ok(self.tools_snapshot(&record.name).await)
    }

    /// The tool declarations exactly as the runtime holds them.
    ///
    /// The legacy command surface needs the typed declarations rather than the
    /// journal-ready JSON snapshot, and both must come from one read path so the
    /// desktop page and the CLI cannot disagree about what is available.
    pub async fn mcp_tool_declarations(
        &self,
        id: i64,
    ) -> Result<Vec<crate::ai::traits::chat::MCPToolDeclaration>, CapabilityError> {
        let record = self.require_server(id).await?;
        let timing = self.mcp_timing();
        with_timeout(
            timing.status_timeout,
            self.mcp_effects().list_tools(&record.name),
        )
        .await
        .map_err(|_| {
            CapabilityError::new(
                code::RUNTIME_UNAVAILABLE,
                "the runtime did not answer the tool list in time",
            )
        })?
    }

    /// Every persisted record with secret values removed, keeping the legacy
    /// editable `Mcp` wire shape the desktop MCP page depends on. This is the
    /// only list the desktop adapter may return.
    pub async fn mcp_records_redacted(&self) -> Result<Vec<Mcp>, CapabilityError> {
        Ok(self
            .mcp_repository()
            .list()?
            .iter()
            .map(redact_record_secrets)
            .collect())
    }

    /// One persisted record by id with secret values removed, for the command
    /// returns that hand the edited `Mcp` back to the page.
    pub async fn mcp_record_redacted(&self, id: i64) -> Result<Option<Mcp>, CapabilityError> {
        Ok(self.mcp_repository().get(id)?.as_ref().map(redact_record_secrets))
    }

    /// One bounded runtime observation for one name, tri-stated for reconcile:
    /// `Ok(Some)` observed, `Ok(None)` proven absence (never running), `Err`
    /// unknown (a timeout, which must never be read as either running or
    /// stopped — INV-7/INV-8).
    pub async fn mcp_observe(
        &self,
        name: &str,
    ) -> Result<Option<ObservedMcpRuntime>, CapabilityError> {
        self.observe(name).await
    }

    /// One bounded status check for a single server.
    pub async fn mcp_status(&self, id: i64) -> Result<McpServerView, CapabilityError> {
        let record = self.require_server(id).await?;
        let observed = self.observe_for_view(&record.name).await;
        Ok(project_view(&record, observed.as_ref()))
    }

    /// Stops and starts one enabled server.
    pub async fn mcp_restart(
        &self,
        id: i64,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let record = self.require_server(id).await?;
        let key = operation::require_idempotency_key(idempotency_key)?;
        let name = record.name.clone();
        let resource_key = resource_key_for(&name);
        let _guard = self.lock_resource(&resource_key).await;

        let operation_id = match self
            .open_operation(
                "mcp.restart",
                &resource_key,
                json!({ "id": id, "name": name }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };

        if record.disabled {
            let error = CapabilityError::new(
                code::REFUSED,
                "a disabled MCP server cannot be restarted; enable it first",
            );
            self.finish_operation(&operation_id, OperationState::Blocked, None, Some(&error))?;
            return Err(error);
        }

        // Stop first: a restart that leaves two live clients for one record
        // would make the runtime state unreadable.
        self.set_state(&operation_id, OperationState::Applying, Some("stop"))?;
        if !self.stop_and_confirm(&operation_id, &name).await {
            self.require_reconcile(&operation_id, "restart_stop_unconfirmed")?;
            return Err(CapabilityError::new(
                code::NEEDS_RECONCILE,
                "the MCP server could not be confirmed stopped, so it was not restarted",
            ));
        }

        self.set_state(&operation_id, OperationState::Applying, Some("start"))?;
        self.record_effect_intent(&operation_id, EFFECT_START, Some(&name), None)?;
        let timing = self.mcp_timing();
        let started =
            with_timeout(timing.effect_timeout, self.mcp_effects().start(record.config.clone()))
                .await;
        match started {
            Err(_) => {
                self.record_effect_outcome(&operation_id, EFFECT_START, EffectOutcome::Unknown, None)?;
                self.require_reconcile(&operation_id, "restart_start_timed_out")?;
                Err(CapabilityError::new(
                    code::NEEDS_RECONCILE,
                    "the MCP restart did not answer in time",
                ))
            }
            Ok(Err(error)) => {
                self.record_effect_outcome(&operation_id, EFFECT_START, EffectOutcome::Failed, None)?;
                self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
                Err(error)
            }
            Ok(Ok(())) => {
                let (running, observed) = self.wait_until_running(&name).await;
                if !running {
                    self.record_effect_outcome(
                        &operation_id,
                        EFFECT_START,
                        EffectOutcome::Unknown,
                        None,
                    )?;
                    self.require_reconcile(&operation_id, "restart_start_not_observable")?;
                    return Err(CapabilityError::new(
                        code::NEEDS_RECONCILE,
                        "the MCP restart reported success but the runtime does not show the server",
                    ));
                }
                self.record_effect_outcome(
                    &operation_id,
                    EFFECT_START,
                    EffectOutcome::Applied,
                    Some(&json!({ "observed_state": observed.as_ref().map(|answer| answer.state.clone()) })),
                )?;
                let result = json!({
                    "status": "restarted",
                    "id": record.id,
                    "name": name,
                    "server": project_one(&record, observed.as_ref()),
                });
                self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
                Ok(McpMutationResult {
                    operation_id,
                    replayed: false,
                    result,
                })
            }
        }
    }

    /// Rewrites one record and re-applies it to the runtime.
    ///
    /// The persisted rewrite and the runtime swap are separate phases with their
    /// own effect rows, so a crash between them is visible instead of leaving an
    /// old process running a configuration the database no longer contains.
    pub async fn mcp_update(
        &self,
        id: i64,
        name: &str,
        description: &str,
        config: McpServerConfig,
        disabled: bool,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let previous = self.require_server(id).await?;
        // Presence / explicit-replace for secrets (AC-13). The redacted read model
        // never returns a token or env value, so a save that leaves those fields
        // blank arrives as `None` and must mean "keep what is stored", never
        // "delete the secret". A supplied value replaces; absence preserves.
        let mut config = config;
        if config.bearer_token.is_none() {
            config.bearer_token = previous.config.bearer_token.clone();
        }
        if config.env.is_none() {
            config.env = previous.config.env.clone();
        }
        let key = operation::require_idempotency_key(idempotency_key)?;
        let resource_names = vec![previous.name.as_str(), name];
        let _guards = self.lock_mcp_resources(&resource_names).await;
        let resource_key = resource_key_for(&previous.name);
        let operation_id = match self
            .open_operation(
                "mcp.update",
                &resource_key,
            // Only the redacted shape is journaled: a rewritten bearer token or
            // env value must never be stored (AC-13).
            json!({
                "id": id,
                "name": name,
                "disabled": disabled,
                "config": redacted_descriptor(
                    &serde_json::to_value(&config).unwrap_or(Value::Null)
                ),
            }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };

        self.set_state(&operation_id, OperationState::Applying, Some("persist"))?;
        self.record_effect_intent(&operation_id, EFFECT_UPDATE, Some(name), None)?;
        let updated = self
            .mcp_repository()
            .update(id, name, description, config.clone(), disabled);
        let updated = match updated {
            Ok(Some(updated)) => updated,
            Ok(None) => {
                let error = CapabilityError::new(code::NOT_FOUND, "the MCP record vanished");
                self.record_effect_outcome(&operation_id, EFFECT_UPDATE, EffectOutcome::Failed, None)?;
                self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
                return Err(error);
            }
            Err(error) => {
                self.record_effect_outcome(&operation_id, EFFECT_UPDATE, EffectOutcome::Failed, None)?;
                self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
                return Err(error);
            }
        };
        self.record_effect_outcome(
            &operation_id,
            EFFECT_UPDATE,
            EffectOutcome::Applied,
            Some(&json!({ "id": updated.id, "disabled": updated.disabled })),
        )?;

        // The old registration must go, even across a rename, or two runtime
        // entries would claim one record. A stop we cannot confirm is never
        // treated as success: mirroring restart, the new configuration is not
        // started and the operation is left durably reconcilable
        // (AC-10/AC-12/INV-8). The persisted rewrite above already moved the
        // desired state, so the journal carries `mcp.update` applied plus
        // `mcp.stop` unknown; reconcile converges it once the old runtime is
        // provably gone, and never reports the swap complete while it runs.
        self.set_state(&operation_id, OperationState::Applying, Some("swap"))?;
        let renamed = previous.name != updated.name;
        if !previous.disabled || renamed {
            if !self.stop_and_confirm(&operation_id, &previous.name).await {
                self.require_reconcile(&operation_id, "update_stop_unconfirmed")?;
                return Err(CapabilityError::new(
                    code::NEEDS_RECONCILE,
                    "the previous MCP runtime could not be confirmed stopped, so the new configuration was not started",
                ));
            }
        }
        // Starting the swapped-in runtime is an auditable, proven effect, not a
        // best-effort after-thought (AC-2/AC-10/AC-12/INV-7/INV-8). `mcp.start`
        // carries the intent and the observed outcome, mirroring enable: the
        // swap only completes once the new runtime is provably running, a
        // timed-out or unobservable start stays durably reconcilable so
        // reconcile roll-forwards the enable, and a deterministic failure is
        // reported as `Failed`. This never marks an update complete while the
        // runtime is not actually up.
        let observed = if disabled {
            None
        } else {
            self.set_state(&operation_id, OperationState::Applying, Some("start"))?;
            self.record_effect_intent(&operation_id, EFFECT_START, Some(&updated.name), None)?;
            let timing = self.mcp_timing();
            let started = with_timeout(
                timing.effect_timeout,
                self.mcp_effects().start(updated.config.clone()),
            )
            .await;
            match started {
                Err(_) => {
                    self.record_effect_outcome(
                        &operation_id,
                        EFFECT_START,
                        crate::capability::types::EffectOutcome::Unknown,
                        None,
                    )?;
                    self.require_reconcile(&operation_id, "update_start_timed_out")?;
                    return Err(CapabilityError::new(
                        code::NEEDS_RECONCILE,
                        "the updated MCP server start did not answer in time; the runtime state is unknown",
                    ));
                }
                Ok(Err(error)) => {
                    self.record_effect_outcome(
                        &operation_id,
                        EFFECT_START,
                        crate::capability::types::EffectOutcome::Failed,
                        None,
                    )?;
                    self.finish_operation(
                        &operation_id,
                        OperationState::Failed,
                        Some(&json!({
                            "status": "updated_but_start_failed",
                            "name": updated.name,
                            "server": project_one(&updated, None),
                        })),
                        Some(&error),
                    )?;
                    return Err(error);
                }
                Ok(Ok(())) => {
                    let (running, answer) = self.wait_until_running(&updated.name).await;
                    self.record_effect_outcome(
                        &operation_id,
                        EFFECT_START,
                        if running {
                            crate::capability::types::EffectOutcome::Applied
                        } else {
                            crate::capability::types::EffectOutcome::Unknown
                        },
                        Some(&json!({
                            "observed_state": answer.as_ref().map(|runtime| runtime.state.clone()),
                        })),
                    )?;
                    if !running {
                        self.require_reconcile(&operation_id, "update_start_not_observable")?;
                        return Err(CapabilityError::new(
                            code::NEEDS_RECONCILE,
                            "the updated MCP server start reported success but the runtime does not show it running",
                        ));
                    }
                    answer
                }
            }
        };

        let result = json!({
            "status": "updated",
            "id": updated.id,
            "name": updated.name,
            "disabled": updated.disabled,
            "server": project_one(&updated, observed.as_ref()),
        });
        self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
        Ok(McpMutationResult {
            operation_id,
            replayed: false,
            result,
        })
    }

    /// Enables or disables one tool, in persistence and in the runtime.
    pub async fn mcp_set_tool_disabled(
        &self,
        id: i64,
        tool: &str,
        disabled: bool,
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<McpMutationResult, CapabilityError> {
        let record = self.require_server(id).await?;
        let key = operation::require_idempotency_key(idempotency_key)?;
        let name = record.name.clone();
        let resource_key = resource_key_for(&name);
        let _guard = self.lock_resource(&resource_key).await;
        let operation_id = match self
            .open_operation(
                "mcp.tool.state",
                &resource_key,
                json!({ "id": id, "name": name, "tool": tool, "disabled": disabled }),
                key,
                actor_scope,
            )
            .await?
        {
            OpenedOperation::Fresh(operation_id) => operation_id,
            OpenedOperation::Replayed(result) => return Ok(result),
        };

        let mut config = record.config.clone();
        let mut disabled_tools = config.disabled_tools.take().unwrap_or_default();
        if disabled {
            disabled_tools.insert(tool.to_string());
        } else {
            disabled_tools.remove(tool);
        }
        config.disabled_tools = Some(disabled_tools.clone());

        self.set_state(&operation_id, OperationState::Applying, Some("persist"))?;
        self.record_effect_intent(&operation_id, EFFECT_TOOL_PERSIST, Some(&name), None)?;
        let updated = self.mcp_repository().update(
            id,
            &record.name,
            &record.description,
            config,
            record.disabled,
        )?;
        let Some(updated) = updated else {
            let error = CapabilityError::new(code::NOT_FOUND, "the MCP record vanished");
            self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
            return Err(error);
        };
        self.record_effect_outcome(&operation_id, EFFECT_TOOL_PERSIST, EffectOutcome::Applied, None)?;

        // A running client keeps its own copy of the set, so an enabled server
        // needs both the record and the live client to change (AC-12).
        self.set_state(&operation_id, OperationState::Applying, Some("runtime"))?;
        let runtime_effect = if record.disabled {
            Ok(())
        } else {
            self.mcp_effects().set_tool_disabled(&name, tool, disabled).await
        };
        if let Err(error) = runtime_effect {
            self.record_effect_outcome(&operation_id, EFFECT_TOOL_RUNTIME, EffectOutcome::Failed, None)?;
            self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
            return Err(error);
        }
        self.record_effect_outcome(&operation_id, EFFECT_TOOL_RUNTIME, EffectOutcome::Applied, None)?;

        let result = json!({
            "status": "tool_state_applied",
            "id": updated.id,
            "name": updated.name,
            "tool": tool,
            "disabled": disabled,
            "server": project_one(&updated, None),
        });
        self.finish_operation(&operation_id, OperationState::Completed, Some(&result), None)?;
        Ok(McpMutationResult {
            operation_id,
            replayed: false,
            result,
        })
    }

    // ------------------------------------------------------------- internals

    /// Opens a durable operation for one MCP resource.
    ///
    /// A repeat of the same `(actor_scope, idempotency_key)` returns the stored
    /// result instead of running the effect twice, so every operation in this
    /// module funnels through here (AC-2).
    async fn open_operation(
        &self,
        operation_kind: &str,
        resource_key: &str,
        request: Value,
        idempotency_key: String,
        actor_scope: &str,
    ) -> Result<OpenedOperation, CapabilityError> {
        let begin = self.repository().begin(&OperationRequest {
                capability: CapabilityKind::Mcp,
                operation_kind: operation_kind.to_string(),
                actor_scope: actor_scope.to_string(),
                idempotency_key,
                request,
                resource_key: resource_key.to_string(),
            })?;
        Ok(match begin {
            OperationBegin::Started(operation) => OpenedOperation::Fresh(operation.operation_id),
            OperationBegin::Replay(operation) => {
                OpenedOperation::Replayed(replayed_result(operation))
            }
        })
    }


    /// Takes the locks for all MCP names in a stable order.
    ///
    /// Rename operations touch both the old and new runtime names. Sorting before
    /// acquisition prevents two concurrent cross-renames from deadlocking while
    /// still sharing the same lock domain as reconcile.
    pub(crate) async fn lock_mcp_resources(
        &self,
        names: &[&str],
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut names: Vec<&str> = names.iter().copied().filter(|name| !name.is_empty()).collect();
        names.sort_unstable();
        names.dedup();

        let mut guards = Vec::with_capacity(names.len());
        for name in names {
            guards.push(self.lock_resource(&resource_key_for(name)).await);
        }
        guards
    }

    /// Takes the per-resource lock for one MCP name.
    async fn lock_resource(&self, resource_key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.locks().lock(&journal_lock_key(resource_key)).await
    }

    async fn require_server(&self, id: i64) -> Result<Mcp, CapabilityError> {
        self.mcp_repository().get(id)?.ok_or_else(|| {
            CapabilityError::new(code::NOT_FOUND, format!("no MCP record with id {id}"))
        })
    }

    /// One bounded runtime observation.
    ///
    /// The three answers are deliberately distinct: `Ok(Some(..))` is what the
    /// runtime reports, `Ok(None)` is a *proven* absence ("answered, and this
    /// server is not registered"), and `Err` means the answer is unknown, which
    /// proves nothing and must never be reported as either state (INV-7).
    async fn observe(&self, name: &str) -> Result<Option<ObservedMcpRuntime>, CapabilityError> {
        let timing = self.mcp_timing();
        with_timeout(timing.status_timeout, self.mcp_effects().observe(name))
            .await
            .map_err(|_| {
                CapabilityError::new(
                    code::EFFECT_STATE_UNKNOWN,
                    "the runtime did not answer the status probe in time",
                )
            })?
    }

    /// The observation a read model needs: a proven absence is reported as a
    /// stopped server, while an unknown answer stays unobserved.
    async fn observe_for_view(&self, name: &str) -> Option<ObservedMcpRuntime> {
        match self.observe(name).await {
            Ok(Some(observed)) => Some(observed),
            Ok(None) => Some(ObservedMcpRuntime {
                state: "stopped".to_string(),
                cached_tool_count: 0,
            }),
            Err(_) => None,
        }
    }

    /// Asks the runtime to stop a server and confirms it by observation.
    ///
    /// A server the runtime does not know about is already stopped, which is
    /// proven by the observation rather than assumed from the persisted record
    /// (INV-7). Skipping that proof would report a false `needs_reconcile`.
    async fn stop_and_confirm(&self, operation_id: &str, name: &str) -> bool {
        let timing = self.mcp_timing();
        self.record_effect_intent(operation_id, EFFECT_STOP, Some(name), None)
            .ok();

        // A runtime that answers "no such server" means the desired stopped
        // state is already true, so no effect is needed. An unknown answer is
        // not proof and the effect still has to be attempted.
        let skip_effect = matches!(self.observe(name).await, Ok(None));
        if !skip_effect {
            match with_timeout(timing.effect_timeout, self.mcp_effects().stop(name)).await {
                Err(_) => {
                    self.record_effect_outcome(
                        operation_id,
                        EFFECT_STOP,
                        crate::capability::types::EffectOutcome::Unknown,
                        None,
                    )
                    .ok();
                    return false;
                }
                Ok(Err(_)) => {
                    self.record_effect_outcome(
                        operation_id,
                        EFFECT_STOP,
                        crate::capability::types::EffectOutcome::Failed,
                        None,
                    )
                    .ok();
                    // A stop error is not proof: fall through to observation,
                    // which may still show the server gone.
                }
                Ok(Ok(())) => {}
            }
        }

        let deadline = tokio::time::Instant::now() + timing.stop_confirm_timeout;
        loop {
            match self.observe(name).await {
                // The runtime answered and has no entry for it: stopped.
                Ok(None) => {
                    self.record_effect_outcome(
                        operation_id,
                        EFFECT_STOP,
                        crate::capability::types::EffectOutcome::Applied,
                        Some(&json!({ "observed_state": null })),
                    )
                    .ok();
                    return true;
                }
                Ok(Some(observed)) if STOPPED_STATES.contains(&observed.state.as_str()) => {
                    self.record_effect_outcome(
                        operation_id,
                        EFFECT_STOP,
                        crate::capability::types::EffectOutcome::Applied,
                        Some(&json!({ "observed_state": observed.state })),
                    )
                    .ok();
                    return true;
                }
                // Still running, or the runtime refused to answer at all: neither
                // is proof, so keep polling and reconcile when the budget ends.
                Ok(Some(_)) | Err(_) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                self.record_effect_outcome(
                    operation_id,
                    EFFECT_STOP,
                    crate::capability::types::EffectOutcome::Unknown,
                    None,
                )
                .ok();
                return false;
            }
            tokio::time::sleep(timing.poll_interval).await;
        }
    }

    /// Starts a server and waits, bounded, for it to become observable as
    /// running.
    ///
    /// Returns `(true, Some(runtime))` only once the runtime reports `running` or
    /// `connected`; otherwise `(false, last)`, where `last` is the most recent
    /// non-running observation (`None` if the runtime never answered). A start
    /// that cannot be proven running within the window is not reported as a
    /// false success: the caller records `Unknown` and leaves the effect for
    /// reconciliation, which re-observes the real runtime (INV-7/INV-8). This is
    /// the symmetric counterpart of `stop_and_confirm`: `register_mcp_server`
    /// answers before a cold child has connected, so confirming a start has to
    /// poll instead of observing exactly once (which raced the transition and
    /// misclassified a healthy-but-slow start as unobservable). Reconcile uses
    /// the same confirmation for its update roll-forward, so a cold child is
    /// proven up there too instead of being judged by one observation.
    pub(crate) async fn wait_until_running(
        &self,
        name: &str,
    ) -> (bool, Option<ObservedMcpRuntime>) {
        let timing = self.mcp_timing();
        let deadline = tokio::time::Instant::now() + timing.start_confirm_timeout;
        let mut last: Option<ObservedMcpRuntime> = None;
        loop {
            match self.observe(name).await {
                Ok(Some(observed)) => {
                    let running = is_running(&observed.state);
                    last = Some(observed);
                    if running {
                        return (true, last);
                    }
                }
                // A proven absence is not running: keep polling, because a cold
                // child only appears in the runtime once it has connected.
                Ok(None) => {}
                // The runtime refused to answer: not proof either way.
                Err(_) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                return (false, last);
            }
            tokio::time::sleep(timing.poll_interval).await;
        }
    }

    /// Ends an operation as needing reconciliation, with a stable reason.
    fn require_reconcile(
        &self,
        operation_id: &str,
        reason: &str,
    ) -> Result<crate::capability::types::CapabilityOperation, CapabilityError> {
        self.repository()
            .mark_needs_reconcile(operation_id, reason, None)
    }

    /// Reads a tool snapshot and derives its freshness from the journal.
    async fn tools_snapshot(&self, name: &str) -> McpToolsSnapshot {
        let timing = self.mcp_timing();
        let observed_at_ms = operation::now_ms();
        let listed =
            with_timeout(timing.status_timeout, self.mcp_effects().list_tools(name)).await;

        let last_refreshed_at_ms = self
            .repository()
            .list_by_resource(CapabilityKind::Mcp, &resource_key_for(name), 12)
            .ok()
            .and_then(|operations| {
                operations.into_iter().find(|operation| {
                    operation.operation_kind == "mcp.tools.refresh"
                        && operation.state == OperationState::Completed
                })
            })
            .map(|operation| operation.updated_at_ms);

        match listed {
            Ok(Ok(tools)) => McpToolsSnapshot {
                name: name.to_string(),
                source: "runtime".to_string(),
                freshness: match last_refreshed_at_ms {
                    Some(_) => "fresh".to_string(),
                    // A cache that answered without any recorded refresh is only a
                    // best-known snapshot, never a proven-current list.
                    None => "unknown".to_string(),
                },
                tools: tools
                    .iter()
                    .map(|declaration| {
                        json!({
                            "name": declaration.name,
                            "description": declaration.description,
                            "disabled": declaration.disabled,
                            "input_schema": declaration.input_schema,
                        })
                    })
                    .collect(),
                observed_at_ms,
                last_refreshed_at_ms,
                detail: None,
            },
            Ok(Err(error)) => McpToolsSnapshot {
                name: name.to_string(),
                source: "unavailable".to_string(),
                freshness: "unknown".to_string(),
                tools: Vec::new(),
                observed_at_ms,
                last_refreshed_at_ms,
                detail: Some(error.redacted_message()),
            },
            Err(_) => McpToolsSnapshot {
                name: name.to_string(),
                source: "unavailable".to_string(),
                freshness: "stale".to_string(),
                tools: Vec::new(),
                observed_at_ms,
                last_refreshed_at_ms,
                detail: Some("the runtime did not answer the tool list in time".to_string()),
            },
        }
    }
}

/// Whether an observed state means the server is actually up.
fn is_running(state: &str) -> bool {
    matches!(state, "running" | "connected")
}

/// Runs a future with a timeout, reporting `Err(())` when it does not answer.
async fn with_timeout<T>(
    timeout: Duration,
    future: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    tokio::time::timeout(timeout, future).await.map_err(|_| ())
}

/// The stored, secret-free form of an install descriptor.
fn redacted_descriptor(descriptor: &Value) -> Value {
    crate::capability::redaction::bounded_redacted_json(descriptor, 4096)
}

/// Replays a stored terminal result.
fn replayed_result(operation: crate::capability::types::CapabilityOperation) -> McpMutationResult {
    McpMutationResult {
        operation_id: operation.operation_id,
        replayed: true,
        result: operation.result.unwrap_or(Value::Null),
    }
}

/// Projects one record plus one observation into the shared read DTO.
fn project_view(record: &Mcp, observed: Option<&ObservedMcpRuntime>) -> McpServerView {
    // `observed` is only `None` when the runtime could not answer at all; a
    // proven absence arrives as a synthesized stopped answer, so this stays
    // total without reaching through a one-element collection.
    match observed {
        Some(answer) => project_mcp_server(record, Some(answer), Some(operation::now_ms())),
        None => project_mcp_server(record, None, None),
    }
}

/// The same projection, ready to embed in a journal result.
fn project_one(record: &Mcp, observed: Option<&ObservedMcpRuntime>) -> Value {
    serde_json::to_value(project_view(record, observed)).unwrap_or(Value::Null)
}
