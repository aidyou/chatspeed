//! `cs` — ChatSpeed workflow CLI.
//!
//! A pure HTTP/SSE client of the local loopback control plane. It never opens
//! the database, never starts a workflow runtime or executor and never runs an
//! input loop. Phase 2F campaign commands additionally link the shared,
//! dependency-free `chatspeed_lib::campaign` contract (strict parsers and
//! canonical hashes only) so validation cannot drift from the backend's.

// Shared i18n catalogs are embedded into this binary at crate root so human
// output can be localized without linking any workflow runtime behavior.
rust_i18n::i18n!("i18n", fallback = "en");

#[path = "cs/args.rs"]
mod args;
#[path = "cs/artifact.rs"]
mod artifact;
#[path = "cs/benchmark.rs"]
mod benchmark;
#[path = "cs/campaign.rs"]
mod campaign;
#[path = "cs/capability.rs"]
mod capability;
#[path = "cs/client.rs"]
mod client;
#[path = "cs/discovery.rs"]
mod discovery;
#[path = "cs/error.rs"]
mod error;
#[path = "cs/evaluate.rs"]
mod evaluate;
#[path = "cs/experiment.rs"]
mod experiment;
#[path = "cs/mcp.rs"]
mod mcp;
#[path = "cs/output.rs"]
mod output;
#[path = "cs/promotion.rs"]
mod promotion;
#[path = "cs/schedule.rs"]
mod schedule;
#[path = "cs/skill.rs"]
mod skill;
#[path = "cs/sse.rs"]
mod sse;
#[path = "cs/verifier.rs"]
mod verifier;

use args::{
    AgentCommand, BenchmarkCommand, CampaignCommand, Cli, Command, DoctorCommand, ExperimentCommand,
    OutputFormat, PromotionCommand, WorkflowCommand,
};
use clap::Parser as _;
use client::ControlPlaneClient;
use discovery::ControlPlaneDiscovery;
use error::CliError;
use futures_util::StreamExt;
use output::{eprint_diagnostic, print_json, print_jsonl, render_result};
use serde_json::{json, Value};
use sse::{SseFrame, SseParser};
use std::io::Write;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Some(lang) = &cli.lang {
        rust_i18n::set_locale(lang);
    }

    let exit_code = match run(&cli).await {
        Ok(()) => 0,
        Err(error) => {
            eprint_diagnostic(&format!("cs: {}", error));
            error.exit_code()
        }
    };
    std::process::exit(exit_code);
}

async fn run(cli: &Cli) -> Result<(), CliError> {
    // Offline experiment commands run before discovery loading so they work
    // without a running main process, discovery file, database or network.
    if let Command::Experiment { command } = &cli.command {
        match command {
            ExperimentCommand::Inspect { artifact_dir } => {
                return experiment::inspect(cli, artifact_dir)
            }
            ExperimentCommand::Replay { artifact_dir } => {
                return experiment::replay(cli, artifact_dir)
            }
            ExperimentCommand::Evaluate {
                artifact_dir,
                evaluation_dir,
            } => return evaluate::evaluate(cli, artifact_dir, evaluation_dir),
            ExperimentCommand::Capture { .. } | ExperimentCommand::Run { .. } => {}
            // Benchmark verify is offline; benchmark run needs the client.
            ExperimentCommand::Benchmark {
                command:
                    BenchmarkCommand::Verify {
                        suite,
                        task,
                        artifact_dir,
                        verdict_dir,
                    },
            } => return verifier::verify(cli, suite, task, artifact_dir, verdict_dir),
            ExperimentCommand::Benchmark { .. } => {}
            // Campaign inspect is offline; create/run/close need the client.
            ExperimentCommand::Campaign {
                command: CampaignCommand::Inspect { out },
            } => return campaign::inspect(cli, out),
            ExperimentCommand::Campaign { .. } => {}
            // Promotion inspect is offline; submit/status/reconcile/audit need
            // the client.
            ExperimentCommand::Promotion {
                command: PromotionCommand::Inspect { out },
            } => return promotion::inspect(cli, out),
            ExperimentCommand::Promotion { .. } => {}
        }
    }

    let discovery = discovery::load_discovery(cli.discovery_file.as_deref())?;
    let client = ControlPlaneClient::new(&discovery)?;

    match &cli.command {
        Command::Doctor { command } => match command {
            // Bare `cs doctor` keeps its exact connectivity/identity check.
            None => doctor(cli, &discovery, &client).await,
            Some(DoctorCommand::Capabilities) => {
                capability::doctor_capabilities(cli, &client).await
            }
            Some(DoctorCommand::Reconcile { idempotency_key }) => {
                capability::doctor_reconcile(cli, &client, idempotency_key).await
            }
        },
        Command::Skill { command } => skill::run(cli, &client, command).await,
        Command::Mcp { command } => mcp::run(cli, &client, command).await,
        Command::Agent { command } => run_agent_command(cli, &client, command).await,
        Command::Workflow { command } => run_workflow_command(cli, &client, command).await,
        Command::Experiment { command } => match command {
            ExperimentCommand::Capture {
                session_id,
                artifact_dir,
            } => experiment::capture(cli, &client, session_id, artifact_dir).await,
            ExperimentCommand::Run {
                agent,
                spec,
                prompt,
                prompt_file,
                follow,
                artifact_dir,
            } => {
                let prompt = args::resolve_prompt(prompt, prompt_file).map_err(CliError::usage)?;
                experiment::run(
                    cli,
                    &client,
                    agent,
                    spec,
                    prompt,
                    *follow,
                    artifact_dir.as_deref(),
                )
                .await
            }
            // Inspect/replay/evaluate are handled above before discovery loading.
            ExperimentCommand::Inspect { .. }
            | ExperimentCommand::Replay { .. }
            | ExperimentCommand::Evaluate { .. } => Ok(()),
            ExperimentCommand::Benchmark { command } => match command {
                BenchmarkCommand::Run {
                    suite,
                    task,
                    agent,
                    model,
                    artifact_dir,
                } => {
                    benchmark::run(
                        cli,
                        &client,
                        suite,
                        task,
                        agent,
                        model.as_deref(),
                        artifact_dir.as_deref(),
                    )
                    .await
                }
                // Verify is handled above before discovery loading.
                BenchmarkCommand::Verify { .. } => Ok(()),
            },
            ExperimentCommand::Campaign { command } => match command {
                CampaignCommand::Inspect { .. } => Ok(()),
                CampaignCommand::Create { plan, out } => {
                    campaign::create(cli, &client, plan, out).await
                }
                CampaignCommand::Run { out, candidate } => {
                    campaign::run(cli, &client, out, candidate).await
                }
                CampaignCommand::Close { out, reason } => {
                    campaign::close(cli, &client, out, reason.as_deref().unwrap_or("")).await
                }
                CampaignCommand::Schedule {
                    plan,
                    profile,
                    bundle_ref,
                } => schedule::schedule(cli, &client, plan, profile, bundle_ref.clone()).await,
                CampaignCommand::Jobs { campaign_id } => {
                    schedule::jobs(cli, &client, campaign_id).await
                }
                CampaignCommand::Job { job_id } => schedule::job(cli, &client, job_id).await,
                CampaignCommand::Cancel {
                    campaign_id,
                    reason,
                } => {
                    schedule::cancel(cli, &client, campaign_id, reason.as_deref().unwrap_or(""))
                        .await
                }
                CampaignCommand::Reconcile { campaign_id } => {
                    schedule::reconcile(cli, &client, campaign_id).await
                }
            },
            ExperimentCommand::Promotion { command } => match command {
                // Inspect is handled above before discovery loading.
                PromotionCommand::Inspect { .. } => Ok(()),
                PromotionCommand::Run { evidence, out } => {
                    promotion::run(cli, &client, evidence, out).await
                }
                PromotionCommand::Status { promotion_id } => {
                    promotion::status(cli, &client, promotion_id).await
                }
                PromotionCommand::Reconcile { promotion_id } => {
                    promotion::reconcile(cli, &client, promotion_id).await
                }
                PromotionCommand::Audit { promotion_id, out } => {
                    promotion::audit(cli, &client, promotion_id, out).await
                }
            },
        },
    }
}

async fn doctor(
    cli: &Cli,
    discovery: &ControlPlaneDiscovery,
    client: &ControlPlaneClient,
) -> Result<(), CliError> {
    let meta = client.get("/control/v1/meta").await?;

    // Instance binding: the discovery document must belong to the running
    // control plane, otherwise it is stale and must not be trusted.
    if discovery.server_instance_id != meta["server_instance_id"].as_str().unwrap_or("") {
        return Err(CliError::protocol(
            "Discovery document does not match the running control plane instance; it may be stale",
        ));
    }
    if !process_alive(discovery.pid) {
        eprint_diagnostic(
            "cs: warning: the process that published the discovery document is not running; the document may be stale",
        );
    }

    let endpoint = format!("http://{}:{}", discovery.host, discovery.port);
    match cli.output {
        OutputFormat::Human => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let _ = writeln!(
                handle,
                "{}",
                rust_i18n::t!(
                    "cs.doctor_connected",
                    endpoint = endpoint,
                    protocol = meta["protocol_version"].as_str().unwrap_or("?"),
                    instance = meta["server_instance_id"].as_str().unwrap_or("?"),
                    pid = meta["pid"].as_u64().unwrap_or(0)
                )
            );
            let _ = writeln!(handle, "{}", rust_i18n::t!("cs.doctor_ok"));
        }
        OutputFormat::Json => print_json(&meta),
        OutputFormat::Jsonl => print_jsonl(&meta),
    }
    Ok(())
}

async fn run_agent_command(
    cli: &Cli,
    client: &ControlPlaneClient,
    command: &AgentCommand,
) -> Result<(), CliError> {
    match command {
        AgentCommand::List => {
            let agents = client.get("/control/v1/agents").await?;
            render_result(cli.output, &agents);
            Ok(())
        }
        AgentCommand::Get { agent_id } => {
            let agent = client
                .get(&format!("/control/v1/agents/{}", agent_id))
                .await?;
            render_result(cli.output, &agent);
            Ok(())
        }
    }
}

async fn run_workflow_command(
    cli: &Cli,
    client: &ControlPlaneClient,
    command: &WorkflowCommand,
) -> Result<(), CliError> {
    match command {
        WorkflowCommand::List => {
            let workflows = client.get("/control/v1/workflows").await?;
            render_result(cli.output, &workflows);
            Ok(())
        }
        WorkflowCommand::Create {
            agent,
            prompt,
            prompt_file,
            allowed_paths,
            model,
            agent_config,
            final_audit,
        } => {
            let prompt = args::resolve_prompt(prompt, prompt_file).map_err(CliError::usage)?;
            let body = json!({
                "agent_id": agent,
                "user_query": prompt,
                "allowed_paths": allowed_paths_value(allowed_paths),
                "auto_approve_plan": Value::Null,
                "final_audit": final_audit.then_some(true),
                "inherited_agent_config": inherited_agent_config_value(model, agent_config)?,
            });
            let result = client.post("/control/v1/workflows", body, None).await?;
            report_session(cli, &result, "cs.workflow_created")
        }
        WorkflowCommand::Start {
            session_id,
            prompt,
            prompt_file,
            plan,
            follow,
        } => {
            let prompt = args::resolve_prompt(prompt, prompt_file).map_err(CliError::usage)?;
            start_workflow(
                cli,
                client,
                session_id,
                None,
                prompt,
                (*plan).then_some(true),
                *follow,
            )
            .await
        }
        WorkflowCommand::Run {
            agent,
            prompt,
            prompt_file,
            allowed_paths,
            model,
            agent_config,
            final_audit,
            plan,
            follow,
        } => {
            let prompt = args::resolve_prompt(prompt, prompt_file).map_err(CliError::usage)?;
            let create_body = json!({
                "agent_id": agent,
                "user_query": prompt,
                "allowed_paths": allowed_paths_value(allowed_paths),
                "auto_approve_plan": Value::Null,
                "final_audit": final_audit.then_some(true),
                "inherited_agent_config": inherited_agent_config_value(model, agent_config)?,
            });
            // Sequential create -> start; on start failure the created
            // session_id is reported and the workflow is kept (not deleted).
            // The resolved prompt is passed again as initial_prompt so the
            // runtime appends exactly one initial user message (same contract
            // as the Tauri create -> start sequence).
            let created = client
                .post("/control/v1/workflows", create_body, None)
                .await?;
            let session_id = created["session_id"]
                .as_str()
                .ok_or_else(|| CliError::protocol("Create response missing session_id"))?
                .to_string();
            match start_workflow(
                cli,
                client,
                &session_id,
                Some(agent.clone()),
                prompt,
                (*plan).then_some(true),
                *follow,
            )
            .await
            {
                Ok(()) => Ok(()),
                Err(error) => {
                    eprint_diagnostic(&format!(
                        "cs: start failed for created session {}: {}",
                        session_id, error
                    ));
                    Err(error)
                }
            }
        }
        WorkflowCommand::Get { session_id } => {
            let snapshot = client
                .get(&format!("/control/v1/workflows/{}", session_id))
                .await?;
            render_result(cli.output, &snapshot);
            Ok(())
        }
        WorkflowCommand::Events {
            session_id,
            after,
            follow,
        } => {
            let mut path = format!("/control/v1/workflows/{}/events", session_id);
            if let Some(after) = after {
                path.push_str(&format!("?after={}", after));
            }
            let events = client.get(&path).await?;
            render_result(cli.output, &events);
            if *follow {
                follow_events(cli, client, session_id, None).await?;
            }
            Ok(())
        }
        WorkflowCommand::Signal {
            session_id,
            json: signal_json,
            file,
        } => {
            let signal =
                args::validate_signal_source(signal_json, file).map_err(CliError::usage)?;
            submit_signal(cli, client, session_id, &signal).await
        }
        WorkflowCommand::Message {
            session_id,
            text,
            file,
        } => {
            let text = match (text, file) {
                (Some(text), _) => text.clone(),
                (None, Some(path)) => {
                    if path == &std::path::PathBuf::from("-") {
                        use std::io::Read;
                        let mut buffer = String::new();
                        std::io::stdin()
                            .read_to_string(&mut buffer)
                            .map_err(|e| CliError::io(e.to_string()))?;
                        buffer
                    } else {
                        std::fs::read_to_string(path).map_err(|e| {
                            CliError::io(format!("Failed to read {}: {}", path.display(), e))
                        })?
                    }
                }
                (None, None) => return Err(CliError::usage("Either --text or --file is required")),
            };
            let signal = json!({ "type": "user_message", "content": text });
            submit_signal(cli, client, session_id, &signal.to_string()).await
        }
        WorkflowCommand::Approve {
            session_id,
            tool_call_id,
            all,
        } => {
            let signal = json!({
                "type": "approval",
                "id": tool_call_id.clone().unwrap_or_default(),
                "approved": true,
                "approve_all": all,
            });
            submit_signal(cli, client, session_id, &signal.to_string()).await
        }
        WorkflowCommand::Reject {
            session_id,
            tool_call_id,
            all,
            message,
        } => {
            let signal = json!({
                "type": "approval",
                "id": tool_call_id.clone().unwrap_or_default(),
                "approved": false,
                "approve_all": all,
                "rejection_message": message,
            });
            submit_signal(cli, client, session_id, &signal.to_string()).await
        }
        WorkflowCommand::Continue { session_id } => {
            let signal = json!({ "type": "continue" });
            submit_signal(cli, client, session_id, &signal.to_string()).await
        }
        WorkflowCommand::Stop { session_id } => {
            let result = client
                .post(
                    &format!("/control/v1/workflows/{}/stop", session_id),
                    json!({}),
                    Some(&new_idempotency_key()),
                )
                .await?;
            report_session(cli, &result, "cs.workflow_stopped")
        }
    }
}

fn allowed_paths_value(allowed_paths: &[String]) -> Value {
    if allowed_paths.is_empty() {
        Value::Null
    } else {
        json!(allowed_paths)
    }
}

/// Builds the `inherited_agent_config` create-request value.
///
/// The field is a JSON *string* containing a camelCase AgentConfig document
/// (the Tauri frontend sends `JSON.stringify(config)`). `--model GROUP@MODEL`
/// overrides the act-phase model; provider id 0 targets the built-in `cs`
/// proxy group. `--agent-config` passes a raw AgentConfig JSON string.
fn inherited_agent_config_value(
    model: &Option<String>,
    agent_config: &Option<String>,
) -> Result<Value, CliError> {
    match (model, agent_config) {
        (None, None) => Ok(Value::Null),
        (Some(model), None) => {
            if model.split('@').count() != 2 || model.starts_with('@') || model.ends_with('@') {
                return Err(CliError::usage(format!(
                    "Invalid --model '{}': expected the form group@model (e.g. cs@free:ds-v4-flash)",
                    model
                )));
            }
            Ok(Value::String(
                json!({
                    "models": {
                        "act": { "id": 0, "model": model }
                    }
                })
                .to_string(),
            ))
        }
        (None, Some(agent_config)) => {
            // Validate it parses as JSON before sending.
            serde_json::from_str::<Value>(agent_config)
                .map_err(|e| CliError::usage(format!("--agent-config is not valid JSON: {}", e)))?;
            Ok(Value::String(agent_config.clone()))
        }
        // clap enforces the conflict; this arm only satisfies the type checker.
        (Some(_), Some(_)) => Err(CliError::usage(
            "--model and --agent-config are mutually exclusive".to_string(),
        )),
    }
}

async fn start_workflow(
    cli: &Cli,
    client: &ControlPlaneClient,
    session_id: &str,
    agent_id: Option<String>,
    prompt: String,
    planning_mode: Option<bool>,
    follow: bool,
) -> Result<(), CliError> {
    // The runtime loads the Agent config by agent_id and appends the initial
    // user message only when `initial_prompt` is present (same contract as the
    // Tauri workflow_start command). Resolve missing pieces from the
    // authoritative snapshot: agent_id when not supplied, and the stored
    // user_query when the caller did not pass --prompt.
    let snapshot = if agent_id.is_none() || prompt.is_empty() {
        Some(
            client
                .get(&format!("/control/v1/workflows/{}", session_id))
                .await?,
        )
    } else {
        None
    };
    let (agent_id, initial_prompt) = resolve_start_inputs(snapshot.as_ref(), agent_id, prompt)?;
    let body = json!({
        "session_id": session_id,
        "agent_id": agent_id,
        "initial_prompt": initial_prompt,
        "initial_metadata": Value::Null,
        "initial_attached_context": Value::Null,
        "planning_mode": planning_mode,
    });
    let result = client
        .post(
            &format!("/control/v1/workflows/{}/start", session_id),
            body,
            Some(&new_idempotency_key()),
        )
        .await?;
    report_session(cli, &result, "cs.workflow_started")?;
    if follow {
        follow_events(cli, client, session_id, None).await?;
    }
    Ok(())
}

async fn submit_signal(
    cli: &Cli,
    client: &ControlPlaneClient,
    session_id: &str,
    signal: &str,
) -> Result<(), CliError> {
    // Validate the signal is well-formed JSON before sending.
    let parsed: Value = serde_json::from_str(signal)
        .map_err(|e| CliError::usage(format!("Signal is not valid JSON: {}", e)))?;
    let result = client
        .post(
            &format!("/control/v1/workflows/{}/signal", session_id),
            parsed,
            Some(&new_idempotency_key()),
        )
        .await?;
    report_session(cli, &result, "cs.workflow_signal_sent")
}

fn report_session(cli: &Cli, result: &Value, human_key: &str) -> Result<(), CliError> {
    match cli.output {
        OutputFormat::Human => {
            let session_id = result["session_id"].as_str().unwrap_or("?");
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let _ = writeln!(
                handle,
                "{}",
                rust_i18n::t!(human_key, session_id = session_id)
            );
        }
        OutputFormat::Json => print_json(result),
        OutputFormat::Jsonl => print_jsonl(result),
    }
    Ok(())
}

fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Resolved start inputs: the agent to load and the initial user message.
///
/// The initial prompt must be present whenever the workflow has a task so the
/// runtime appends exactly one initial user message (AC-4 parity with the
/// Tauri create -> start sequence).
fn resolve_start_inputs(
    snapshot: Option<&Value>,
    agent_id: Option<String>,
    prompt: String,
) -> Result<(String, Option<String>), CliError> {
    let needs_snapshot = agent_id.is_none() || prompt.is_empty();
    let snapshot = match (needs_snapshot, snapshot) {
        (false, _) => None,
        (true, Some(value)) => Some(value),
        (true, None) => {
            return Err(CliError::protocol(
                "Internal error: start requires a snapshot but none was fetched",
            ))
        }
    };

    let agent_id = match agent_id {
        Some(agent_id) => agent_id,
        None => snapshot
            .and_then(|snapshot| snapshot["workflow"]["agent_id"].as_str())
            .ok_or_else(|| CliError::protocol("Snapshot is missing workflow.agent_id"))?
            .to_string(),
    };

    // Explicit --prompt wins; otherwise fall back to the stored user_query so
    // `create` + `start` launches the created task. An empty stored query
    // (e.g. resuming a waiting session) starts without injecting a message.
    let initial_prompt = if !prompt.is_empty() {
        Some(prompt)
    } else {
        snapshot
            .and_then(|snapshot| snapshot["workflow"]["user_query"].as_str())
            .filter(|query| !query.is_empty())
            .map(str::to_string)
    };

    Ok((agent_id, initial_prompt))
}

/// Best-effort liveness check for the process that published discovery.
fn process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::path::Path::new(&format!("/proc/{}", pid)).exists()
    }
    #[cfg(not(unix))]
    {
        // No portable liveness probe; assume alive and rely on request failures.
        let _ = pid;
        true
    }
}

/// Follows live SSE events for a session.
///
/// Disconnects and Ctrl-C only detach the follow loop; they never stop the
/// workflow. On `reset_required` the CLI converges by fetching the snapshot
/// plus durable events, then re-subscribes without a cursor.
async fn follow_events(
    cli: &Cli,
    client: &ControlPlaneClient,
    session_id: &str,
    mut last_event_id: Option<String>,
) -> Result<(), CliError> {
    // One transport-level reconnect is allowed per follow session; further
    // failures detach the follow loop without stopping the workflow.
    let mut reconnect_used = false;
    loop {
        let response = match client
            .stream(
                &format!("/control/v1/workflows/{}/stream", session_id),
                last_event_id.as_deref(),
            )
            .await
        {
            Ok(response) => response,
            Err(error @ (CliError::Auth(_) | CliError::Protocol(_))) => return Err(error),
            Err(error) => {
                if reconnect_used {
                    return Err(error);
                }
                reconnect_used = true;
                eprint_diagnostic(&format!(
                    "cs: stream disconnected ({}); reconnecting",
                    error
                ));
                continue;
            }
        };

        let mut stream = response.bytes_stream();
        let mut parser = SseParser::new();
        let mut needs_reset = false;
        let mut interrupted = false;

        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => {
                    eprint_diagnostic(&format!("cs: stream interrupted ({})", error));
                    interrupted = true;
                    break;
                }
            };
            let text = String::from_utf8_lossy(&bytes);
            for frame in parser.push(&text) {
                match classify_frame(&frame) {
                    FrameKind::Reset => {
                        needs_reset = true;
                        break;
                    }
                    FrameKind::Terminal => return Ok(()),
                    FrameKind::Event => {
                        if let Some(cursor) = &frame.id {
                            last_event_id = Some(cursor.clone());
                        }
                        emit_event(cli, &frame)?;
                    }
                }
            }
            if needs_reset {
                break;
            }
        }

        if interrupted {
            if reconnect_used {
                eprint_diagnostic(&rust_i18n::t!("cs.workflow_follow_detached"));
                return Ok(());
            }
            reconnect_used = true;
            continue;
        }

        if needs_reset {
            // Converge: snapshot + durable events, then re-subscribe fresh.
            eprint_diagnostic("cs: stream reset; converging via snapshot and durable events");
            let snapshot = client
                .get(&format!("/control/v1/workflows/{}", session_id))
                .await?;
            emit_event(
                cli,
                &SseFrame {
                    id: None,
                    event: Some("snapshot".to_string()),
                    data: snapshot.to_string(),
                },
            )?;
            let events = client
                .get(&format!("/control/v1/workflows/{}/events", session_id))
                .await?;
            emit_event(
                cli,
                &SseFrame {
                    id: None,
                    event: Some("durable_events".to_string()),
                    data: events.to_string(),
                },
            )?;
            last_event_id = None;
            continue;
        }

        // Stream ended without reset (server cleanup): stop following.
        eprint_diagnostic(&rust_i18n::t!("cs.workflow_follow_detached"));
        return Ok(());
    }
}

enum FrameKind {
    Event,
    Reset,
    Terminal,
}

/// Classifies a frame: reset events, structured terminal events, or normal
/// events. Terminal detection uses structured payloads only (INV-3).
fn classify_frame(frame: &SseFrame) -> FrameKind {
    if frame.event.as_deref() == Some("reset_required") {
        return FrameKind::Reset;
    }
    let payload: Value = serde_json::from_str(&frame.data).unwrap_or(Value::Null);
    let payload_type = payload["payload"]["type"].as_str().unwrap_or("");
    if payload_type == "task_completed" {
        return FrameKind::Terminal;
    }
    if payload_type == "state" {
        let state = payload["payload"]["state"].as_str().unwrap_or("");
        if matches!(state, "completed" | "error" | "cancelled") {
            return FrameKind::Terminal;
        }
    }
    FrameKind::Event
}

fn emit_event(cli: &Cli, frame: &SseFrame) -> Result<(), CliError> {
    let payload: Value = serde_json::from_str(&frame.data).unwrap_or(json!({ "raw": frame.data }));
    match cli.output {
        OutputFormat::Jsonl => print_jsonl(&payload),
        OutputFormat::Json => print_json(&payload),
        OutputFormat::Human => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let _ = writeln!(handle, "{}", payload);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(agent_id: &str, user_query: &str) -> Value {
        json!({
            "workflow": {
                "agent_id": agent_id,
                "user_query": user_query,
            }
        })
    }

    #[test]
    fn run_passes_explicit_prompt_as_initial_prompt() {
        // `workflow run` knows the agent and the resolved prompt; no snapshot
        // is fetched and the prompt must reach the runtime as initial_prompt.
        let (agent_id, initial_prompt) =
            resolve_start_inputs(None, Some("agent-1".into()), "do the task".into()).unwrap();
        assert_eq!(agent_id, "agent-1");
        assert_eq!(initial_prompt.as_deref(), Some("do the task"));
    }

    #[test]
    fn start_without_prompt_uses_stored_user_query() {
        // `create --prompt X` + `start` must launch the created task: the
        // stored user_query becomes the initial user message.
        let snapshot = snapshot("agent-1", "stored task");
        let (agent_id, initial_prompt) =
            resolve_start_inputs(Some(&snapshot), None, String::new()).unwrap();
        assert_eq!(agent_id, "agent-1");
        assert_eq!(initial_prompt.as_deref(), Some("stored task"));
    }

    #[test]
    fn explicit_prompt_overrides_stored_user_query() {
        let snapshot = snapshot("agent-1", "stored task");
        let (_, initial_prompt) =
            resolve_start_inputs(Some(&snapshot), None, "new task".into()).unwrap();
        assert_eq!(initial_prompt.as_deref(), Some("new task"));
    }

    #[test]
    fn empty_stored_query_starts_without_initial_message() {
        // Resuming a waiting session must not inject a synthetic message.
        let snapshot = snapshot("agent-1", "");
        let (_, initial_prompt) =
            resolve_start_inputs(Some(&snapshot), None, String::new()).unwrap();
        assert!(initial_prompt.is_none());
    }

    #[test]
    fn missing_agent_in_snapshot_is_a_protocol_error() {
        let snapshot = json!({ "workflow": {} });
        let error =
            resolve_start_inputs(Some(&snapshot), None, String::new()).expect_err("must fail");
        assert!(matches!(error, CliError::Protocol(_)));
    }
}
