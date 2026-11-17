//! Isolated Tokio thread. Only list, snapshot, history-query and authenticated SSE are used.
use crate::{model::Model, types::*};
use chatspeed_runtime_client::{load_discovery, RuntimeClient};
use serde_json::json;
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

async fn connect(options: &Options) -> Option<RuntimeClient> {
    let discovery = load_discovery(options.discovery.as_deref()).ok()?;
    RuntimeClient::connect(&discovery).await.ok()
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
                            })
                            .collect(),
                    ),
                )
                .await
                {
                    return;
                }
            } else if let Some(client) = connect(&list_options).await {
                if let Ok(value) = client.get("/control/v1/workflows").await {
                    if let Ok(sessions) = serde_json::from_value(value) {
                        if !send(&list_tx, 0, UpdateKind::Sessions(sessions)).await
                            || !send(&list_tx, 0, UpdateKind::Connected(true)).await
                        {
                            return;
                        }
                    }
                }
            } else if !send(&list_tx, 0, UpdateKind::Connected(false)).await {
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    let mut selection: Option<tokio::task::JoinHandle<()>> = None;
    while let Some(command) = rx.recv().await {
        match command {
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
                if let Some(client) = connect(&options).await {
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
    let mut needs_snapshot = true;
    loop {
        if !send(&tx, generation, UpdateKind::Reconnect).await {
            return;
        }
        if let Some(client) = connect(&options).await {
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
