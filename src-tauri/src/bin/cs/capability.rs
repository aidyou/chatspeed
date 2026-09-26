//! Shared helpers for the Phase 3 capability read commands.
//!
//! The CLI is a pure HTTP adapter: every command reads one `/control/v1`
//! document and renders it. It never opens the database, a capability
//! directory or the runtime (INV-1).

use crate::args::{Cli, OutputFormat};
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use crate::output::{print_json, print_jsonl};
use serde_json::{json, Value};
use std::io::Write;

/// Fetches one control-plane path and renders it in the requested format.
///
/// The human rendering is a pure function of the parsed document, so it can be
/// tested without a live control plane, and `json`/`jsonl` always emit the
/// unmodified document so the machine contract stays stable.
pub async fn fetch_and_render(
    cli: &Cli,
    client: &ControlPlaneClient,
    path: &str,
    human: fn(&Value) -> Vec<String>,
) -> Result<(), CliError> {
    let value = client.get(path).await?;
    match cli.output {
        OutputFormat::Json => print_json(&value),
        OutputFormat::Jsonl => print_jsonl(&value),
        OutputFormat::Human => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            for row in human(&value) {
                let _ = writeln!(handle, "{}", row);
            }
        }
    }
    Ok(())
}

/// Renders one already-fetched document in the requested format.
///
/// Mutation responses use this so `json`/`jsonl` emit the unmodified document
/// and the human layout stays a testable pure function of it.
pub fn render(cli: &Cli, value: &Value, human: fn(&Value) -> Vec<String>) -> Result<(), CliError> {
    match cli.output {
        OutputFormat::Json => print_json(value),
        OutputFormat::Jsonl => print_jsonl(value),
        OutputFormat::Human => {
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            for row in human(value) {
                let _ = writeln!(handle, "{}", row);
            }
        }
    }
    Ok(())
}

/// Reads a structured source document from `--source-json` or `--source-file`.
///
/// The document is parsed here so an unreadable file or malformed JSON fails as
/// a usage error before any request reaches the control plane.
pub fn read_source_document(
    source_json: &Option<String>,
    source_file: &Option<std::path::PathBuf>,
) -> Result<Value, CliError> {
    let raw = match (source_json, source_file) {
        (Some(json), _) => json.clone(),
        (None, Some(path)) if path == &std::path::PathBuf::from("-") => {
            use std::io::Read;
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|e| CliError::io(format!("Failed to read the source from stdin: {e}")))?;
            buffer
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .map_err(|e| CliError::io(format!("Failed to read {}: {e}", path.display())))?,
        (None, None) => {
            return Err(CliError::usage(
                "Either --source-json or --source-file is required",
            ));
        }
    };
    serde_json::from_str(&raw)
        .map_err(|e| CliError::usage(format!("The source document is not valid JSON: {e}")))
}

/// A text cell, or `-` when the field is absent.
pub fn cell(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        _ => "-".to_string(),
    }
}

/// A text cell from an owned lookup, or `-`.
pub fn text(value: &Value, key: &str) -> String {
    cell(value.get(key))
}

/// The `name` field of a record, or `-`.
pub fn name_of(value: &Value) -> String {
    text(value, "name")
}

/// The elements of an array field, or an empty slice.
pub fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(|field| field.as_array())
        .map(|items| items.as_slice())
        .unwrap_or(&[])
}

/// Implements `cs doctor capabilities`.
///
/// This is an additive sub-capability of `cs doctor`: the bare `cs doctor`
/// connectivity check keeps its exact previous behaviour and exit codes.
pub async fn doctor_capabilities(
    cli: &Cli,
    client: &ControlPlaneClient,
) -> Result<(), CliError> {
    fetch_and_render(cli, client, "/control/v1/capability-doctor", human_doctor).await
}

/// Implements `cs doctor reconcile`.
///
/// A mutation, so it always carries an idempotency key (minted when the flag is
/// omitted) and posts an empty body to the control-plane reconcile route. The
/// server converges only proven interrupted effects and leaves the rest as
/// `needs_reconcile`; the CLI renders that report unchanged.
pub async fn doctor_reconcile(
    cli: &Cli,
    client: &ControlPlaneClient,
    provided_key: &Option<String>,
) -> Result<(), CliError> {
    let key = match provided_key {
        Some(key) if !key.trim().is_empty() => key.clone(),
        _ => format!("cs-reconcile-{}", uuid::Uuid::new_v4().simple()),
    };
    let value = client
        .post("/control/v1/capability-doctor/reconcile", json!({}), Some(&key))
        .await?;
    render(cli, &value, human_reconcile)
}

/// Renders the doctor report as one line per section plus the finding codes.
fn human_doctor(value: &Value) -> Vec<String> {
    let journal = value.get("journal").cloned().unwrap_or(Value::Null);
    let skills = value.get("skills").cloned().unwrap_or(Value::Null);
    let mcp = value.get("mcp").cloned().unwrap_or(Value::Null);
    let staging = value.get("staging").cloned().unwrap_or(Value::Null);

    let mut rows = vec![rust_i18n::t!("cs.capability_doctor_header").to_string()];
    rows.push(format!(
        "journal\tinterrupted={}\tneeds_reconcile={}",
        items(&journal, "interrupted").len(),
        items(&journal, "needs_reconcile").len()
    ));
    rows.push(format!(
        "skills\tbuiltin={}\tmanaged={}\tdiscovered={}\tdrifted={}\tmissing={}",
        text(&skills, "builtin"),
        text(&skills, "managed"),
        text(&skills, "discovered"),
        items(&skills, "drifted").len(),
        items(&skills, "missing").len()
    ));
    rows.push(format!(
        "mcp\tregistered={}\tdesired_enabled={}\tdrift={}",
        text(&mcp, "registered"),
        text(&mcp, "desired_enabled"),
        items(&mcp, "drift").len()
    ));
    rows.push(format!(
        "staging\tstaging_entries={}\tquarantine_entries={}",
        text(&staging, "staging_entries"),
        text(&staging, "quarantine_entries")
    ));

    let findings = items(value, "findings");
    if findings.is_empty() {
        rows.push(rust_i18n::t!("cs.capability_doctor_clean").to_string());
    } else {
        let joined = findings
            .iter()
            .map(|finding| cell(Some(finding)))
            .collect::<Vec<_>>()
            .join(",");
        rows.push(
            rust_i18n::t!("cs.capability_doctor_findings", findings = joined).to_string(),
        );
    }

    rows
}

/// Renders a reconcile report as one converged-count line plus the operation
/// IDs still needing attention.
fn human_reconcile(value: &Value) -> Vec<String> {
    let mut rows = vec![rust_i18n::t!("cs.capability_reconcile_header").to_string()];
    rows.push(format!(
        "quarantines_finalized={}\tinstalls_recovered={}\tmcp_effects_recovered={}\tstaging_residue_removed={}",
        items(value, "quarantines_finalized").len(),
        items(value, "installs_recovered").len(),
        items(value, "mcp_effects_recovered").len(),
        text(value, "staging_residue_removed"),
    ));
    let still = items(value, "still_needs_reconcile");
    if still.is_empty() {
        rows.push(rust_i18n::t!("cs.capability_reconcile_clean").to_string());
    } else {
        let joined = still
            .iter()
            .map(|id| cell(Some(id)))
            .collect::<Vec<_>>()
            .join(",");
        rows.push(rust_i18n::t!(
            "cs.capability_reconcile_pending",
            ids = joined
        ).to_string());
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cells_render_absent_and_present_fields_consistently() {
        let value = json!({ "name": "weather", "enabled": true, "count": 3, "empty": "" });
        assert_eq!(text(&value, "name"), "weather");
        assert_eq!(text(&value, "enabled"), "true");
        assert_eq!(text(&value, "count"), "3");
        assert_eq!(text(&value, "empty"), "-");
        assert_eq!(text(&value, "missing"), "-");
    }

    #[test]
    fn items_of_a_non_array_field_is_empty() {
        let value = json!({ "skills": [{ "name": "a" }], "targets": "nope" });
        assert_eq!(items(&value, "skills").len(), 1);
        assert!(items(&value, "targets").is_empty());
        assert!(items(&value, "missing").is_empty());
    }

    #[test]
    fn a_source_document_is_read_inline_or_from_a_file() {
        let inline = read_source_document(
            &Some("{\"kind\":\"local_directory\",\"path\":\"/tmp/demo\"}".to_string()),
            &None,
        )
        .expect("inline source");
        assert_eq!(inline["kind"], "local_directory");

        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("source.json");
        std::fs::write(&path, "{\"kind\":\"zip\",\"path\":\"/tmp/demo.zip\"}").expect("write");
        let from_file = read_source_document(&None, &Some(path)).expect("file source");
        assert_eq!(from_file["kind"], "zip");
    }

    #[test]
    fn a_missing_or_malformed_source_document_is_a_usage_error() {
        // A mutation must never reach the control plane without a source.
        let missing = read_source_document(&None, &None).expect_err("no source");
        assert_eq!(missing.exit_code(), 2);

        let malformed = read_source_document(&Some("not json".to_string()), &None)
            .expect_err("malformed source");
        assert_eq!(malformed.exit_code(), 2);

        let unreadable = read_source_document(
            &None,
            &Some(std::path::PathBuf::from("/definitely/not/here.json")),
        )
        .expect_err("unreadable source");
        assert_eq!(unreadable.exit_code(), 1);
    }

    #[test]
    fn the_doctor_rendering_reports_every_section_and_the_finding_codes() {
        let value = json!({
            "journal": { "interrupted": [], "needs_reconcile": ["op-skill-1"] },
            "skills": {
                "builtin": 1, "managed": 0, "discovered": 2,
                "drifted": [], "missing": ["chatspeed:gone"], "orphan_ownership": ["chatspeed:gone"],
            },
            "mcp": { "registered": 1, "desired_enabled": 1, "drift": ["weather:desired_enabled_not_running"] },
            "staging": { "staging_root": "/tmp/s", "quarantine_root": "/tmp/q", "staging_entries": 1, "quarantine_entries": 0 },
            "findings": ["operation_needs_reconcile", "skill_directory_missing"],
        });
        let rows = human_doctor(&value);
        assert!(rows[1].contains("needs_reconcile=1"));
        assert!(rows[2].contains("discovered=2"));
        assert!(rows[2].contains("missing=1"));
        assert!(rows[3].contains("drift=1"));
        assert!(rows[4].contains("staging_entries=1"));
        assert!(rows[5].contains("operation_needs_reconcile"));
    }

    #[test]
    fn a_clean_report_says_so() {
        let value = json!({
            "journal": { "interrupted": [], "needs_reconcile": [] },
            "skills": { "builtin": 0, "managed": 0, "discovered": 0, "drifted": [], "missing": [] },
            "mcp": { "registered": 0, "desired_enabled": 0, "drift": [] },
            "staging": { "staging_entries": 0, "quarantine_entries": 0 },
            "findings": [],
        });
        let rows = human_doctor(&value);
        assert_eq!(rows.len(), 6);
    }

    #[test]
    fn the_reconcile_rendering_reports_convergence_and_pending_operations() {
        let converged = human_reconcile(&json!({
            "quarantines_finalized": ["chatspeed:demo"],
            "installs_recovered": ["chatspeed:other"],
            "mcp_effects_recovered": ["mcp:weather:mcp.stop"],
            "staging_residue_removed": 2,
            "still_needs_reconcile": ["op-1", "op-2"],
        }));
        assert!(converged[1].contains("quarantines_finalized=1"));
        assert!(converged[1].contains("installs_recovered=1"));
        assert!(converged[1].contains("mcp_effects_recovered=1"));
        assert!(converged[1].contains("staging_residue_removed=2"));
        assert!(converged[2].contains("op-1"));
        assert!(converged[2].contains("op-2"));

        let clean = human_reconcile(&json!({
            "quarantines_finalized": [],
            "installs_recovered": [],
            "mcp_effects_recovered": [],
            "staging_residue_removed": 0,
            "still_needs_reconcile": [],
        }));
        // The last line reports nothing outstanding.
        assert_eq!(clean.len(), 3);
    }
}
