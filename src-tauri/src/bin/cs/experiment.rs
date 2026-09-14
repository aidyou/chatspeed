//! `cs experiment` command orchestration (Phase 2A).
//!
//! `capture` is a read-only consumer of the existing control plane: it fetches
//! `meta`, the authoritative workflow snapshot, and all durable events, then
//! writes a redacted artifact bundle locally. `inspect`/`replay` are fully
//! offline: they read only the artifact directory and never touch discovery,
//! the network, the database, or any LLM key.

use crate::args::{Cli, OutputFormat};
use crate::artifact::{self, ArtifactError};
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use crate::output::{eprint_diagnostic, render_result};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

/// Durable events page size. The server clamps to its own maximum (500).
const EVENTS_PAGE_LIMIT: usize = 500;

/// Captures an existing workflow session into an artifact directory.
pub async fn capture(
    cli: &Cli,
    client: &ControlPlaneClient,
    session_id: &str,
    artifact_dir: &Path,
) -> Result<(), CliError> {
    let meta = client.get("/control/v1/meta").await?;
    let server_instance_id = meta
        .get("server_instance_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let protocol_version = meta
        .get("protocol_version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let snapshot = client
        .get(&format!("/control/v1/workflows/{}", session_id))
        .await?;
    let agent_id = snapshot
        .get("workflow")
        .and_then(|workflow| workflow.get("agent_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let events = fetch_all_events(client, session_id).await?;

    let capture_timestamp = chrono::Utc::now().to_rfc3339();
    let input = artifact::CaptureInput {
        session_id,
        agent_id: &agent_id,
        server_instance_id: &server_instance_id,
        protocol_version: &protocol_version,
        capture_timestamp: &capture_timestamp,
        snapshot: &snapshot,
        events: &events,
    };

    let bundle = artifact::construct_bundle(&input).map_err(to_cli_error)?;
    artifact::write_bundle(&bundle, artifact_dir).map_err(to_cli_error)?;

    let status = result_status(&bundle);
    match cli.output {
        OutputFormat::Human => {
            eprint_diagnostic(&format!(
                "cs: artifact captured to {} (status={})",
                artifact_dir.display(),
                status
            ));
            render_result(
                cli.output,
                &json!({
                    "artifact_dir": artifact_dir.display().to_string(),
                    "artifact_status": status,
                    "session_id": session_id,
                }),
            );
        }
        _ => {
            render_result(
                cli.output,
                &json!({
                    "artifact_dir": artifact_dir.display().to_string(),
                    "artifact_status": status,
                    "session_id": session_id,
                    "event_count": bundle_event_count(&bundle),
                }),
            );
        }
    }
    Ok(())
}

/// Pages through all durable events for a session using the durable `after`
/// cursor (never an SSE cursor). Stops when a page is short of the limit.
async fn fetch_all_events(
    client: &ControlPlaneClient,
    session_id: &str,
) -> Result<Vec<Value>, CliError> {
    let mut all: Vec<Value> = Vec::new();
    let mut after: Option<i64> = None;
    loop {
        let mut path = format!(
            "/control/v1/workflows/{}/events?limit={}",
            session_id, EVENTS_PAGE_LIMIT
        );
        if let Some(after) = after {
            path.push_str(&format!("&after={}", after));
        }
        let page = client.get(&path).await?;
        let items = page
            .as_array()
            .ok_or_else(|| CliError::protocol("Durable events response is not a JSON array"))?;
        if items.is_empty() {
            break;
        }
        let last_id = items
            .last()
            .and_then(|event| event.get("id"))
            .and_then(Value::as_i64);
        all.extend(items.iter().cloned());
        if all.len() > artifact::MAX_EVENTS {
            return Err(CliError::io(format!(
                "session has more than {} durable events; refusing to capture",
                artifact::MAX_EVENTS
            )));
        }
        if items.len() < EVENTS_PAGE_LIMIT {
            break;
        }
        match last_id {
            Some(id) => after = Some(id),
            None => break,
        }
    }
    Ok(all)
}

/// Verifies an artifact directory offline and renders the inspect projection.
pub fn inspect(cli: &Cli, artifact_dir: &Path) -> Result<(), CliError> {
    render_verification(cli, artifact_dir, artifact::inspect_projection)
}

/// Verifies an artifact directory offline and renders the replay projection.
pub fn replay(cli: &Cli, artifact_dir: &Path) -> Result<(), CliError> {
    render_verification(cli, artifact_dir, artifact::replay_projection)
}

fn render_verification(
    cli: &Cli,
    artifact_dir: &Path,
    project: fn(&artifact::VerifyReport) -> Value,
) -> Result<(), CliError> {
    match artifact::verify_bundle_dir(artifact_dir) {
        Ok(report) => {
            let projection = project(&report);
            if cli.output == OutputFormat::Human {
                render_human_verification(artifact_dir, &projection);
            } else {
                render_result(cli.output, &projection);
            }
            Ok(())
        }
        Err(error) => {
            let projection = artifact::error_projection(artifact_dir, &error);
            if cli.output == OutputFormat::Human {
                eprint_diagnostic(&format!("cs: {}: {}", error.code, error.message));
            } else {
                render_result(cli.output, &projection);
            }
            Err(CliError::io(format!("{}: {}", error.code, error.message)))
        }
    }
}

fn render_human_verification(artifact_dir: &Path, projection: &Value) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = writeln!(handle, "artifact: {}", artifact_dir.display());
    let _ = writeln!(
        handle,
        "status:   {} (terminal={}, cost={})",
        projection["artifact_status"].as_str().unwrap_or("?"),
        projection["terminal_status"].as_str().unwrap_or("?"),
        projection["cost_status"].as_str().unwrap_or("?"),
    );
    if let Some(count) = projection.get("event_count").and_then(Value::as_u64) {
        let _ = writeln!(handle, "events:   {}", count);
    }
    if let Some(timeline) = projection.get("timeline").and_then(Value::as_array) {
        for item in timeline {
            let _ = writeln!(
                handle,
                "  #{} {}",
                item["durable_id"].as_str().unwrap_or("?"),
                item["event_type"].as_str().unwrap_or("?"),
            );
        }
    }
}

fn result_status(bundle: &artifact::ArtifactBundle) -> &'static str {
    serde_json::from_str::<Value>(&bundle.result_json)
        .ok()
        .and_then(|value| {
            value
                .get("artifact_status")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(|status| match status.as_str() {
            "complete" => "complete",
            "incomplete" => "incomplete",
            _ => "invalid",
        })
        .unwrap_or("invalid")
}

fn bundle_event_count(bundle: &artifact::ArtifactBundle) -> usize {
    bundle
        .events_jsonl
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
}

/// Maps an artifact error to a CLI error, preserving the machine code prefix.
fn to_cli_error(error: ArtifactError) -> CliError {
    CliError::io(format!("{}: {}", error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    fn write_valid_artifact(dir: &Path) -> std::path::PathBuf {
        let snapshot = json!({
            "workflow": {
                "id": "s1", "agent_id": "builtin:coding", "status": "completed",
                "wait_reason": null, "user_query": "prompt text",
                "agent_config": "{\"models\":{\"act\":{\"id\":0,\"model\":\"cs@free:ds-v4-flash\"}}}",
                "is_automation_run": false
            },
            "messages": [], "has_live_session": false
        });
        let events = vec![
            json!({
                "id": 1, "session_id": "s1", "event_type": "workflow_started",
                "event_version": "1.0.0", "created_at": "t0",
                "event_data": { "agent_id": "builtin:coding" }
            }),
            json!({
                "id": 2, "session_id": "s1", "event_type": "task_completed",
                "event_version": "1.0.0", "created_at": "t1",
                "event_data": { "usage_summary": {
                    "version": 1, "terminal_status": "completed", "duration_ms": 10,
                    "is_partial": false, "has_sub_agents": false,
                    "self_usage": { "total_tokens": 5, "unpriced_tokens": 0, "estimated_cost": 0.001 },
                    "with_sub_agents": { "total_tokens": 5, "unpriced_tokens": 0, "estimated_cost": 0.001 },
                    "model_breakdowns": []
                } }
            }),
        ];
        let input = artifact::CaptureInput {
            session_id: "s1",
            agent_id: "builtin:coding",
            server_instance_id: "inst",
            protocol_version: "1.0",
            capture_timestamp: "now",
            snapshot: &snapshot,
            events: &events,
        };
        let bundle = artifact::construct_bundle(&input).expect("construct");
        let target = dir.join("artifact");
        artifact::write_bundle(&bundle, &target).expect("write");
        target
    }

    fn cli_for(subcommand: &str, dir: &Path) -> Cli {
        let mut argv = vec!["cs", "experiment", subcommand];
        let dir_str = dir.display().to_string();
        argv.push(dir_str.as_str());
        Cli::try_parse_from(argv).expect("valid args")
    }

    #[test]
    fn inspect_and_replay_succeed_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_valid_artifact(tmp.path());
        // These calls never load discovery or touch the network/DB.
        let inspect_cli = cli_for("inspect", &artifact_dir);
        inspect(&inspect_cli, &artifact_dir).expect("inspect ok");
        let replay_cli = cli_for("replay", &artifact_dir);
        replay(&replay_cli, &artifact_dir).expect("replay ok");
    }

    #[test]
    fn inspect_fails_closed_on_tamper() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_valid_artifact(tmp.path());
        // Tamper with run.json without refreshing the manifest.
        let run_path = artifact_dir.join("run.json");
        let mut run: Value =
            serde_json::from_str(&std::fs::read_to_string(&run_path).unwrap()).unwrap();
        run["agent_id"] = json!("evil-agent");
        std::fs::write(&run_path, serde_json::to_string_pretty(&run).unwrap()).unwrap();

        let cli = cli_for("inspect", &artifact_dir);
        let error = inspect(&cli, &artifact_dir).expect_err("must fail closed");
        assert!(matches!(error, CliError::Io(_)));
    }

    #[test]
    fn result_status_and_event_count_helpers() {
        let snapshot = json!({ "workflow": { "status": "running" }, "messages": [] });
        let events = vec![json!({
            "id": 1, "session_id": "s1", "event_type": "workflow_started",
            "event_version": "1.0.0", "created_at": "t0", "event_data": {}
        })];
        let input = artifact::CaptureInput {
            session_id: "s1",
            agent_id: "a",
            server_instance_id: "i",
            protocol_version: "1.0",
            capture_timestamp: "now",
            snapshot: &snapshot,
            events: &events,
        };
        let bundle = artifact::construct_bundle(&input).expect("construct");
        assert_eq!(result_status(&bundle), "incomplete");
        assert_eq!(bundle_event_count(&bundle), 1);
    }
}
