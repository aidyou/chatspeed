//! Deterministic state-machine tests for the MCP capability service (V-6).
//!
//! Every test drives the real `CapabilityApplicationService` through its ports,
//! so the assertions cover the journal, the desired/observed split, the bounded
//! waits and the reconcile paths — not just the happy path.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;

use crate::capability::error::{code, CapabilityError};
use crate::capability::mcp::orchestrator::McpTiming;
use crate::capability::mcp::repository::{McpRepositoryPort, NewMcpRecord};
use crate::capability::mcp::runtime::{McpRuntimeEffects, McpRuntimePort, ObservedMcpRuntime};
use crate::capability::types::{
    CapabilityKind, EffectOutcome, OperationRequest, OperationState,
};
use crate::capability::CapabilityApplicationService;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::MainStore;
use crate::db::Mcp;
use crate::mcp::client::{McpProtocolType, McpServerConfig, McpStatus};

/// A scripted persistence port.
#[derive(Default)]
struct FakeRepository {
    records: Mutex<Vec<Mcp>>,
    next_id: Mutex<i64>,
    /// When set, `add` fails with this code.
    fail_add: Mutex<Option<String>>,
    /// When set, `delete` fails with this code.
    fail_delete: Mutex<Option<String>>,
    add_calls: AtomicUsize,
    delete_calls: AtomicUsize,
}

impl FakeRepository {
    fn seed(&self, record: Mcp) -> i64 {
        let id = record.id;
        self.records.lock().expect("records").push(record);
        *self.next_id.lock().expect("next_id") = id + 1;
        id
    }

    fn snapshot(&self) -> Vec<Mcp> {
        self.records.lock().expect("records").clone()
    }
}

#[async_trait::async_trait]
impl McpRepositoryPort for FakeRepository {
    fn list(&self) -> Result<Vec<Mcp>, CapabilityError> {
        Ok(self.snapshot())
    }

    fn get(&self, id: i64) -> Result<Option<Mcp>, CapabilityError> {
        Ok(self
            .snapshot()
            .into_iter()
            .find(|record| record.id == id))
    }

    fn find_by_name(&self, name: &str) -> Result<Option<Mcp>, CapabilityError> {
        Ok(self
            .snapshot()
            .into_iter()
            .find(|record| record.name == name || record.config.name == name))
    }

    fn add(&self, record: NewMcpRecord) -> Result<Mcp, CapabilityError> {
        self.add_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(message) = self.fail_add.lock().expect("fail_add").take() {
            return Err(CapabilityError::new(code::STORE_ERROR, message));
        }
        let mut next = self.next_id.lock().expect("next_id");
        let id = *next;
        *next += 1;
        let stored = Mcp {
            id,
            name: record.name.clone(),
            description: record.description,
            config: McpServerConfig {
                name: record.name,
                ..record.config
            },
            disabled: record.disabled,
            status: None,
        };
        self.records
            .lock()
            .expect("records")
            .push(stored.clone());
        Ok(stored)
    }

    fn update(
        &self,
        id: i64,
        name: &str,
        description: &str,
        config: McpServerConfig,
        disabled: bool,
    ) -> Result<Option<Mcp>, CapabilityError> {
        let mut records = self.records.lock().expect("records");
        let Some(record) = records.iter_mut().find(|record| record.id == id) else {
            return Ok(None);
        };
        record.name = name.to_string();
        record.description = description.to_string();
        record.config = config;
        record.disabled = disabled;
        Ok(Some(record.clone()))
    }

    fn set_disabled(&self, id: i64, disabled: bool) -> Result<Option<Mcp>, CapabilityError> {
        let mut records = self.records.lock().expect("records");
        let Some(record) = records.iter_mut().find(|record| record.id == id) else {
            return Ok(None);
        };
        record.disabled = disabled;
        Ok(Some(record.clone()))
    }

    fn delete(&self, id: i64) -> Result<(), CapabilityError> {
        self.delete_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(message) = self.fail_delete.lock().expect("fail_delete").take() {
            return Err(CapabilityError::new(code::STORE_ERROR, message));
        }
        let mut records = self.records.lock().expect("records");
        records.retain(|record| record.id != id);
        Ok(())
    }
}

/// A scripted runtime.
#[derive(Default)]
struct FakeRuntime {
    /// The states the runtime reports, by server name.
    states: Mutex<HashMap<String, String>>,
    tools: Mutex<HashMap<String, Vec<String>>>,
    /// Effect log, e.g. `start:weather`, used to prove ordering and absence.
    calls: Mutex<Vec<String>>,
    start_result: Mutex<Option<String>>,
    stop_result: Mutex<Option<String>>,
    refresh_result: Mutex<Option<String>>,
    /// When true, effects never answer, so the bounded wait must fire.
    hang: Mutex<bool>,
    /// When true, only `start` never answers, so an already-confirmed stop can
    /// still be followed by a timed-out enable in the update path.
    start_hang: Mutex<bool>,
    /// When true, `start` answers but the server never becomes observable.
    silent_start: Mutex<bool>,
    /// When set, `start` leaves the server in this state (e.g. `starting`)
    /// instead of `running`, so the caller cannot observe it up on the first
    /// try. Models a cold child that is accepted before it has connected.
    start_state: Mutex<Option<String>>,
    /// When >0, a server started into `start_state` flips to `running` only
    /// after this many `observe` calls, so the bounded start confirmation must
    /// poll to prove the effect rather than observe once and give up.
    running_after_observations: Mutex<usize>,
    /// Total `observe` effect calls, used to prove the start polled (INV-7).
    observe_count: AtomicUsize,
    /// When true, `stop` answers but the server stays observable.
    stubborn_stop: Mutex<bool>,
    /// Guards against a released-then-observed race in the concurrency test.
    concurrent: AtomicUsize,
    max_concurrent: AtomicUsize,
}

impl FakeRuntime {
    fn set_state(&self, name: &str, state: &str) {
        self.states
            .lock()
            .expect("states")
            .insert(name.to_string(), state.to_string());
    }

    fn state_of(&self, name: &str) -> Option<String> {
        self.states.lock().expect("states").get(name).cloned()
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }

    fn record_call(&self, call: &str) {
        self.calls
            .lock()
            .expect("calls")
            .push(call.to_string());
    }

    /// Models one effect, with the concurrency instrumentation.
    ///
    /// When `hang` is set the effect never answers, which is how the bounded
    /// wait and the `needs_reconcile` path are exercised without a long sleep.
    async fn effect(&self, hang: bool) {
        let live = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_concurrent.fetch_max(live, Ordering::SeqCst);
        if hang {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        } else {
            // Yield so a second caller can prove it waited for the lock.
            tokio::task::yield_now().await;
        }
        self.concurrent.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl McpRuntimePort for FakeRuntime {
    async fn observed_runtime(&self) -> Result<BTreeMap<String, ObservedMcpRuntime>, CapabilityError> {
        let mut observed = BTreeMap::new();
        for (name, state) in self.states.lock().expect("states").iter() {
            observed.insert(
                name.clone(),
                ObservedMcpRuntime {
                    state: state.clone(),
                    cached_tool_count: self
                        .tools
                        .lock()
                        .expect("tools")
                        .get(name)
                        .map(|tools| tools.len())
                        .unwrap_or(0),
                },
            );
        }
        Ok(observed)
    }
}

#[async_trait::async_trait]
impl McpRuntimeEffects for FakeRuntime {
    async fn start(&self, config: McpServerConfig) -> Result<(), CapabilityError> {
        self.record_call(&format!("start:{}", config.name));
        let hang = *self.hang.lock().expect("hang")
            || *self.start_hang.lock().expect("start_hang");
        self.effect(hang).await;
        if hang {
            return Ok(());
        }
        match self.start_result.lock().expect("start_result").as_deref() {
            Some(message) => Err(CapabilityError::new(code::INTERNAL, message.to_string())),
            None => {
                if let Some(state) = self.start_state.lock().expect("start_state").clone() {
                    // Accepted but not yet up: a cold child connects later, so the
                    // first observations must not already read as running.
                    self.set_state(&config.name, &state);
                } else if !*self.silent_start.lock().expect("silent_start") {
                    self.set_state(&config.name, "running");
                }
                Ok(())
            }
        }
    }

    async fn stop(&self, name: &str) -> Result<(), CapabilityError> {
        self.record_call(&format!("stop:{name}"));
        let hang = *self.hang.lock().expect("hang");
        self.effect(hang).await;
        if hang {
            return Ok(());
        }
        match self.stop_result.lock().expect("stop_result").as_deref() {
            Some(message) => Err(CapabilityError::new(code::INTERNAL, message.to_string())),
            None => {
                if !*self.stubborn_stop.lock().expect("stubborn_stop") {
                    self.states.lock().expect("states").remove(name);
                }
                Ok(())
            }
        }
    }

    async fn refresh_tools(&self, name: &str) -> Result<(), CapabilityError> {
        self.record_call(&format!("refresh:{name}"));
        let hang = *self.hang.lock().expect("hang");
        self.effect(hang).await;
        if hang {
            return Ok(());
        }
        match self.refresh_result.lock().expect("refresh_result").as_deref() {
            Some(message) => Err(CapabilityError::new(code::INTERNAL, message.to_string())),
            None => {
                self.tools.lock().expect("tools").insert(
                    name.to_string(),
                    vec![format!("{name}_tool"), "other".to_string()],
                );
                Ok(())
            }
        }
    }

    async fn list_tools(&self, name: &str) -> Result<Vec<MCPToolDeclaration>, CapabilityError> {
        self.record_call(&format!("list_tools:{name}"));
        let Some(tools) = self
            .tools
            .lock()
            .expect("tools")
            .get(name)
            .cloned()
        else {
            return Err(CapabilityError::new(
                code::NOT_FOUND,
                format!("no cached tools for '{name}'"),
            ));
        };
        Ok(tools
            .into_iter()
            .map(|tool| MCPToolDeclaration {
                name: tool,
                description: "a tool".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                disabled: false,
                scope: None,
            })
            .collect())
    }

    async fn set_tool_disabled(
        &self,
        server: &str,
        tool: &str,
        disabled: bool,
    ) -> Result<(), CapabilityError> {
        self.record_call(&format!("tool_state:{server}:{tool}:{disabled}"));
        Ok(())
    }

    async fn observe(&self, name: &str) -> Result<Option<ObservedMcpRuntime>, CapabilityError> {
        self.observe_count.fetch_add(1, Ordering::SeqCst);
        {
            let mut remaining = self
                .running_after_observations
                .lock()
                .expect("running_after");
            if *remaining > 0 {
                *remaining -= 1;
                if *remaining == 0 && self.state_of(name).as_deref() != Some("running") {
                    self.set_state(name, "running");
                }
            }
        }
        Ok(self.state_of(name).map(|state| ObservedMcpRuntime {
            state,
            cached_tool_count: self
                .tools
                .lock()
                .expect("tools")
                .get(name)
                .map(|tools| tools.len())
                .unwrap_or(0),
        }))
    }
}

struct Fixture {
    _temp: TempDir,
    service: Arc<CapabilityApplicationService>,
    repository: Arc<FakeRepository>,
    runtime: Arc<FakeRuntime>,
}

fn fixture() -> Fixture {
    let temp = TempDir::new().expect("temp dir");
    let store = Arc::new(MainStore::new(":memory:").expect("in-memory store"));
    let repository = Arc::new(FakeRepository::default());
    let runtime = Arc::new(FakeRuntime::default());
    let service = Arc::new(
        CapabilityApplicationService::new(store, temp.path().to_path_buf())
            .with_mcp_repository(repository.clone())
            .with_mcp_runtime(runtime.clone(), runtime.clone())
            .with_mcp_timing(McpTiming {
                effect_timeout: Duration::from_millis(60),
                stop_confirm_timeout: Duration::from_millis(60),
                start_confirm_timeout: Duration::from_millis(120),
                status_timeout: Duration::from_millis(60),
                poll_interval: Duration::from_millis(2),
            }),
    );
    Fixture {
        _temp: temp,
        service,
        repository,
        runtime,
    }
}

fn descriptor(name: &str) -> Value {
    json!({
        "name": name,
        "description": "fixture",
        "type": "stdio",
        "command": "node",
        "args": ["server.js"]
    })
}

fn record(name: &str, id: i64, disabled: bool) -> Mcp {
    Mcp {
        id,
        name: name.to_string(),
        description: "fixture".to_string(),
        config: McpServerConfig {
            name: name.to_string(),
            protocol_type: McpProtocolType::Stdio,
            url: None,
            bearer_token: None,
            proxy: None,
            command: Some("node".to_string()),
            args: Some(vec!["server.js".to_string()]),
            env: Some(Vec::new()),
            disabled_tools: Some(HashSet::new()),
            timeout: None,
        },
        disabled,
        // A stopped record still has a status field, so `stop_and_confirm`
        // exercises the real effect rather than skipping it.
        status: Some(McpStatus::Stopped),
    }
}

// ------------------------------------------------------------------ install

#[tokio::test]
async fn the_desktop_read_path_serializes_no_secret_canary() {
    // `list_mcp_servers` returns exactly `mcp_records_redacted()`, so this drives
    // the same projection the Tauri IPC serializes for the desktop page (AC-13).
    // It is a real serialized-response check, not just the pure helper.
    let fixture = fixture();
    let mut secret = record("weather", 1, false);
    secret.config.bearer_token = Some("CANARY-BEARER".to_string());
    secret.config.env = Some(vec![(
        "API_TOKEN".to_string(),
        "CANARY-ENV".to_string(),
    )]);
    fixture.repository.seed(secret);

    let records = fixture
        .service
        .mcp_records_redacted()
        .await
        .expect("redacted records");
    let serialized = serde_json::to_string(&records).expect("serialize");
    assert!(
        !serialized.contains("CANARY-BEARER"),
        "bearer token leaked into the desktop read: {serialized}"
    );
    assert!(
        !serialized.contains("CANARY-ENV"),
        "env value leaked into the desktop read: {serialized}"
    );
    // The editable non-sensitive wire and the record identity survive (INV-2).
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, "weather");
    assert_eq!(records[0].config.command.as_deref(), Some("node"));
    assert!(records[0].config.bearer_token.is_none());
}

#[tokio::test]
async fn install_registers_disabled_and_performs_no_runtime_effect() {
    let fixture = fixture();
    let result = fixture
        .service
        .mcp_install(&descriptor("weather"), "key-1", "test")
        .await
        .expect("install");

    assert_eq!(result.result["status"], "registered");
    assert_eq!(result.result["disabled"], json!(true));
    let records = fixture.repository.snapshot();
    assert_eq!(records.len(), 1);
    assert!(records[0].disabled, "a new record is never enabled (AC-9)");
    // The decisive proof: installing touched no runtime at all.
    assert!(
        fixture.runtime.calls().is_empty(),
        "install must not start or probe anything, got {:?}",
        fixture.runtime.calls()
    );
    assert!(fixture.runtime.state_of("weather").is_none());

    // The operation is durable and completed.
    let operation = fixture
        .service
        .operation(&result.operation_id)
        .expect("operation");
    assert_eq!(operation.state, OperationState::Completed);
    assert_eq!(operation.operation_kind, "mcp.install");
    assert_eq!(operation.capability, CapabilityKind::Mcp);
}

#[tokio::test]
async fn install_refuses_a_duplicate_name_without_a_second_record_or_process() {
    let fixture = fixture();
    fixture
        .service
        .mcp_install(&descriptor("weather"), "key-1", "test")
        .await
        .expect("first install");
    let second = fixture
        .service
        .mcp_install(&descriptor("weather"), "key-2", "test")
        .await
        .expect("duplicate install");

    assert_eq!(second.result["status"], "already_registered");
    assert_eq!(fixture.repository.snapshot().len(), 1);
    assert_eq!(
        fixture.repository.add_calls.load(Ordering::SeqCst),
        1,
        "a duplicate must not add a second record"
    );
    assert!(fixture.runtime.calls().is_empty());
}

#[tokio::test]
async fn install_replays_one_key_and_conflicts_on_a_changed_request() {
    let fixture = fixture();
    let first = fixture
        .service
        .mcp_install(&descriptor("weather"), "key-1", "test")
        .await
        .expect("install");
    let replay = fixture
        .service
        .mcp_install(&descriptor("weather"), "key-1", "test")
        .await
        .expect("replay");

    assert!(replay.replayed);
    assert_eq!(replay.operation_id, first.operation_id);
    assert_eq!(replay.result, first.result);
    assert_eq!(fixture.repository.snapshot().len(), 1);

    let conflict = fixture
        .service
        .mcp_install(&descriptor("other"), "key-1", "test")
        .await
        .err()
        .expect("conflict");
    assert_eq!(conflict.code(), code::IDEMPOTENCY_KEY_CONFLICT);

    // A missing key is refused before any journal row exists.
    let missing = fixture
        .service
        .mcp_install(&descriptor("third"), "   ", "test")
        .await
        .err()
        .expect("missing key");
    assert_eq!(missing.code(), code::IDEMPOTENCY_KEY_REQUIRED);
    assert_eq!(fixture.repository.snapshot().len(), 1);
}

#[tokio::test]
async fn the_journal_never_stores_a_descriptor_secret() {
    let fixture = fixture();
    let result = fixture
        .service
        .mcp_install(
            &json!({
                "name": "remote",
                "type": "streamable_http",
                "url": "https://example.test/mcp",
                "bearer_token": "canary-descriptor-token",
                "timeout": 30
            }),
            "key-1",
            "test",
        )
        .await
        .expect("install");

    let operation = fixture
        .service
        .operation(&result.operation_id)
        .expect("operation");
    let stored = format!(
        "{}{}{:?}",
        serde_json::to_string(&operation.request).expect("request json"),
        serde_json::to_string(&operation.result).expect("result json"),
        operation.error_message
    );
    assert!(
        !stored.contains("canary-descriptor-token"),
        "the journal leaked the bearer token: {stored}"
    );
    // The presence bit is still reported, which is what the UI needs.
    assert_eq!(operation.result.expect("result")["server"]["secret_present"], json!(true));
}

// ------------------------------------------------------------------- update

/// A record that already stores a secret, for the presence / explicit-replace
/// contract that keeps a blank re-save from deleting a credential (AC-13).
fn secret_record(name: &str, id: i64) -> Mcp {
    Mcp {
        id,
        name: name.to_string(),
        description: "fixture".to_string(),
        config: McpServerConfig {
            name: name.to_string(),
            protocol_type: McpProtocolType::Stdio,
            url: None,
            bearer_token: Some("stored-token".to_string()),
            proxy: None,
            command: Some("node".to_string()),
            args: Some(vec!["server.js".to_string()]),
            env: Some(vec![("API_TOKEN".to_string(), "stored-env".to_string())]),
            disabled_tools: Some(HashSet::new()),
            timeout: None,
        },
        disabled: true,
        status: Some(McpStatus::Stopped),
    }
}

fn blank_secret_config(name: &str) -> McpServerConfig {
    McpServerConfig {
        name: name.to_string(),
        protocol_type: McpProtocolType::Stdio,
        url: None,
        bearer_token: None,
        proxy: None,
        command: Some("node".to_string()),
        args: Some(vec!["server.js".to_string()]),
        env: None,
        disabled_tools: Some(HashSet::new()),
        timeout: None,
    }
}

#[tokio::test]
async fn an_update_that_omits_secrets_keeps_the_stored_token_and_env() {
    let fixture = fixture();
    let id = fixture.repository.seed(secret_record("weather", 1));

    fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            true,
            "key-1",
            "test",
        )
        .await
        .expect("update");

    let stored = &fixture.repository.snapshot()[0];
    assert_eq!(stored.description, "updated");
    assert_eq!(
        stored.config.bearer_token.as_deref(),
        Some("stored-token"),
        "a blank token must never delete the stored secret"
    );
    assert_eq!(
        stored.config.env.as_deref(),
        Some([("API_TOKEN".to_string(), "stored-env".to_string())].as_slice()),
        "a blank env must never delete the stored secret"
    );
}

#[tokio::test]
async fn an_update_that_supplies_secrets_replaces_them() {
    let fixture = fixture();
    let id = fixture.repository.seed(secret_record("weather", 1));

    let mut replacement = blank_secret_config("weather");
    replacement.bearer_token = Some("new-token".to_string());
    replacement.env = Some(vec![("OTHER".to_string(), "new-env".to_string())]);
    fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            replacement,
            true,
            "key-1",
            "test",
        )
        .await
        .expect("update");

    let stored = &fixture.repository.snapshot()[0];
    assert_eq!(stored.config.bearer_token.as_deref(), Some("new-token"));
    assert_eq!(
        stored.config.env.as_deref(),
        Some([("OTHER".to_string(), "new-env".to_string())].as_slice())
    );
}

/// A previously-running server whose old process cannot be confirmed stopped
/// must not have its new configuration started, and the operation must not be
/// reported complete: otherwise the old runtime and a fresh one would both
/// claim a single record (AC-10/AC-12/INV-8). This mirrors the restart gate.
#[tokio::test]
async fn an_update_does_not_start_the_new_config_when_the_old_stop_is_unconfirmed() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    // The stop call answers, yet the server stays observable as running.
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = true;

    let error = fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .err()
        .expect("an unconfirmed stop must not swap the runtime");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);

    // The safety property: a stop was attempted but the new config never started.
    let calls = fixture.runtime.calls();
    assert!(
        calls.contains(&"stop:weather".to_string()),
        "the old runtime must be asked to stop: {calls:?}"
    );
    assert!(
        !calls.iter().any(|call| call.starts_with("start:")),
        "the new config must not start while the old stop is unproven: {calls:?}"
    );

    let op = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0]
        .clone();
    assert_eq!(op.state, OperationState::NeedsReconcile);
    assert_eq!(op.reconcile_reason.as_deref(), Some("update_stop_unconfirmed"));
    // The unproven stop is preserved so recovery can converge it, not lose it.
    assert_eq!(
        fixture
            .service
            .repository()
            .count_unproven_effects(&op.operation_id)
            .expect("count"),
        1
    );
}

/// The interrupted update stop phase must fully roll forward: once the old
/// runtime is proven gone, reconcile starts the enabled new configuration,
/// observes it running, and only then completes the operation (AC-10/AC-12/
/// INV-7/INV-8). It never reports completion while the desired runtime is down.
#[tokio::test]
async fn reconcile_converges_an_update_once_the_old_runtime_is_proven_stopped() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = true;
    fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .expect_err("stop unconfirmed");
    let op = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0]
        .clone();
    // The live swap never started the new config.
    assert!(!fixture
        .runtime
        .calls()
        .iter()
        .any(|call| call.starts_with("start:")));

    // Recovery: the old process is finally gone.
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = false;
    fixture.runtime.set_state("weather", "stopped");

    let report = fixture.service.reconcile().await.expect("reconcile");
    assert_eq!(
        report.mcp_effects_recovered,
        vec![
            "mcp:weather:mcp.stop".to_string(),
            "mcp:weather:mcp.start".to_string(),
        ],
        "reconcile must prove the stop AND start the enabled new config"
    );
    assert!(report.still_needs_reconcile.is_empty());
    assert_eq!(
        fixture
            .service
            .operation(&op.operation_id)
            .expect("op")
            .state,
        OperationState::Completed
    );
    // The convergence actually brought the desired runtime up.
    assert_eq!(fixture.runtime.state_of("weather").as_deref(), Some("running"));
}

/// A desired-disabled update that bailed at the stop gate converges to a
/// stopped runtime without ever starting a new process.
#[tokio::test]
async fn reconcile_converges_an_update_to_a_disabled_desired_without_starting() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = true;
    fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            true,
            "key-1",
            "test",
        )
        .await
        .expect_err("stop unconfirmed");
    let op = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0]
        .clone();

    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = false;
    fixture.runtime.set_state("weather", "stopped");

    let report = fixture.service.reconcile().await.expect("reconcile");
    assert_eq!(
        report.mcp_effects_recovered,
        vec!["mcp:weather:mcp.stop".to_string()]
    );
    assert!(report.still_needs_reconcile.is_empty());
    assert_eq!(
        fixture
            .service
            .operation(&op.operation_id)
            .expect("op")
            .state,
        OperationState::Completed
    );
    assert!(
        !fixture
            .runtime
            .calls()
            .iter()
            .any(|call| call.starts_with("start:")),
        "a disabled desired must never be started"
    );
}

/// If the new configuration cannot be started during convergence, the update
/// ends as a structured failure and is never reported complete.
#[tokio::test]
async fn reconcile_fails_an_update_when_the_new_config_cannot_start() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = true;
    fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .expect_err("stop unconfirmed");
    let op = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0]
        .clone();

    // Old gone, but the new server refuses to start.
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = false;
    fixture.runtime.set_state("weather", "stopped");
    *fixture.runtime.start_result.lock().expect("start_result") =
        Some("handshake failed".to_string());

    let report = fixture.service.reconcile().await.expect("reconcile");
    assert!(report.still_needs_reconcile.is_empty());
    let op = fixture.service.operation(&op.operation_id).expect("op");
    assert_eq!(op.state, OperationState::Failed);
    assert!(fixture.runtime.state_of("weather").as_deref() != Some("running"));
}

/// A start that answers but is never observable keeps the update in
/// `needs_reconcile` rather than falsely completing it.
#[tokio::test]
async fn reconcile_keeps_an_update_needs_reconcile_when_the_start_is_unobservable() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = true;
    fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .expect_err("stop unconfirmed");
    let op_id = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0]
        .operation_id
        .clone();

    // Old gone; the start answers but the server never shows up.
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = false;
    fixture.runtime.set_state("weather", "stopped");
    *fixture.runtime.silent_start.lock().expect("silent_start") = true;

    let report = fixture.service.reconcile().await.expect("reconcile");
    assert_eq!(report.still_needs_reconcile, vec![op_id.clone()]);
    let op = fixture.service.operation(&op_id).expect("op");
    assert_eq!(op.state, OperationState::NeedsReconcile);
    assert_eq!(
        op.reconcile_reason.as_deref(),
        Some("update_reconcile_start_not_observable")
    );
}


/// The happy path still swaps: the old runtime is confirmed stopped before the
/// new configuration starts, in that order.
#[tokio::test]
async fn an_update_confirms_the_old_stop_before_starting_the_new_config() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");

    let result = fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .expect("update");

    assert_eq!(result.result["status"], "updated");
    assert_eq!(
        fixture.runtime.calls(),
        vec!["stop:weather".to_string(), "start:weather".to_string()],
        "the old runtime must stop, and only then the new one start"
    );
    assert_eq!(
        fixture
            .service
            .operation(&result.operation_id)
            .expect("op")
            .state,
        OperationState::Completed
    );
}

/// A live update whose new runtime answers "started" but is never observable
/// must not report the swap complete: it records `mcp.start` as Unknown and
/// leaves the operation durably reconcilable, so reconcile can later roll the
/// enable forward rather than leaving desired enabled with a stopped runtime.
#[tokio::test]
async fn an_update_that_cannot_observe_the_new_runtime_needs_reconcile() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    // The stop confirms; the start answers but the server never shows up.
    *fixture.runtime.silent_start.lock().expect("silent_start") = true;

    let error = fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .err()
        .expect("an unobservable enable cannot be a success");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);

    let op = &fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0];
    assert_eq!(
        op.reconcile_reason.as_deref(),
        Some("update_start_not_observable")
    );
    assert_eq!(
        fixture
            .service
            .repository()
            .list_effects(&op.operation_id)
            .expect("effects")
            .into_iter()
            .find(|effect| effect.effect_key == "mcp.start")
            .expect("a start effect was recorded")
            .outcome,
        EffectOutcome::Unknown,
        "the unproven start must be recorded, never silently completed"
    );
    assert_eq!(
        fixture.runtime.calls(),
        vec!["stop:weather".to_string(), "start:weather".to_string()]
    );
}

/// A live update whose new start never answers is a timed-out effect: the
/// journal records an Unknown `mcp.start` and keeps the operation reconcilable,
/// so the enable is neither reported done nor blindly retried.
#[tokio::test]
async fn an_update_start_that_never_answers_needs_reconcile() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    // Only the start hangs; the old stop still confirms.
    *fixture.runtime.start_hang.lock().expect("start_hang") = true;

    let error = fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .err()
        .expect("the bounded start wait must fire");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);

    let op = &fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0];
    assert_eq!(op.reconcile_reason.as_deref(), Some("update_start_timed_out"));
    assert_eq!(
        fixture
            .service
            .repository()
            .list_effects(&op.operation_id)
            .expect("effects")
            .into_iter()
            .find(|effect| effect.effect_key == "mcp.start")
            .expect("a start effect was recorded")
            .outcome,
        EffectOutcome::Unknown
    );
}

/// A deterministic start failure on a live update is reported as `Failed` with
/// the swap status, not as a reconcile or a false completion.
#[tokio::test]
async fn an_update_start_that_fails_is_reported_failed_not_complete() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    *fixture
        .runtime
        .start_result
        .lock()
        .expect("start_result") = Some("handshake refused".to_string());

    let error = fixture
        .service
        .mcp_update(
            id,
            "weather",
            "updated",
            blank_secret_config("weather"),
            false,
            "key-1",
            "test",
        )
        .await
        .err()
        .expect("a deterministic start failure is an error");
    assert_eq!(error.code(), code::INTERNAL);

    let op_id = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list");
    assert!(
        op_id.is_empty(),
        "a proven start failure must not masquerade as an unknown reconcile case"
    );

    // The most recent operation is Failed with the updated_but_start_failed shape.
    let all = fixture
        .service
        .repository()
        .list_by_resource(CapabilityKind::Mcp, "mcp:weather", 10)
        .expect("operations");
    let failed = all
        .iter()
        .find(|op| op.state == OperationState::Failed)
        .expect("a failed operation");
    assert_eq!(
        fixture
            .service
            .repository()
            .list_effects(&failed.operation_id)
            .expect("effects")
            .into_iter()
            .find(|effect| effect.effect_key == "mcp.start")
            .expect("a start effect was recorded")
            .outcome,
        EffectOutcome::Failed
    );
    assert_eq!(
        failed.result.as_ref().and_then(|r| r["status"].as_str()),
        Some("updated_but_start_failed")
    );
}

// ------------------------------------------------------------------- enable

#[tokio::test]
async fn enable_reports_desired_and_observed_state_separately() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));

    let result = fixture
        .service
        .mcp_enable(id, "key-1", "test")
        .await
        .expect("enable");

    assert_eq!(result.result["status"], "enabled");
    assert_eq!(result.result["desired_enabled"], json!(true));
    // The nested view keeps the two facts apart (INV-7).
    assert_eq!(result.result["server"]["desired"]["enabled"], json!(true));
    assert_eq!(result.result["server"]["runtime"]["state"], "running");
    assert_eq!(result.result["server"]["runtime"]["observed"], json!(true));
    assert!(!fixture.repository.snapshot()[0].disabled);
    assert_eq!(fixture.runtime.calls(), vec!["start:weather".to_string()]);
}

/// A cold start that only becomes observable after the process has connected
/// must be confirmed by polling, not by a single observation that races the
/// transition. `register_mcp_server` answers before a cold child is up, so a
/// start that reports success and later reaches `running` inside the window is a
/// genuine success and completes the operation with a proven running runtime.
#[tokio::test]
async fn a_cold_start_that_becomes_running_is_proven_by_polling() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    // `start` is accepted but leaves the server `starting`; it reaches
    // `running` only after a few observations, like a slow npx handshake.
    *fixture.runtime.start_state.lock().expect("start_state") =
        Some("starting".to_string());
    *fixture.runtime.running_after_observations.lock().expect("running_after") = 3;

    let result = fixture
        .service
        .mcp_enable(id, "key-1", "test")
        .await
        .expect("a start that becomes running must be proven, not needs_reconcile");

    assert_eq!(result.result["status"], "enabled");
    assert_eq!(result.result["server"]["runtime"]["state"], "running");
    assert_eq!(result.result["server"]["runtime"]["observed"], json!(true));
    // The confirmation had to observe more than once to catch the transition.
    assert!(
        fixture.runtime.observe_count.load(Ordering::SeqCst) >= 2,
        "the bounded start confirmation must poll, not observe once"
    );
    let op = fixture
        .service
        .repository()
        .list_by_resource(CapabilityKind::Mcp, "mcp:weather", 5)
        .expect("operations")
        .into_iter()
        .next()
        .expect("one operation");
    assert_eq!(op.state, OperationState::Completed);
    assert_eq!(
        fixture
            .service
            .repository()
            .list_effects(&op.operation_id)
            .expect("effects")
            .into_iter()
            .find(|effect| effect.effect_key == "mcp.start")
            .expect("a start effect")
            .outcome,
        EffectOutcome::Applied,
        "an observed running start is Applied"
    );
}

/// A start that never reaches `running` within the bounded window stays
/// fail-closed: the effect is recorded `Unknown` and the operation is left for
/// reconciliation, never completed on a guess. This is the same invariant that
/// must hold when the single immediate observation used to falsely short-circuit.
#[tokio::test]
async fn a_start_that_never_becomes_running_in_the_window_needs_reconcile() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    // Accepted, stuck in `starting`, and never flips to running.
    *fixture.runtime.start_state.lock().expect("start_state") =
        Some("starting".to_string());

    let error = fixture
        .service
        .mcp_enable(id, "key-1", "test")
        .await
        .err()
        .expect("an unproven start cannot be a success");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);

    let op = &fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list")[0];
    assert_eq!(op.reconcile_reason.as_deref(), Some("start_not_observable"));
    assert_eq!(
        fixture
            .service
            .repository()
            .list_effects(&op.operation_id)
            .expect("effects")
            .into_iter()
            .find(|effect| effect.effect_key == "mcp.start")
            .expect("a start effect")
            .outcome,
        EffectOutcome::Unknown,
        "the unproven start must be recorded, never silently completed"
    );
    assert!(
        fixture.runtime.observe_count.load(Ordering::SeqCst) >= 2,
        "the start confirmation polled the whole window"
    );
}

#[tokio::test]
async fn a_failed_start_keeps_the_desired_state_and_reports_drift_rather_than_running() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    *fixture.runtime.start_result.lock().expect("start_result") =
        Some("the executable is missing".to_string());

    let error = fixture
        .service
        .mcp_enable(id, "key-1", "test")
        .await
        .err()
        .expect("start failure");
    assert_eq!(error.code(), code::INTERNAL);

    // Desired is enabled, runtime is not: that is drift, not a false success.
    assert!(!fixture.repository.snapshot()[0].disabled);
    let operation = fixture
        .service
        .repository()
        .list_by_resource(CapabilityKind::Mcp, "mcp:weather", 5)
        .expect("operations")
        .into_iter()
        .next()
        .expect("one operation");
    assert_eq!(operation.state, OperationState::Failed);
    let view = fixture.service.mcp_status(id).await.expect("status");
    assert!(view.desired.enabled);
    assert_ne!(view.runtime.state, "running");
    assert_eq!(
        view.drift.as_deref(),
        Some(crate::capability::mcp_service::DRIFT_DESIRED_BUT_NOT_RUNNING)
    );
}

#[tokio::test]
async fn an_unanswered_start_ends_in_needs_reconcile() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    *fixture.runtime.hang.lock().expect("hang") = true;

    let error = fixture
        .service
        .mcp_enable(id, "key-1", "test")
        .await
        .err()
        .expect("a bounded wait must fire");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);

    let operations = fixture
        .service
        .repository()
        .list_needing_reconcile()
        .expect("reconcile list");
    assert_eq!(operations.len(), 1);
    assert_eq!(
        operations[0].reconcile_reason.as_deref(),
        Some("start_timed_out")
    );
    // The effect row stays unproven so recovery cannot retry it blindly.
    assert_eq!(
        fixture
            .service
            .repository()
            .count_unproven_effects(&operations[0].operation_id)
            .expect("count"),
        1
    );
}

#[tokio::test]
async fn a_start_that_reports_success_but_is_not_observable_needs_reconcile() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    // The runtime answers "started" but never shows the server.
    *fixture.runtime.silent_start.lock().expect("silent_start") = true;

    let error = fixture
        .service
        .mcp_enable(id, "key-1", "test")
        .await
        .err()
        .expect("an unobservable start cannot be a success");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);
    assert_eq!(
        fixture
            .service
            .repository()
            .list_needing_reconcile()
            .expect("reconcile")[0]
            .reconcile_reason
            .as_deref(),
        Some("start_not_observable")
    );
}

// ----------------------------------------------------------------- disable

#[tokio::test]
async fn disable_stops_and_confirms_before_it_reports_success() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");

    let result = fixture
        .service
        .mcp_disable(id, "key-1", "test")
        .await
        .expect("disable");
    assert_eq!(result.result["status"], "disabled");
    assert_eq!(result.result["stop_confirmed"], json!(true));
    assert!(fixture.repository.snapshot()[0].disabled);
    assert_eq!(fixture.runtime.calls(), vec!["stop:weather".to_string()]);
    assert_eq!(
        fixture
            .service
            .repository()
            .list_needing_reconcile()
            .expect("reconcile")
            .len(),
        0
    );
}

#[tokio::test]
async fn an_unconfirmed_stop_leaves_the_disabled_record_in_needs_reconcile() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    // The runtime keeps reporting the server as running no matter what.
    fixture.runtime.set_state("weather", "running");
    *fixture.runtime.hang.lock().expect("hang") = true;

    let error = fixture
        .service
        .mcp_disable(id, "key-1", "test")
        .await
        .err()
        .expect("an unconfirmed stop must not claim success");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);
    assert!(
        fixture.repository.snapshot()[0].disabled,
        "the desired state was still changed, so a reconcile can see it"
    );
}

// --------------------------------------------------------------- uninstall

#[tokio::test]
async fn uninstall_disables_then_stops_and_only_then_deletes() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");

    let result = fixture
        .service
        .mcp_uninstall(id, "key-1", "test")
        .await
        .expect("uninstall");

    assert_eq!(result.result["status"], "uninstalled");
    assert!(fixture.repository.snapshot().is_empty());
    assert!(fixture.runtime.state_of("weather").is_none());
    // The order is the safety property: stop happened, delete happened.
    assert_eq!(fixture.runtime.calls(), vec!["stop:weather".to_string()]);
    let effects = fixture
        .service
        .repository()
        .list_effects(&result.operation_id)
        .expect("effects");
    let keys: Vec<&str> = effects.iter().map(|effect| effect.effect_key.as_str()).collect();
    assert!(keys.contains(&"mcp.stop"), "got {keys:?}");
    assert!(keys.contains(&"mcp.delete"), "got {keys:?}");
    assert!(effects
        .iter()
        .all(|effect| effect.outcome == crate::capability::types::EffectOutcome::Applied));
}

#[tokio::test]
async fn uninstall_keeps_the_record_when_the_stop_cannot_be_confirmed() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    // The stop call answers, yet the server is still observable afterwards.
    *fixture.runtime.stubborn_stop.lock().expect("stubborn_stop") = true;

    let error = fixture
        .service
        .mcp_uninstall(id, "key-1", "test")
        .await
        .err()
        .expect("an unconfirmed stop must block the delete");
    assert_eq!(error.code(), code::NEEDS_RECONCILE);
    assert_eq!(
        fixture.repository.snapshot().len(),
        1,
        "the record must survive an unproven stop (AC-10)"
    );
    assert!(fixture.repository.snapshot()[0].disabled);
    assert_eq!(
        fixture.repository.delete_calls.load(Ordering::SeqCst),
        0,
        "nothing may be deleted while the runtime is unproven"
    );
    assert!(fixture
        .runtime
        .calls()
        .contains(&"stop:weather".to_string()));
}

#[tokio::test]
async fn uninstall_of_an_unknown_id_is_not_found() {
    let fixture = fixture();
    let error = fixture
        .service
        .mcp_uninstall(4242, "key-1", "test")
        .await
        .err()
        .expect("unknown id");
    assert_eq!(error.code(), code::NOT_FOUND);
    assert_eq!(fixture.repository.delete_calls.load(Ordering::SeqCst), 0);
}

// ----------------------------------------------------------- tools/refresh

#[tokio::test]
async fn listing_tools_never_starts_or_invokes_anything() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture
        .runtime
        .tools
        .lock()
        .expect("tools")
        .insert("weather".to_string(), vec!["forecast".to_string()]);

    let snapshot = fixture.service.mcp_tools(id).await.expect("tools");
    assert_eq!(snapshot.source, "runtime");
    assert_eq!(snapshot.tools.len(), 1);
    assert_eq!(snapshot.tools[0]["name"], "forecast");
    // Nothing was asked of the runtime except a list read.
    assert_eq!(
        fixture.runtime.calls(),
        vec!["list_tools:weather".to_string()],
        "listing must not start, stop or call a tool (AC-11)"
    );
}

#[tokio::test]
async fn a_server_the_runtime_does_not_know_reports_a_stable_empty_result() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    let snapshot = fixture.service.mcp_tools(id).await.expect("tools read succeeds");
    assert_eq!(snapshot.source, "unavailable");
    assert!(snapshot.tools.is_empty());
    assert_eq!(snapshot.freshness, "unknown");
    assert!(snapshot.detail.is_some());
}

#[tokio::test]
async fn refresh_of_a_disabled_server_is_refused_without_touching_the_runtime() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    let error = fixture
        .service
        .mcp_refresh_tools(id, "key-1", "test")
        .await
        .err()
        .expect("refused");
    assert_eq!(error.code(), code::REFUSED);
    assert!(fixture.runtime.calls().is_empty());
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_last_known_snapshot_and_marks_it() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");
    fixture
        .runtime
        .tools
        .lock()
        .expect("tools")
        .insert("weather".to_string(), vec!["cached_tool".to_string()]);
    *fixture.runtime.refresh_result.lock().expect("refresh_result") =
        Some("connection reset".to_string());

    let result = fixture
        .service
        .mcp_refresh_tools(id, "key-1", "test")
        .await
        .expect("a failed refresh still answers structurally");
    assert_eq!(result.result["status"], "refresh_failed");
    assert_eq!(result.result["freshness"], "failed");
    assert_eq!(result.result["kept_last_known"][0]["name"], "cached_tool");

    let operation = fixture
        .service
        .operation(&result.operation_id)
        .expect("operation");
    assert_eq!(operation.state, OperationState::Failed);
    // Last-known tools survive the failure, which is the point of the snapshot.
    let snapshot = fixture.service.mcp_tools(id).await.expect("tools");
    assert_eq!(snapshot.tools.len(), 1);
}

#[tokio::test]
async fn a_refreshed_list_is_reported_fresh_afterwards() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, false));
    fixture.runtime.set_state("weather", "running");

    let result = fixture
        .service
        .mcp_refresh_tools(id, "key-1", "test")
        .await
        .expect("refresh");
    assert_eq!(result.result["status"], "refreshed");
    assert_eq!(result.result["tool_count"], json!(2));

    let snapshot = fixture.service.mcp_tools(id).await.expect("tools");
    assert_eq!(snapshot.freshness, "fresh");
    assert!(snapshot.last_refreshed_at_ms.is_some());
}

// ------------------------------------------------------- concurrency, crash

#[tokio::test]
async fn two_operations_on_one_server_serialize_their_runtime_effects() {
    let fixture = fixture();
    let id = fixture.repository.seed(record("weather", 1, true));
    fixture.runtime.set_state("weather", "running");

    let enable_service = fixture.service.clone();
    let disable_service = fixture.service.clone();
    let enable = tokio::spawn(async move {
        enable_service
            .mcp_enable(id, "enable-1", "test")
            .await
            .map(|result| result.operation_id)
    });
    let disable = tokio::spawn(async move {
        disable_service
            .mcp_disable(id, "disable-1", "test")
            .await
            .map(|result| result.operation_id)
    });

    let (first, second) = (enable.await.expect("join"), disable.await.expect("join"));
    assert!(first.is_ok(), "enable: {first:?}");
    assert!(second.is_ok(), "disable: {second:?}");

    // The fake counts overlapping effect calls; more than one would mean the
    // two operations interleaved their effects on the same server.
    assert_eq!(
        fixture.runtime.max_concurrent.load(Ordering::SeqCst),
        1,
        "runtime effects overlapped: {:?}",
        fixture.runtime.calls()
    );
    // Both operations are journaled independently.
    assert_eq!(
        fixture
            .service
            .repository()
            .list_by_resource(CapabilityKind::Mcp, "mcp:weather", 10)
            .expect("operations")
            .len(),
        2
    );
}

#[tokio::test]
async fn startup_recovery_classifies_an_mcp_operation_with_an_unproven_effect() {
    let fixture = fixture();
    // Open an operation and record an intent, then simulate the crash: the
    // process never gets to record the outcome.
    let operation = fixture
        .service
        .begin_operation(OperationRequest {
            capability: CapabilityKind::Mcp,
            operation_kind: "mcp.enable".to_string(),
            actor_scope: "test".to_string(),
            idempotency_key: "crashed-1".to_string(),
            request: json!({ "id": 1, "name": "weather" }),
            resource_key: "mcp:weather".to_string(),
        })
        .await
        .expect("begin")
        .into_operation();
    fixture
        .service
        .record_effect_intent(&operation.operation_id, "mcp.start", Some("weather"), None)
        .expect("intent");

    let report = fixture
        .service
        .recover_interrupted_operations()
        .expect("recovery");
    assert_eq!(report.needs_reconcile, vec![operation.operation_id.clone()]);
    assert!(report.failed_before_effect.is_empty());

    // The reconcile path must not delete the record it is unsure about.
    assert_eq!(fixture.repository.delete_calls.load(Ordering::SeqCst), 0);
    let recovered = fixture
        .service
        .operation(&operation.operation_id)
        .expect("operation");
    assert_eq!(recovered.state, OperationState::NeedsReconcile);
}

// ----------------------------------------------------------------- reconcile

/// Opens an MCP operation, records one unproven effect, and crashes (recovers)
/// so the operation enters `needs_reconcile`.
async fn crashed_mcp_operation(
    fixture: &Fixture,
    operation_kind: &str,
    effect_key: &str,
    key: &str,
) -> String {
    let operation = fixture
        .service
        .begin_operation(OperationRequest {
            capability: CapabilityKind::Mcp,
            operation_kind: operation_kind.to_string(),
            actor_scope: "test".to_string(),
            idempotency_key: key.to_string(),
            request: json!({ "id": 1, "name": "weather" }),
            resource_key: "mcp:weather".to_string(),
        })
        .await
        .expect("begin")
        .into_operation();
    fixture
        .service
        .record_effect_intent(&operation.operation_id, effect_key, Some("weather"), None)
        .expect("intent");
    fixture
        .service
        .recover_interrupted_operations()
        .expect("recovery");
    operation.operation_id
}

#[tokio::test]
async fn reconcile_completes_an_mcp_operation_whose_start_is_proven_running() {
    let fixture = fixture();
    let op = crashed_mcp_operation(&fixture, "mcp.enable", "mcp.start", "reconcile-start").await;
    // The runtime now answers that the server is up, proving the effect.
    fixture.runtime.set_state("weather", "running");

    let report = fixture.service.reconcile().await.expect("reconcile");

    assert_eq!(report.mcp_effects_recovered, vec!["mcp:weather:mcp.start".to_string()]);
    assert!(report.still_needs_reconcile.is_empty());
    assert_eq!(
        fixture.service.operation(&op).expect("op").state,
        OperationState::Completed,
        "a proven start completes the operation"
    );
}

#[tokio::test]
async fn reconcile_completes_an_mcp_operation_whose_stop_is_proven() {
    let fixture = fixture();
    let op = crashed_mcp_operation(&fixture, "mcp.disable", "mcp.stop", "reconcile-stop").await;
    // No state set: the runtime proves the server is not running.

    let report = fixture.service.reconcile().await.expect("reconcile");

    assert_eq!(report.mcp_effects_recovered, vec!["mcp:weather:mcp.stop".to_string()]);
    assert_eq!(
        fixture.service.operation(&op).expect("op").state,
        OperationState::Completed
    );
}

#[tokio::test]
async fn reconcile_keeps_an_mcp_delete_needs_reconcile_while_the_record_remains() {
    let fixture = fixture();
    // The delete effect crashed, but persistence still holds the record, so the
    // effect is unproven and must not be rolled forward or re-deleted.
    fixture.repository.seed(record("weather", 1, false));
    let op =
        crashed_mcp_operation(&fixture, "mcp.uninstall", "mcp.delete", "reconcile-del").await;

    let report = fixture.service.reconcile().await.expect("reconcile");

    assert!(report.mcp_effects_recovered.is_empty());
    assert_eq!(report.still_needs_reconcile, vec![op.clone()]);
    assert_eq!(
        fixture.service.operation(&op).expect("op").state,
        OperationState::NeedsReconcile,
        "an unproven effect is preserved, never blind-retried"
    );
    assert_eq!(
        fixture.repository.delete_calls.load(Ordering::SeqCst),
        0,
        "reconcile never deletes persistence it cannot prove"
    );
    assert!(
        fixture.repository.find_by_name("weather").expect("lookup").is_some(),
        "the record survives until proven"
    );
}

#[tokio::test]
async fn reconcile_keeps_an_mcp_stop_needs_reconcile_when_the_runtime_still_runs() {
    let fixture = fixture();
    let op =
        crashed_mcp_operation(&fixture, "mcp.disable", "mcp.stop", "reconcile-running").await;
    // The server is still running, so a stop is not proven.
    fixture.runtime.set_state("weather", "running");

    let report = fixture.service.reconcile().await.expect("reconcile");

    assert!(report.mcp_effects_recovered.is_empty());
    assert_eq!(report.still_needs_reconcile, vec![op.clone()]);
    assert_eq!(
        fixture.service.operation(&op).expect("op").state,
        OperationState::NeedsReconcile,
        "a stop the runtime contradicts is not reported as done"
    );
}

// ------------------------------------------------------------- boundary guard

/// Reads a source file relative to the crate root.
fn source(path: &str) -> String {
    let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|error| {
        panic!("cannot read {path}: {error}");
    })
}

/// Reports every forbidden substring present in `text`.
fn present<'a>(text: &str, needles: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    needles
        .into_iter()
        .filter(|needle| text.contains(needle))
        .collect()
}

/// Reports every forbidden *call* of each named primitive.
///
/// Only call syntax counts: a wrapper legitimately shares a word prefix with a
/// primitive (`delete_mcp_server` versus `delete_mcp`), and matching bare names
/// would report the adapter's own command names as violations.
fn present_calls<'a>(text: &str, primitives: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    primitives
        .into_iter()
        .filter(|name| text.contains(&format!(".{name}(")) || text.contains(&format!("::{name}(")))
        .collect()
}

/// AC-1 / INV-1: the adapters hold exactly one mutation path.
///
/// A second implementation is only detectable at review time unless it is
/// mechanically forbidden, so this guard names the primitives that must appear
/// solely inside the capability service. It is a source-text check on purpose:
/// the moment an adapter reaches for a runtime or store primitive directly, this
/// test fails instead of relying on a reviewer noticing.
#[test]
fn adapters_never_call_an_mcp_mutation_primitive_directly() {
    const PRIMITIVES: [&str; 8] = [
        "register_mcp_server",
        "unregister_mcp_server",
        "start_mcp_server",
        "stop_mcp_server",
        "refresh_mcp_server_tools",
        "disable_mcp_tool",
        "change_mcp_status",
        "delete_mcp",
    ];

    // The Tauri adapter keeps only the two read paths (the page's list and the
    // manual tool tester) and delegates every mutation.
    let commands = source("src/commands/mcp.rs");
    assert!(
        present_calls(&commands, PRIMITIVES).is_empty(),
        "commands/mcp.rs must delegate: {:?}",
        present_calls(&commands, PRIMITIVES)
    );
    assert!(
        !commands.contains("tokio::spawn"),
        "an untracked mutation task would report success the journal cannot prove"
    );
    assert!(
        !commands.contains("add_mcp(") && !commands.contains("update_mcp("),
        "the store may not be written from the command layer"
    );

    // The HTTP adapter may only call the facade.
    let http = source("src/workflow/react/client/http/server.rs");
    assert!(
        present_calls(&http, PRIMITIVES).is_empty(),
        "the HTTP adapter must delegate: {:?}",
        present_calls(&http, PRIMITIVES)
    );
    // No blanket `tokio::spawn` ban here: the control plane legitimately spawns
    // for run streaming. What must not happen is a *capability* handler hiding
    // an untracked mutation, which the primitive check above already forbids.

    // The CLI is an HTTP client. It may read a local descriptor *file* the user
    // named, but it must never become a second owner of the data: it cannot reach
    // the store, the database or the runtime, because it does not depend on them.
    // Path strings do appear in its fixtures, so the ban is on the owners, not on
    // any mention of a directory.
    const CLI_FORBIDDEN: [&str; 4] = [
        "MainStore",
        "ToolManager",
        "rusqlite",
        "chatspeed_lib::db",
    ];
    for path in [
        "src/bin/cs/mcp.rs",
        "src/bin/cs/skill.rs",
        "src/bin/cs/capability.rs",
    ] {
        let text = source(path);
        assert!(
            present(&text, CLI_FORBIDDEN).is_empty(),
            "{path} must stay a thin HTTP client: {:?}",
            present(&text, CLI_FORBIDDEN)
        );
    }
}

/// AC-14: the phase stays inside its declared scope.
#[test]
fn the_capability_surface_does_not_grow_an_export_or_automation_path() {
    for path in [
        "src/capability/mod.rs",
        "src/capability/mcp/mod.rs",
        "src/capability/skill/mod.rs",
    ] {
        let text = source(path);
        assert!(
            present(
                &text,
                ["export", "import_all", "automation", "benchmark", "marketplace"]
            )
            .is_empty(),
            "{path} must not introduce an excluded capability"
        );
    }
}
