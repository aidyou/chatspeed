//! Focused replay and cancellation checks; no daemon, database or display is required.
use chatspeed_gpui::{
    types::{Command, Options, UpdateKind},
    worker::Worker,
};
use std::time::{Duration, Instant};

#[test]
fn replay_is_ordered_and_window_drop_joins_worker() {
    let mut worker = Worker::start(Options {
        discovery: None,
        metrics: None,
        demo: true,
    })
    .unwrap();
    worker
        .commands
        .try_send(Command::Select {
            generation: 9,
            id: "demo".into(),
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    let mut snapshot = false;
    let mut event = false;
    while Instant::now() < until {
        if let Ok(update) = worker.updates.try_recv() {
            match update.kind {
                UpdateKind::Snapshot(page) => {
                    assert_eq!(update.generation, 9);
                    assert_eq!(page.messages.len(), 500);
                    snapshot = true;
                }
                UpdateKind::Event(_) => {
                    assert!(snapshot);
                    event = true;
                    break;
                }
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(snapshot && event);
    let start = Instant::now();
    drop(worker);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn child_reopen_tags_updates_without_replacing_parent() {
    let mut worker = Worker::start(Options { discovery: None, metrics: None, demo: true }).unwrap();
    worker.commands.try_send(Command::Select { generation: 9, id: "parent".into() }).unwrap();
    for epoch in 1..=2 {
        worker.set_child(Some((9, epoch, "child".into())));
        let until = Instant::now() + Duration::from_secs(3);
        let mut child_snapshot = false;
        let mut parent_event = false;
        while Instant::now() < until && !(child_snapshot && parent_event) {
            match worker.updates.try_recv().map(|update| update.kind) {
                Ok(UpdateKind::Child(current, id, kind)) if current == epoch => {
                    assert_eq!(id, "child");
                    child_snapshot |= matches!(*kind, UpdateKind::Snapshot(_));
                }
                Ok(UpdateKind::Event(_)) => parent_event = true,
                Ok(_) => {},
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        assert!(child_snapshot && parent_event);
        worker.set_child(None);
    }
}

#[test]
fn immediate_shutdown_cancels_selected_worker() {
    let worker = Worker::start(Options {
        discovery: None,
        metrics: None,
        demo: true,
    })
    .unwrap();
    worker
        .commands
        .try_send(Command::Select {
            generation: 1,
            id: "demo".into(),
        })
        .unwrap();
    let start = Instant::now();
    drop(worker);
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn demo_switching_and_older_pages_stay_local() {
    let mut worker = Worker::start(Options {
        discovery: None,
        metrics: None,
        demo: true,
    })
    .unwrap();
    let mut bodies = Vec::new();
    for generation in 1..=3 {
        worker
            .commands
            .try_send(Command::Select {
                generation,
                id: format!("demo-{generation}"),
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut before = None;
        while Instant::now() < deadline {
            if let Ok(update) = worker.updates.try_recv() {
                if update.generation == generation {
                    if let UpdateKind::Snapshot(page) = update.kind {
                        bodies.push(page.messages[0].message.clone());
                        before = page.message_window_before_id;
                        break;
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        worker
            .commands
            .try_send(Command::Older {
                generation,
                id: format!("demo-{generation}"),
                before: before.expect("demo has older history"),
            })
            .unwrap();
        let mut older = false;
        while Instant::now() < deadline {
            if let Ok(update) = worker.updates.try_recv() {
                if update.generation == generation {
                    if let UpdateKind::Older(page) = update.kind {
                        assert_eq!(page.messages[0].id, 1);
                        assert_eq!(page.messages.last().unwrap().id, 400);
                        older = true;
                        break;
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert!(older);
    }
    assert_eq!(bodies.len(), 3);
    assert!(bodies[0] != bodies[1] && bodies[1] != bodies[2]);
}

#[test]
fn loopback_reconnect_reset_and_snapshot_race_converge() {
    use chatspeed_gpui::model::Model;
    use serde_json::json;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let discovery = std::env::temp_dir().join(format!(
        "observer-discovery-{}-{port}.json",
        std::process::id()
    ));
    std::fs::write(&discovery, json!({"protocol_version":"1","server_instance_id":"mock","pid":std::process::id(),"host":"127.0.0.1","port":port,"token":"fixture","started_at":"fixture"}).to_string()).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let done_server = done.clone();
    let (saved_tx, saved_rx) = std::sync::mpsc::channel();
    let (signal_tx, signal_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut held_streams = Vec::new();
        let mut streams = 0;
        let mut snapshots = 0;
        while !done_server.load(Ordering::Relaxed) {
            let Ok((mut socket, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            while let Ok(n) = socket.read(&mut buf) {
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request
                .to_lowercase()
                .contains("authorization: bearer fixture"));
            let path = request.split_whitespace().nth(1).unwrap();
            let frame = |seq, payload| {
                format!(
                    "data: {}\n\n",
                    json!({"schema_version":1,"server_instance_id":"mock","sequence":seq,"session_id":"subagent_transcript","payload":payload})
                )
            };
            let (content_type, body) = if path == "/control/v1/meta" {
                ("application/json", json!({"service":"chatspeed-runtime","protocol_version":"1","schema_version":1,"server_instance_id":"mock","pid":std::process::id()}).to_string())
            } else if path == "/control/v1/data-commands/get_all_config" {
                // A stalled settings response must not hold up the first sidebar list.
                held_streams.push(socket);
                continue;
            } else if path.ends_with("/signal") {
                signal_tx.send(path.to_owned()).unwrap();
                ("application/json", "{}".to_owned())
            } else if path == "/control/v1/data-commands/set_config" {
                saved_tx.send(()).unwrap();
                ("application/json", "null".to_owned())
            } else if path == "/control/v1/workflows" {
                (
                    "application/json",
                    json!([{"id":"background","title":"Background","status":"awaiting_user"}])
                        .to_string(),
                )
            } else if path == "/control/v1/workflows/background/stream" {
                let body = "data: {\"schema_version\":1,\"server_instance_id\":\"mock\",\"sequence\":1,\"session_id\":\"background\",\"payload\":{\"type\":\"sub_agent_progress\",\"sub_agent_id\":\"subagent_question\",\"wait_reason\":\"user_input\"}}\n\n";
                let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n", body.len()).as_bytes());
                held_streams.push(socket);
                continue;
            } else if path == "/control/v1/workflows/background" {
                (
                    "application/json",
                    json!({"workflow":{"status":"awaiting_user"}}).to_string(),
                )
            } else if path.ends_with("/stream") {
                streams += 1;
                let body = match streams {
                    1 => {
                        assert!(!request.to_lowercase().contains("last-event-id"));
                        frame(1, json!({"type":"chunk","content":"part"}))
                    }
                    2 => {
                        assert_eq!(snapshots, 1, "ordinary reconnect must not snapshot");
                        assert!(request.to_lowercase().contains("last-event-id: mock:1"));
                        format!(
                            "{}{}{}",
                            frame(1, json!({"type":"chunk","content":"part"})),
                            frame(2, json!({"type":"chunk","content":"ial"})),
                            "event: reset\ndata: {\"error\":{\"code\":\"reset_required\"}}\n\n"
                        )
                    }
                    _ => {
                        assert!(snapshots >= 2);
                        assert!(!request.to_lowercase().contains("last-event-id"));
                        // A final was durable before subscription. Its stale delta must
                        // disappear at periodic authoritative terminal reconciliation.
                        frame(3, json!({"type":"chunk","content":"stale"}))
                    }
                };
                ("text/event-stream", body)
            } else if path == "/control/v1/workflows/subagent_transcript?limit=500" {
                snapshots += 1;
                ("application/json", if snapshots == 1 { json!({"messages":[],"status":"executing"}) } else { json!({"messages":[{"id":7,"role":"assistant","message":"durable final"}],"workflow":{"status":"completed"}}) }.to_string())
            } else {
                ("application/json", "[]".to_string())
            };
            if streams >= 3 && content_type == "text/event-stream" {
                let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n", body.len()).as_bytes());
                // Keep SSE open while allowing the worker's periodic HTTP snapshot.
                held_streams.push(socket);
            } else {
                let _ = socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            }
        }
    });
    let mut worker = Worker::start(Options {
        discovery: Some(discovery.clone()),
        metrics: None,
        demo: false,
    })
    .unwrap();
    let mut background_question = false;
    let mut child_question = false;
    let list_deadline = Instant::now() + Duration::from_secs(4);
    let mut listed = false;
    while Instant::now() < list_deadline {
        if let Ok(update) = worker.updates.try_recv() {
            match update.kind {
                UpdateKind::Sessions(sessions) => {
                    listed = sessions.iter().any(|session| session.id == "background");
                    break;
                }
                UpdateKind::Decisions { owner, pending, .. } if owner == "background" => {
                    background_question |= pending.iter().any(|entry| entry.target == "background");
                    child_question |= pending
                        .iter()
                        .any(|entry| entry.target == "subagent_question");
                }
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let mut model = Model::default();
    let generation = model.select();
    worker
        .commands
        .try_send(Command::Select {
            generation,
            id: "subagent_transcript".into(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(18);
    let mut reset = false;
    let mut partial = false;
    let mut converged = false;
    let mut pages = 0;
    while Instant::now() < deadline {
        if let Ok(update) = worker.updates.try_recv() {
            if update.generation != generation {
                if let UpdateKind::Decisions { owner, pending, .. } = update.kind {
                    if owner == "background" {
                        background_question |=
                            pending.iter().any(|entry| entry.target == "background");
                        child_question |= pending
                            .iter()
                            .any(|entry| entry.target == "subagent_question");
                    }
                }
                continue;
            }
            match update.kind {
                UpdateKind::Snapshot(page) => {
                    pages += 1;
                    model.page(page, false);
                    if pages >= 3
                        && model.live.message.is_empty()
                        && model
                            .messages
                            .get(&7)
                            .is_some_and(|m| m.message == "durable final")
                    {
                        converged = true;
                        break;
                    }
                }
                UpdateKind::Reset => {
                    reset = true;
                    model.reset_live();
                    assert!(model.live.message.is_empty());
                }
                UpdateKind::Event(event) => {
                    model.event(event);
                    if model.live.message == "partial" {
                        partial = true;
                    }
                }
                _ => {}
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    worker.set_child(Some((generation, 1, "subagent_question".into())));
    for signal in [
        json!({"type":"approval", "id":"shared-call", "approved":true, "approve_all":false}),
        json!({"type":"user_message", "content":"answer", "metadata":{"ask_user_response":true,"requested_tool_call_id":"shared-call"}}),
    ] {
        worker.commands.try_send(Command::Signal { id: "subagent_question".into(), signal }).unwrap();
    }
    let captured_signals: Vec<_> = (0..2)
        .map(|_| signal_rx.recv_timeout(Duration::from_secs(4)))
        .collect();
    worker.set_child(None);
    assert!(worker.save_sidebar(false));
    assert!(worker.save_sidebar(true));
    worker.save_geometry(vec![("workflow_window_size", json!({"width": 1100, "height": 700}))]);
    worker.save_geometry(vec![("workflow_window_size", json!({"width": 1200, "height": 800}))]);
    drop(worker);
    let saves_on_close = saved_rx.try_iter().count();
    done.store(true, Ordering::Relaxed);
    server.join().unwrap();
    std::fs::remove_file(discovery).unwrap();
    assert_eq!(captured_signals, vec![Ok("/control/v1/workflows/subagent_question/signal".to_owned()); 2]);
    assert_eq!(saves_on_close, 2, "shutdown must drain one sidebar and one coalesced geometry save");
    assert!(
        listed,
        "sidebar list must arrive while settings are stalled"
    );
    assert!(background_question && child_question, "existing background questions and live child questions must both appear outside the selected transcript");
    assert!(partial, "resume must not duplicate partial delta");
    assert!(reset, "client reset must clear transient state");
    assert!(
        converged,
        "periodic snapshot must clear post-snapshot stale delta"
    );
}
