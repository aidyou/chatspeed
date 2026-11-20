//! Isolated Tokio thread. Only list, snapshot, history-query and authenticated SSE are used.
use crate::{model::Model, native, types::*};
use chatspeed_runtime_client::RuntimeClient;
use serde_json::{json, Value};
use std::{
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

/// Owns shutdown: dropping the window cancels all network futures and joins its thread.
pub struct Worker {
    pub commands: mpsc::Sender<Command>,
    pub updates: mpsc::Receiver<Update>,
    stop: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn start(options: Options) -> std::io::Result<Self> {
        if let Some(path) = &options.metrics {
            crate::metrics::Metrics::prepare(path)?;
            crate::metrics::Metrics {
                started: Some(Instant::now()),
                ..Default::default()
            }
            .emit(Some(path))?;
        }
        let (commands, rx) = mpsc::channel(16);
        let (tx, updates) = mpsc::channel(128);
        let (stop, mut stopped) = watch::channel(false);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("gpui-observer-io".into())
            .spawn(move || {
                runtime.block_on(async move {
                    tokio::select! {
                        _ = stopped.changed() => {},
                        _ = coordinate(options, rx, tx) => {},
                    }
                });
                // Runtime drop cancels every selected-session and list task as well.
            })?;
        Ok(Self {
            commands,
            updates,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

async fn send(tx: &mpsc::Sender<Update>, generation: u64, kind: UpdateKind) -> bool {
    let queued = Instant::now();
    let Ok(permit) = tx.reserve().await else {
        return false;
    };
    let queue_depth = tx.max_capacity() - tx.capacity();
    permit.send(Update {
        generation,
        queued,
        queue_depth,
        kind,
    });
    true
}

async fn coordinate(options: Options, mut rx: mpsc::Receiver<Command>, tx: mpsc::Sender<Update>) {
    let list_options = options.clone();
    let list_tx = tx.clone();
    tokio::spawn(async move {
        loop {
            if list_options.demo {
                if !send(
                    &list_tx,
                    0,
                    UpdateKind::Sessions(
                        (1..=3)
                            .map(|n| Session {
                                id: format!("demo-{n}"),
                                title: format!("{} {n}", rust_i18n::t!("demo")),
                                status: "executing".into(),
                                ..Session::default()
                            })
                            .collect(),
                    ),
                )
                .await
                {
                    return;
                }
            } else {
                // Locale and appearance follow the runtime configuration, so a
                // setting change shows up on the next poll without a restart.
                let fetched_at = Instant::now();
                if let Ok(config) = native::config(&list_options).await {
                    if !send(&list_tx, 0, UpdateKind::Config(config, fetched_at)).await {
                        return;
                    }
                }
                match native::connect(&list_options).await {
                    Some(client) => {
                        if let Ok(value) = client.get("/control/v1/workflows").await {
                            if let Ok(sessions) = serde_json::from_value(value) {
                                if !send(&list_tx, 0, UpdateKind::Sessions(sessions)).await
                                    || !send(&list_tx, 0, UpdateKind::Connected(true)).await
                                {
                                    return;
                                }
                            }
                        }
                    }
                    None => {
                        if !send(&list_tx, 0, UpdateKind::Connected(false)).await {
                            return;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    let mut selection: Option<tokio::task::JoinHandle<()>> = None;
    while let Some(command) = rx.recv().await {
        match command {
            Command::DesktopMenu(action) => {
                use chatspeed_runtime_client::{desktop_menu, try_discovery_path};
                let accepted = if options.demo {
                    false
                } else if let Ok(discovery) = try_discovery_path(options.discovery.as_deref()) {
                    let dir = discovery
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or_else(|| std::path::Path::new("."));
                    desktop_menu::request(dir, action).await.is_ok()
                } else {
                    false
                };
                if !accepted && !send(&tx, 0, UpdateKind::DesktopMenuUnavailable).await {
                    return;
                }
            }
            Command::SetSound { chime, muted } => {
                let saved = options.demo
                    || native::data_command(
                        &options,
                        "set_config",
                        json!({"key": chime.config_key(), "value": muted}),
                    )
                    .await
                    .is_ok();
                if !send(
                    &tx,
                    0,
                    UpdateKind::SoundSaved {
                        chime,
                        muted,
                        saved,
                    },
                )
                .await
                {
                    return;
                }
            }
            Command::Metrics(mut metrics, path) => {
                // Low-frequency proc sampling and append-only writes stay off the UI thread.
                let _ = tokio::task::spawn_blocking(move || {
                    if metrics.emit(path.as_deref()).is_err() {
                        eprintln!("{}", rust_i18n::t!("metrics_error"));
                    }
                })
                .await;
            }
            Command::Select { generation, id } => {
                if let Some(task) = selection.take() {
                    task.abort();
                }
                let options = options.clone();
                let tx = tx.clone();
                selection = Some(tokio::spawn(async move {
                    follow(options, tx, generation, id).await;
                }));
            }
            Command::Signal { id, signal } => {
                if options.demo {
                    continue;
                }
                let path = format!("/control/v1/workflows/{id}/signal");
                if let Err(error) = native::control_post(&options, &path, signal).await {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::Stop { id } => {
                if options.demo {
                    continue;
                }
                let path = format!("/control/v1/workflows/{id}/stop");
                if let Err(error) = native::control_post(&options, &path, json!({})).await {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::StartNew {
                prompt,
                attached_context,
                metadata,
            } => {
                if options.demo {
                    if !send(
                        &tx,
                        0,
                        UpdateKind::Error("demo mode cannot start a session".into()),
                    )
                    .await
                    {
                        return;
                    }
                    continue;
                }
                match start_session(&options, &prompt, attached_context, metadata).await {
                    Ok(id) => {
                        if !send(&tx, 0, UpdateKind::Opened(id)).await {
                            return;
                        }
                    }
                    Err(error) => {
                        if !send(&tx, 0, UpdateKind::Error(error)).await {
                            return;
                        }
                    }
                }
            }
            Command::Child { id, session } => {
                let path = format!("/control/v1/workflows/{session}");
                if let Ok(value) = native::control_get(&options, &path).await {
                    if let Ok(page) = serde_json::from_value(value) {
                        if !send(&tx, 0, UpdateKind::Child(id, page)).await {
                            return;
                        }
                    }
                }
            }
            Command::ListDir { path } => {
                let tx = tx.clone();
                tokio::task::spawn(async move {
                    let listing = tokio::task::spawn_blocking({
                        let path = path.clone();
                        move || crate::native::list_dir(&path)
                    })
                    .await;
                    let (entries, error) = match listing {
                        Ok(Ok(entries)) => (entries, None),
                        Ok(Err(error)) => (Vec::new(), Some(error)),
                        Err(error) => (Vec::new(), Some(error.to_string())),
                    };
                    // Directory listings are not tied to a selected session.
                    let _ = send(
                        &tx,
                        0,
                        UpdateKind::Dir {
                            path,
                            entries,
                            error,
                        },
                    )
                    .await;
                });
            }
            Command::Catalogue => {
                if options.demo {
                    continue;
                }
                let agents = native::control_get(&options, "/control/v1/agents")
                    .await
                    .ok()
                    .and_then(|value| value.as_array().cloned())
                    .unwrap_or_default();
                let models = native::data_command(&options, "get_all_ai_models", json!({}))
                    .await
                    .ok()
                    .and_then(|value| value.as_array().cloned())
                    .unwrap_or_default();
                let schemes = native::data_command(&options, "get_sandbox_schemes", json!({}))
                    .await
                    .ok()
                    .and_then(|value| value.as_array().cloned())
                    .unwrap_or_default();
                let skills =
                    native::workflow_command(&options, "get_system_skills", json!({}), None)
                        .await
                        .ok()
                        .and_then(|value| value.as_array().cloned())
                        .unwrap_or_default();
                let tools = native::data_command(&options, "get_available_tools", json!({}))
                    .await
                    .ok()
                    .and_then(|value| value.as_array().cloned())
                    .unwrap_or_default();
                if !send(
                    &tx,
                    0,
                    UpdateKind::Catalogue {
                        agents,
                        models,
                        skills,
                        schemes,
                        tools,
                    },
                )
                .await
                {
                    return;
                }
            }
            Command::SetAgent { session, agent } => {
                if options.demo {
                    continue;
                }
                let command = "update_workflow_agent_id";
                let params = json!({"session_id": session, "agent_id": agent});
                let result = native::workflow_command(
                    &options,
                    command,
                    params,
                    Some(&idempotency_key(command)),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetModelConfig { session, configs } => {
                if options.demo {
                    continue;
                }
                let command = "update_workflow_model_config";
                let params = json!({"session_id": session, "configs": configs});
                let result = native::workflow_command(
                    &options,
                    command,
                    params,
                    Some(&idempotency_key(command)),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetSandbox {
                session,
                mode,
                scheme,
            } => {
                if options.demo {
                    continue;
                }
                let command = "update_workflow_sandbox_config";
                let params = json!({
                    "session_id": session,
                    "execution_mode": mode,
                    "sandbox_scheme_id": scheme,
                });
                let result = native::workflow_command(
                    &options,
                    command,
                    params,
                    Some(&idempotency_key(command)),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::ClearContext { session } => {
                if options.demo {
                    continue;
                }
                let command = "workflow_begin_new_context_frame";
                let params = json!({"session_id": session});
                let result = native::workflow_command(
                    &options,
                    command,
                    params,
                    Some(&idempotency_key(command)),
                )
                .await;
                match result {
                    Ok(_) => {
                        let toast = rust_i18n::t!("clear_context_frame_done").to_string();
                        if !send(&tx, 0, UpdateKind::Toast(toast)).await {
                            return;
                        }
                    }
                    Err(error) => {
                        if !send(&tx, 0, UpdateKind::Error(error)).await {
                            return;
                        }
                    }
                }
            }
            Command::SetAutoCompress { session, enabled } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_auto_compress",
                    json!({"session_id": session, "auto_compress": enabled}),
                    Some(&idempotency_key("auto-compress")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetFinalAudit { session, enabled } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_final_audit",
                    json!({"session_id": session, "final_audit": enabled}),
                    Some(&idempotency_key("final-audit")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetApprovalLevel { session, level } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_approval_level",
                    json!({"session_id": session, "approval_level": level}),
                    Some(&idempotency_key("approval-level")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetPlanning { session, enabled } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_phase",
                    json!({
                        "session_id": session,
                        "phase": if enabled { "planning" } else { "standard" },
                    }),
                    Some(&idempotency_key("workflow-phase")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetPersonality {
                session,
                personality,
            } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_personality",
                    json!({"session_id": session, "personality": personality}),
                    Some(&idempotency_key("personality")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetSkills {
                session,
                enabled,
                selected,
            } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_skills_config",
                    json!({
                        "session_id": session,
                        "skill_enabled": enabled,
                        "selected_skills": selected,
                    }),
                    Some(&idempotency_key("skills-config")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SetAgentConfig { session, config } => {
                if options.demo {
                    continue;
                }
                let result = native::workflow_command(
                    &options,
                    "update_workflow_agent_config",
                    json!({
                        "session_id": session,
                        "agent_config": serde_json::to_string(&config).unwrap_or_default(),
                    }),
                    Some(&idempotency_key("agent-config")),
                )
                .await;
                if let Err(error) = result {
                    if !send(&tx, 0, UpdateKind::Error(error)).await {
                        return;
                    }
                }
            }
            Command::SearchFiles { paths, query } => {
                if options.demo {
                    continue;
                }
                let files = native::workflow_command(
                    &options,
                    "search_workspace_files",
                    json!({"paths": paths, "query": query}),
                    None,
                )
                .await
                .ok()
                .and_then(|value| value.as_array().cloned())
                .unwrap_or_default();
                if !send(&tx, 0, UpdateKind::Files { query, files }).await {
                    return;
                }
            }
            Command::Older {
                generation,
                id,
                before,
            } => {
                if options.demo {
                    if !send(
                        &tx,
                        generation,
                        UpdateKind::Older(Model::demo_history(&id, before.parse().unwrap_or(1))),
                    )
                    .await
                    {
                        return;
                    }
                    continue;
                }
                // This query cannot mutate runtime state. The compatibility route retains camelCase rows.
                if let Some(client) = native::connect(&options).await {
                    if let Ok(value) = client
                        .post(
                            "/control/v1/workflow-commands/get_earlier_workflow_message_page",
                            &json!({"session_id":id,"before_message_id":before}),
                        )
                        .await
                    {
                        if let Ok(page) = serde_json::from_value(value) {
                            if !send(&tx, generation, UpdateKind::Older(page)).await {
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(task) = selection {
        task.abort();
    }
}

/// A unique idempotency key for one user-initiated workflow command.
fn idempotency_key(command: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("gpui-{command}-{nanos}")
}

/// Creates a workflow session on the primary agent and starts it with `prompt`.
///
/// Matches the Vue new-workflow flow: create carries the query the list shows,
/// start carries the first prompt the engine runs.
async fn start_session(
    options: &Options,
    prompt: &str,
    attached_context: Option<String>,
    metadata: Option<Value>,
) -> Result<String, String> {
    let client = native::connect(options)
        .await
        .ok_or_else(|| "runtime unavailable".to_string())?;
    let agents = client
        .get("/control/v1/agents")
        .await
        .map_err(|error| error.to_string())?;
    let agent_id = agents
        .as_array()
        .and_then(|agents| {
            agents
                .iter()
                .find(|agent| agent["role"] == "primary")
                .or_else(|| agents.first())
        })
        .and_then(|agent| agent["id"].as_str())
        .ok_or_else(|| "no agent configured".to_string())?
        .to_owned();
    let created = client
        .post(
            "/control/v1/workflows",
            &json!({
                "user_query": prompt,
                "agent_id": agent_id,
                "allowed_paths": null,
                "auto_approve_plan": null,
                "final_audit": null,
                "inherited_agent_config": null,
            }),
        )
        .await
        .map_err(|error| error.to_string())?;
    let session_id = created["session_id"]
        .as_str()
        .ok_or_else(|| "runtime returned no session id".to_string())?
        .to_owned();
    client
        .post(
            &format!("/control/v1/workflows/{session_id}/start"),
            &json!({
                "session_id": session_id,
                "agent_id": agent_id,
                "initial_prompt": prompt,
                "initial_metadata": metadata,
                "initial_attached_context": attached_context,
                "planning_mode": null,
            }),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(session_id)
}

async fn snapshot(
    client: &RuntimeClient,
    id: &str,
    tx: &mpsc::Sender<Update>,
    generation: u64,
) -> bool {
    let Ok(value) = client.get(&format!("/control/v1/workflows/{id}")).await else {
        return false;
    };
    let decode_start = Instant::now();
    let Ok(page) = serde_json::from_value(value) else {
        return false;
    };
    if !send(
        tx,
        generation,
        UpdateKind::DecodeUs(decode_start.elapsed().as_micros() as u64),
    )
    .await
    {
        return false;
    }
    send(tx, generation, UpdateKind::Snapshot(page)).await
}

async fn follow(options: Options, tx: mpsc::Sender<Update>, generation: u64, id: String) {
    if options.demo {
        if !send(&tx, generation, UpdateKind::Connected(true)).await {
            return;
        }
        if !send(
            &tx,
            generation,
            UpdateKind::Snapshot(Model::demo_history(&id, 901)),
        )
        .await
        {
            return;
        }
        let mut i = 0u64;
        loop {
            let event = match i % 50 {
                0 => {
                    json!({"type":"tool_started","tool_call_id":"replay","tool_name":"demo","arguments":{"read_only":true}})
                }
                10 => json!({"type":"tool_completed","tool_call_id":"replay","result":{"ok":true}}),
                20 => json!({"type":"reasoning_chunk","content":"Reasoning delta. "}),
                49 => {
                    json!({"type":"message","message_id":(901+i/50).to_string(),"role":"assistant","content":format!("{id}: completed output {}", i/50)})
                }
                _ => {
                    json!({"type":"chunk","content":format!("{id}: Streaming **Markdown** delta. ")})
                }
            };
            if !send(&tx, generation, UpdateKind::Event(event)).await {
                return;
            }
            i = i.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let mut cursor: Option<String> = None;
    let mut chimes = crate::chimes::ChimeTracker::default();
    let mut needs_snapshot = true;
    loop {
        if !send(&tx, generation, UpdateKind::Reconnect).await {
            return;
        }
        if let Some(client) = native::connect(&options).await {
            // Resume ordinary transport drops without mixing snapshots and old replay.
            if needs_snapshot && !snapshot(&client, &id, &tx, generation).await {
                let _ = send(&tx, generation, UpdateKind::Connected(false)).await;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
            needs_snapshot = false;
            if let Ok(mut stream) = client.stream_workflow(&id, cursor.as_deref()).await {
                if !send(&tx, generation, UpdateKind::Connected(true)).await {
                    return;
                }
                let mut reconcile = tokio::time::interval(Duration::from_secs(5));
                reconcile.tick().await;
                loop {
                    tokio::select! {
                        next = stream.next_event() => {
                            match next {
                                Ok(Some(envelope)) => {
                                    let next_cursor = format!("{}:{}", envelope.server_instance_id, envelope.sequence);
                                    if cursor.as_deref().and_then(|value| value.rsplit_once(':'))
                                        .is_some_and(|(instance, sequence)| instance == envelope.server_instance_id
                                            && sequence.parse::<u64>().is_ok_and(|last| envelope.sequence <= last)) { continue; }
                                    cursor = Some(next_cursor);
                                    if let Some(chime) = chimes.event(&envelope.payload) {
                                        if !send(&tx, generation, UpdateKind::Chime(chime)).await { return; }
                                    }
                                    if !send(&tx, generation, UpdateKind::Event(envelope.payload)).await { return; }
                                }
                                Ok(None) => break,
                                Err(error) => {
                                    // The client decodes reset frames as Serialization, not a reset enum.
                                    // Treat every invalid envelope as a resync; never inspect error text.
                                    if matches!(error, chatspeed_runtime_client::ClientError::Serialization(_)) {
                                        cursor = None;
                                        chimes = crate::chimes::ChimeTracker::default();
                                        needs_snapshot = true;
                                        if !send(&tx, generation, UpdateKind::Reset).await { return; }
                                    }
                                    break;
                                }
                            }
                        }
                        _ = reconcile.tick() => {
                            // Bounded snapshot reconciliation closes the snapshot/subscription race,
                            // including a final durable message emitted before the stream attached.
                            if !snapshot(&client, &id, &tx, generation).await { break; }
                        }
                    }
                }
            }
        }
        if !send(&tx, generation, UpdateKind::Connected(false)).await {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
