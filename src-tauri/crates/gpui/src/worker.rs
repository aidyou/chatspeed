//! Isolated Tokio thread for runtime requests and authenticated workflow streams.
use crate::{
    chimes::{self, NotificationState},
    model::{Model, PAGE_LIMIT},
    native,
    types::*,
};
use chatspeed_runtime_client::RuntimeClient;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

fn automation_request(action: AutomationAction) -> (&'static str, Value) {
    match action {
        AutomationAction::Enable => ("enable", json!({})),
        AutomationAction::Disable => ("disable", json!({})),
        AutomationAction::Run => ("run", json!({})),
        AutomationAction::Delete => ("delete", json!({"confirm": true})),
    }
}

#[test]
fn automation_delete_requires_explicit_confirmation_payload() {
    assert_eq!(automation_request(AutomationAction::Delete), ("delete", json!({"confirm": true})));
    assert_eq!(automation_request(AutomationAction::Run), ("run", json!({})));
}

fn decode_rewind_response(value: Value) -> Result<bool, String> {
    #[derive(serde::Deserialize)]
    struct Response {
        deleted: bool,
    }

    serde_json::from_value::<Response>(value)
        .map(|response| response.deleted)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[test]
fn rewind_response_uses_runtime_deleted_field() {
    assert_eq!(decode_rewind_response(json!({"deleted": true})), Ok(true));
    assert_eq!(decode_rewind_response(json!({"deleted": false})), Ok(false));
    assert!(decode_rewind_response(json!({})).is_err());
    assert!(decode_rewind_response(json!({"deleted": "true"})).is_err());
}

/// Owns shutdown: dropping the window cancels all network futures and joins its thread.
pub struct Worker {
    pub commands: mpsc::Sender<Command>,
    pub updates: mpsc::Receiver<Update>,
    sidebar_collapsed: Option<watch::Sender<Option<bool>>>,
    geometry: Option<watch::Sender<Vec<(&'static str, Value)>>>,
    stop: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn save_geometry(&self, values: Vec<(&'static str, Value)>) {
        if let Some(tx) = &self.geometry {
            tx.send_replace(values);
        }
    }
    pub fn save_sidebar(&self, collapsed: bool) -> bool {
        self.sidebar_collapsed.as_ref().is_some_and(|tx| tx.send(Some(collapsed)).is_ok())
    }

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
        let (sidebar_collapsed, mut sidebar) = watch::channel(None);
        let (geometry, mut bounds) = watch::channel(Vec::<(&'static str, Value)>::new());
        let geometry_options = options.clone();
        let prefs_options = options.clone();
        let prefs_tx = tx.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("gpui-io".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let writer = tokio::spawn(async move {
                        while sidebar.changed().await.is_ok() {
                            let value = *sidebar.borrow_and_update();
                            if let Some(collapsed) = value.filter(|_| !prefs_options.demo) {
                                let result = tokio::time::timeout(Duration::from_secs(2), native::data_command(
                                    &prefs_options, "set_config",
                                    json!({"key": "gpui_workflow_sidebar_collapsed", "value": collapsed}),
                                )).await;
                                if !matches!(result, Ok(Ok(_))) {
                                    let _ = prefs_tx.try_send(Update {
                                        generation: 0, queued: Instant::now(), queue_depth: 0,
                                        kind: UpdateKind::Error(rust_i18n::t!("sidebar_save_failed").into_owned()),
                                    });
                                }
                            }
                        }
                    });
                    let geometry_writer = tokio::spawn(async move {
                        while bounds.changed().await.is_ok() {
                            while matches!(tokio::time::timeout(Duration::from_millis(300), bounds.changed()).await, Ok(Ok(()))) {}
                            let values = bounds.borrow_and_update().clone();
                            if !geometry_options.demo {
                                for (key, value) in values {
                                    let _ = tokio::time::timeout(Duration::from_millis(750), native::data_command(
                                        &geometry_options, "set_config", json!({"key": key, "value": value}),
                                    )).await;
                                }
                            }
                        }
                    });
                    tokio::select! {
                        _ = stopped.changed() => {},
                        _ = coordinate(options, rx, tx) => {},
                    }
                    let _ = writer.await;
                    let _ = geometry_writer.await;
                });
                // Runtime drop cancels every selected-session and list task as well.
            })?;
        Ok(Self {
            commands,
            updates,
            sidebar_collapsed: Some(sidebar_collapsed),
            geometry: Some(geometry),
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.sidebar_collapsed.take();
        self.geometry.take();
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
    let (watch_tx, watch_rx) = watch::channel(None::<String>);
    let (sessions_tx, sessions_rx) = watch::channel(Vec::<Session>::new());
    if !options.demo {
        tokio::spawn(watch_decisions(
            options.clone(),
            tx.clone(),
            watch_rx,
            sessions_rx,
        ));
    }
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
                match native::connect(&list_options).await {
                    Some(client) => {
                        let started = Instant::now();
                        let sessions = match client.get("/control/v1/workflows").await {
                            Ok(value) => {
                                serde_json::from_value::<Vec<Session>>(value).map_err(|_| "decode")
                            }
                            Err(_) => Err("request"),
                        };
                        match sessions {
                            Ok(sessions) => {
                                let _ = sessions_tx.send(sessions.clone());
                                if !send(&list_tx, 0, UpdateKind::Sessions(sessions)).await
                                    || !send(&list_tx, 0, UpdateKind::Connected(true)).await
                                {
                                    return;
                                }
                            }
                            Err(stage) => {
                                eprintln!(
                                    "chatspeed-gpui: session list refresh failed (stage={stage}, elapsed_ms={})",
                                    started.elapsed().as_millis(),
                                );
                                if !send(&list_tx, 0, UpdateKind::Connected(false)).await {
                                    return;
                                }
                            }
                        }
                        // Appearance is optional and must not delay publishing the list.
                        let fetched_at = Instant::now();
                        let config = tokio::time::timeout(
                            Duration::from_secs(5),
                            client.post(
                                &chatspeed_runtime_client::data_command_path("get_all_config"),
                                &json!({}),
                            ),
                        )
                        .await;
                        if let Ok(Ok(config)) = config {
                            let show_cost = config["show_today_cost_stats"].as_bool() == Some(true);
                            if !send(&list_tx, 0, UpdateKind::Config(config, fetched_at)).await {
                                return;
                            }
                            if show_cost {
                                let fetch = |command| {
                                    let path = chatspeed_runtime_client::data_command_path(command);
                                    let client = &client;
                                    async move { client.post(&path, &json!({})).await }
                                };
                                if let Ok((Ok(rows), Ok(providers))) =
                                    tokio::time::timeout(Duration::from_secs(5), async {
                                        tokio::join!(
                                            fetch("get_ccproxy_today_cost_stats"),
                                            fetch("get_all_ai_models")
                                        )
                                    })
                                    .await
                                {
                                    if !send(
                                        &list_tx,
                                        0,
                                        UpdateKind::TodayCost(native::today_cost(
                                            &rows, &providers,
                                        )),
                                    )
                                    .await
                                    {
                                        return;
                                    }
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
                if !id.starts_with("subagent_") && !id.starts_with("task_") {
                    let _ = watch_tx.send(Some(id.clone()));
                }
                if let Some(task) = selection.take() {
                    task.abort();
                }
                let options = options.clone();
                let tx = tx.clone();
                selection = Some(tokio::spawn(async move {
                    follow(options, tx, generation, id).await;
                }));
            }
            Command::SubmitQueuedMessage {
                generation,
                session,
                queue_id,
                signal,
            } => {
                let result = if options.demo {
                    Err(rust_i18n::t!("queued_message_send_failed").to_string())
                } else {
                    native::control_post(
                        &options,
                        &format!("/control/v1/workflows/{session}/signal"),
                        signal,
                    )
                    .await
                };
                if let Err(message) = result {
                    if !send(
                        &tx,
                        generation,
                        UpdateKind::QueueSubmissionFailed { queue_id, message },
                    )
                    .await
                    {
                        return;
                    }
                }
            }
            Command::ApprovalBatch { session, tool_ids } => {
                if options.demo {
                    continue;
                }
                let path = format!("/control/v1/workflows/{session}/signal");
                for id in tool_ids {
                    let signal =
                        json!({"type":"approval", "id":id, "approved":true, "approve_all":false});
                    if let Err(error) = native::control_post(&options, &path, signal).await {
                        if !send(&tx, 0, UpdateKind::Error(error)).await {
                            return;
                        }
                    }
                }
            }
            Command::SubmitTurn { generation, session, client_id, signal } => {
                let path = format!("/control/v1/workflows/{session}/signal");
                if let Err(message) = native::control_post(&options, &path, signal).await {
                    if !send(&tx, generation, UpdateKind::TurnUnconfirmed { client_id, message }).await {
                        return;
                    }
                }
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
            Command::Rewind {
                generation,
                session,
            } => {
                let result = native::workflow_command(
                    &options,
                    "delete_last_workflow_message",
                    json!({"session_id": session}),
                    Some(&idempotency_key("rewind")),
                )
                .await
                .and_then(decode_rewind_response);
                if !send(&tx, generation, UpdateKind::Rewound(result)).await {
                    return;
                }
            }
            Command::EditSession { session, title } => {
                let (command, payload) = match &title {
                    Some(title) => ("update_workflow_title", json!({"session_id": session, "title": title})),
                    None => ("delete_workflow", json!({"session_id": session})),
                };
                let result = native::workflow_command(
                    &options,
                    command,
                    payload,
                    Some(&idempotency_key(command)),
                ).await.map(|_| ());
                if !send(&tx, 0, UpdateKind::SessionEdited { session, title, result }).await {
                    return;
                }
            }
            Command::ListAutomations => {
                let result = native::control_get(&options, "/control/v1/automations")
                    .await
                    .and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()));
                if !send(&tx, 0, UpdateKind::Automations(result)).await {
                    return;
                }
            }
            Command::ActOnAutomation { id, action } => {
                let (operation, payload) = automation_request(action);
                let path = format!("/control/v1/automations/{id}/{operation}");
                let result = match native::connect(&options).await {
                    Some(client) => client.post_with_idempotency(&path, &payload, &idempotency_key(operation)).await.map(|_| ()).map_err(|error| error.to_string()),
                    None => Err(rust_i18n::t!("automation_action_failed").to_string()),
                };
                let update = match result {
                    Ok(()) => UpdateKind::Automations(native::control_get(&options, "/control/v1/automations").await.and_then(|value| serde_json::from_value(value).map_err(|error| error.to_string()))),
                    Err(error) => UpdateKind::Error(error),
                };
                if !send(&tx, 0, update).await {
                    return;
                }
            }
            Command::CreateAutomation(spec) => {
                let result = match native::connect(&options).await {
                    Some(client) => client
                        .post_with_idempotency(
                            "/control/v1/automations",
                            &spec,
                            &idempotency_key("create-automation"),
                        )
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    None => Err(rust_i18n::t!("automation_create_failed").to_string()),
                };
                if !send(&tx, 0, UpdateKind::AutomationCreated(result)).await {
                    return;
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
                        UpdateKind::StartFailed("demo mode cannot start a session".into()),
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
                        if !send(&tx, 0, UpdateKind::StartFailed(error)).await {
                            return;
                        }
                    }
                }
            }
            Command::Child { generation, session } => {
                let options = options.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let path = format!("/control/v1/workflows/{session}?limit={PAGE_LIMIT}");
                    let result = native::control_get(&options, &path).await
                        .and_then(|value| serde_json::from_value(value).map_err(|err| err.to_string()));
                    let _ = send(&tx, generation, UpdateKind::Child(session, result)).await;
                });
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
                // Never let a slow catalogue endpoint delay Select/StartNew;
                // the command loop must stay responsive.
                let options = options.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
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
                    // The task ends here, so a closed channel needs no branch.
                    let _ = send(
                        &tx,
                        0,
                        UpdateKind::Catalogue {
                            agents,
                            models,
                            skills,
                            schemes,
                        },
                    )
                    .await;
                });
            }
            Command::Tools => {
                if options.demo {
                    continue;
                }
                let options = options.clone();
                let tx = tx.clone();
                tokio::spawn(async move { fetch_tools(options, tx).await });
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
                let result = result.and_then(|reply| {
                    if let Some(text) = reply["agent_config"].as_str() {
                        serde_json::from_str::<Value>(text).map_err(|error| error.to_string())
                    } else {
                        Ok(reply["agent_config"].clone())
                    }
                }).map(|config| json!({"agent_id": agent, "agent_config": config}));
                if !send(&tx, 0, UpdateKind::ConfigSaved { session, key: "agent", result }).await {
                    return;
                }
            }
            Command::SetModelConfig { generation, session, configs } => {
                if options.demo {
                    continue;
                }
                let command = "update_workflow_model_config";
                let params = json!({"session_id": session, "configs": configs});
                let result = native::workflow_command(&options, command, params, Some(&idempotency_key(command))).await;
                if !send(&tx, generation, UpdateKind::ModelsSaved(result.map(|_| configs))).await { return; }
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
                let result = result.map(|_| json!({"sandboxOverride": true, "sandboxExecutionMode": mode, "sandboxSchemeId": scheme}));
                if !send(&tx, 0, UpdateKind::ConfigSaved { session, key: "sandbox", result }).await {
                    return;
                }
            }
            Command::ClearContext { generation, session } => {
                if options.demo { continue; }
                let command = "workflow_begin_new_context_frame";
                let result = native::workflow_command(&options, command, json!({"session_id":session}), Some(&idempotency_key(command))).await;
                if !send(&tx, generation, UpdateKind::ContextCleared(result)).await { return; }
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
                if !send(
                    &tx,
                    0,
                    UpdateKind::SettingsChanged {
                        session,
                        key: "autoCompress",
                        result: result.map(|_| ()),
                    },
                )
                .await
                {
                    return;
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
                if !send(
                    &tx,
                    0,
                    UpdateKind::SettingsChanged {
                        session,
                        key: "finalAudit",
                        result: result.map(|_| ()),
                    },
                )
                .await
                {
                    return;
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
                let result = result.map(|_| json!({"approvalLevel": level}));
                if !send(&tx, 0, UpdateKind::ConfigSaved { session, key: "approval", result }).await {
                    return;
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
                if !send(
                    &tx,
                    0,
                    UpdateKind::SettingsChanged {
                        session,
                        key: "phase",
                        result: result.map(|_| ()),
                    },
                )
                .await
                {
                    return;
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
                let result = result.map(|_| json!({"skillEnabled": enabled, "selectedSkills": selected}));
                if !send(&tx, 0, UpdateKind::ConfigSaved { session, key: "skills", result }).await {
                    return;
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
            Command::SearchFiles {
                request,
                paths,
                query,
            } => {
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
                if !send(&tx, 0, UpdateKind::Files { request, files }).await {
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
                            &json!({"session_id":id,"before_message_id":before,"limit":PAGE_LIMIT}),
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

/// Fetches the tool catalogue without holding up workflow commands.
///
/// `get_available_tools` may wait on MCP discovery, so it gets its own bounded
/// request and never runs on the command loop.
async fn fetch_tools(options: Options, tx: mpsc::Sender<Update>) {
    let tools = tokio::time::timeout(
        Duration::from_secs(8),
        native::data_command(&options, "get_available_tools", json!({})),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .and_then(|value| value.as_array().cloned())
    .unwrap_or_default();
    let _ = send(&tx, 0, UpdateKind::Tools(tools)).await;
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

/// Keep notification subscriptions alive when the transcript switches sessions.
async fn watch_decisions(
    options: Options,
    tx: mpsc::Sender<Update>,
    mut selected: watch::Receiver<Option<String>>,
    mut sessions: watch::Receiver<Vec<Session>>,
) {
    let mut tasks = BTreeMap::<String, tokio::task::JoinHandle<()>>::new();
    let mut known = BTreeSet::new();
    loop {
        let active = selected.borrow_and_update().clone();
        let list = sessions.borrow_and_update().clone();
        let listed = list
            .iter()
            .map(|session| session.id.clone())
            .collect::<BTreeSet<_>>();
        // A newly opened workflow may be selected before its first list refresh.
        let mut wanted = list
            .iter()
            .filter(|session| !chimes::terminal(&session.status))
            .map(|session| session.id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(id) = active {
            if listed.contains(&id) || !known.contains(&id) {
                wanted.insert(id);
            }
        }
        // Keep the last known IDs until a deleted selected workflow is replaced.
        known.retain(|id| listed.contains(id) || selected.borrow().as_ref() == Some(id));
        known.extend(listed);
        let removed = tasks
            .keys()
            .filter(|id| !wanted.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in removed {
            if let Some(task) = tasks.remove(&id) {
                task.abort();
                let _ = task.await;
            }
            if !send(
                &tx,
                0,
                UpdateKind::Decisions {
                    owner: id,
                    pending: Vec::new(),
                    reminder: None,
                },
            )
            .await
            {
                return;
            }
        }
        for id in wanted {
            tasks
                .entry(id.clone())
                .or_insert_with(|| tokio::spawn(follow_decisions(options.clone(), tx.clone(), id)));
        }
        tokio::select! {
            result = selected.changed() => if result.is_err() { return; },
            result = sessions.changed() => if result.is_err() { return; },
        }
    }
}

async fn follow_decisions(options: Options, tx: mpsc::Sender<Update>, id: String) {
    let mut cursor: Option<String> = None;
    let mut state = NotificationState::default();
    let mut published = Vec::new();
    loop {
        if let Some(client) = native::connect(&options).await {
            match client.stream_workflow(&id, cursor.as_deref()).await {
                Ok(mut stream) => {
                    // Subscribe before hydration. Process buffered events after the snapshot,
                    // never replace state with a snapshot after processing newer live events.
                    if cursor.is_none() {
                        if let Ok(snapshot) =
                            client.get(&format!("/control/v1/workflows/{id}")).await
                        {
                            let reminder = state.snapshot(&id, &snapshot);
                            let pending = state.pending();
                            if pending != published || reminder.is_some() {
                                published = pending.clone();
                                if !send(
                                    &tx,
                                    0,
                                    UpdateKind::Decisions {
                                        owner: id.clone(),
                                        pending,
                                        reminder,
                                    },
                                )
                                .await
                                {
                                    return;
                                }
                            }
                        }
                    }
                    loop {
                        match stream.next_event().await {
                            Ok(Some(event)) => {
                                if cursor
                                    .as_deref()
                                    .and_then(|value| value.rsplit_once(':'))
                                    .is_some_and(|(instance, sequence)| {
                                        instance == event.server_instance_id
                                            && sequence
                                                .parse::<u64>()
                                                .is_ok_and(|last| event.sequence <= last)
                                    })
                                {
                                    continue;
                                }
                                cursor =
                                    Some(chatspeed_runtime_client::stream_envelope_cursor(&event));
                                if !NotificationState::handles(&event.payload) {
                                    continue;
                                }
                                let reminder = state.event(&id, &event.payload);
                                let pending = state.pending();
                                if pending != published || reminder.is_some() {
                                    published = pending.clone();
                                    if !send(
                                        &tx,
                                        0,
                                        UpdateKind::Decisions {
                                            owner: id.clone(),
                                            pending,
                                            reminder,
                                        },
                                    )
                                    .await
                                    {
                                        return;
                                    }
                                }
                            }
                            Ok(None) => break,
                            Err(error) => {
                                if matches!(
                                    error,
                                    chatspeed_runtime_client::ClientError::Serialization(_)
                                ) {
                                    cursor = None;
                                }
                                break;
                            }
                        }
                    }
                }
                Err(_) => cursor = None,
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn snapshot(
    client: &RuntimeClient,
    id: &str,
    tx: &mpsc::Sender<Update>,
    generation: u64,
) -> bool {
    let fetched_from = Instant::now();
    let Ok(value) = client
        .get(&format!("/control/v1/workflows/{id}?limit={PAGE_LIMIT}"))
        .await
    else {
        return false;
    };
    let decode_start = Instant::now();
    let Ok(mut page) = serde_json::from_value::<Page>(value) else {
        return false;
    };
    page.fetched_from = Some(fetched_from);
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
                                    if !send(&tx, generation, UpdateKind::Event(envelope.payload)).await { return; }
                                }
                                Ok(None) => break,
                                Err(error) => {
                                    // The client decodes reset frames as Serialization, not a reset enum.
                                    // Treat every invalid envelope as a resync; never inspect error text.
                                    if matches!(error, chatspeed_runtime_client::ClientError::Serialization(_)) {
                                        cursor = None;
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
