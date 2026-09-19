//! `cs skill` — Agent Skill commands.
//!
//! Reads are plain `GET`s; mutations are `POST`s that always carry an
//! idempotency key, so a CLI retry cannot install or remove the same Skill
//! twice. Every command is an HTTP adapter: the CLI never opens the database or
//! a capability directory (INV-1).

use crate::args::{Cli, SkillCommand};
use crate::capability::{fetch_and_render, items, name_of, read_source_document, render, text};
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use serde_json::{json, Value};

/// Runs one `cs skill` subcommand.
pub async fn run(
    cli: &Cli,
    client: &ControlPlaneClient,
    command: &SkillCommand,
) -> Result<(), CliError> {
    match command {
        SkillCommand::Targets => {
            fetch_and_render(cli, client, "/control/v1/skill-targets", human_targets).await
        }
        SkillCommand::List => {
            fetch_and_render(cli, client, "/control/v1/skills", human_inventory).await
        }
        SkillCommand::Check {
            source_json,
            source_file,
        } => {
            let source = read_source_document(source_json, source_file)?;
            let value = client.post("/control/v1/skill-check", source, None).await?;
            render(cli, &value, human_check)
        }
        SkillCommand::Install {
            source_json,
            source_file,
            targets,
            idempotency_key,
        } => {
            let source = read_source_document(source_json, source_file)?;
            let key = generated_idempotency_key(idempotency_key);
            let body = json!({ "source": source, "targets": targets });
            let value = client
                .post("/control/v1/skill-install", body, Some(&key))
                .await?;
            render(cli, &value, human_install)
        }
        SkillCommand::Uninstall {
            name,
            targets,
            idempotency_key,
        } => {
            let key = generated_idempotency_key(idempotency_key);
            let body = json!({ "skill_name": name, "targets": targets });
            let value = client
                .post("/control/v1/skill-uninstall", body, Some(&key))
                .await?;
            render(cli, &value, human_uninstall)
        }
    }
}

/// Uses the caller's key, or mints one so a mutation is always idempotent.
fn generated_idempotency_key(provided: &Option<String>) -> String {
    match provided {
        Some(key) if !key.trim().is_empty() => key.clone(),
        _ => format!("cs-skill-{}", uuid::Uuid::new_v4().simple()),
    }
}

/// `<verdict>  <checker>  <name|->  <files>  <bytes>`
fn human_check(value: &Value) -> Vec<String> {
    let mut rows = vec![rust_i18n::t!("cs.skill_check_header").to_string()];
    rows.push(format!(
        "{}\t{}\t{}\t{}\t{}",
        text(value, "verdict"),
        text(value, "checker_version"),
        text(value, "skill_name"),
        text(value, "file_count"),
        text(value, "total_bytes")
    ));
    rows.extend(findings(value));
    rows
}

/// `<target>  <status>  <path|->` plus the operation that produced it.
fn human_install(value: &Value) -> Vec<String> {
    let detail = result_of(value);
    let mut rows = vec![format!(
        "{}\t{}",
        rust_i18n::t!("cs.skill_install_header"),
        text(detail, "skill_name")
    )];
    rows.push(operation_row(value));
    let outcomes = detail
        .get("install")
        .map(|install| items(install, "outcomes").to_vec())
        .unwrap_or_default();
    append_outcomes(&mut rows, &outcomes);
    rows.extend(findings(detail));
    rows
}

/// `<target>  <status>  <detail|->` for an uninstall.
fn human_uninstall(value: &Value) -> Vec<String> {
    let detail = result_of(value);
    let mut rows = vec![format!(
        "{}\t{}",
        rust_i18n::t!("cs.skill_uninstall_header"),
        text(detail, "skill_name")
    )];
    rows.push(operation_row(value));
    let outcomes: Vec<Value> = items(detail, "outcomes").to_vec();
    append_outcomes(&mut rows, &outcomes);
    rows
}

/// The recorded projection of a mutation response.
///
/// A mutation answers with `{operation_id, replayed, result}`, while a bare
/// document also has to render, so the payload is looked up leniently.
fn result_of(value: &Value) -> &Value {
    value.get("result").unwrap_or(value)
}

/// `<operation label>  <operation id>  <applied|replayed>`
fn operation_row(value: &Value) -> String {
    format!(
        "{}\t{}\t{}",
        rust_i18n::t!("cs.skill_operation"),
        text(value, "operation_id"),
        if value["replayed"] == Value::Bool(true) {
            "replayed"
        } else {
            "applied"
        }
    )
}

fn append_outcomes(rows: &mut Vec<String>, outcomes: &[Value]) {
    if outcomes.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return;
    }
    for outcome in outcomes {
        rows.push(format!(
            "{}\t{}\t{}",
            text(outcome, "target_id"),
            text(outcome, "status"),
            text(outcome, "detail")
        ));
    }
}

/// Every finding, so a blocked verdict is actionable from the terminal.
fn findings(value: &Value) -> Vec<String> {
    items(value, "findings")
        .iter()
        .map(|finding| {
            format!(
                "{}\t{}\t{}\t{}",
                text(finding, "severity"),
                text(finding, "rule"),
                text(finding, "path"),
                text(finding, "detail")
            )
        })
        .collect()
}

/// `<id>  <default|->  <supported|unsupported>  <reason|->`
fn human_targets(value: &Value) -> Vec<String> {
    let targets = value.as_array().cloned().unwrap_or_default();
    let mut rows = vec![rust_i18n::t!("cs.skill_targets_header").to_string()];
    if targets.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    }
    for target in &targets {
        let selected = if target["default_selected"] == Value::Bool(true) {
            "default"
        } else {
            "-"
        };
        let supported = if target["supported"] == Value::Bool(true) {
            "supported"
        } else {
            "unsupported"
        };
        rows.push(format!(
            "{}\t{}\t{}\t{}",
            text(target, "id"),
            selected,
            supported,
            text(target, "unsupported_reason")
        ));
    }
    rows
}

/// `<name>  <source>  <target|->  <managed|unmanaged>  <drifted|clean>`
fn human_inventory(value: &Value) -> Vec<String> {
    let skills = items(value, "skills");
    let mut rows = vec![rust_i18n::t!("cs.skill_list_header").to_string()];
    if skills.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    }
    for skill in skills {
        rows.push(format!(
            "{}\t{}\t{}\t{}\t{}",
            name_of(skill),
            text(skill, "source"),
            text(skill, "target_id"),
            if skill["managed"] == Value::Bool(true) {
                "managed"
            } else {
                "unmanaged"
            },
            if skill["drifted"] == Value::Bool(true) {
                "drifted"
            } else {
                "clean"
            }
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn human_targets_marks_selection_and_support() {
        let value = json!([
            { "id": "chatspeed", "default_selected": true, "supported": true },
            { "id": "codex", "default_selected": false, "supported": false, "unsupported_reason": "path_not_verified" },
        ]);
        let rows = human_targets(&value);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("chatspeed"));
        assert!(rows[1].contains("default"));
        assert!(rows[1].contains("supported"));
        assert!(rows[2].contains("unsupported"));
        assert!(rows[2].contains("path_not_verified"));
    }

    #[test]
    fn human_inventory_reports_ownership_and_drift() {
        let value = json!({
            "targets": [],
            "skills": [
                { "name": "demo", "source": "managed", "target_id": "chatspeed", "managed": true, "drifted": false },
                { "name": "loose", "source": "discovered", "managed": false, "drifted": false },
            ],
        });
        let rows = human_inventory(&value);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("managed"));
        assert!(rows[1].contains("clean"));
        assert!(rows[2].contains("unmanaged"));
        // A missing target is rendered as `-`, never as an empty cell.
        assert!(rows[2].contains('\t'));
    }

    #[test]
    fn an_empty_inventory_says_so_instead_of_printing_nothing() {
        let rows = human_inventory(&json!({ "skills": [] }));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn human_check_reports_the_verdict_and_every_finding() {
        let rows = human_check(&json!({
            "verdict": "blocked",
            "checker_version": "skill-checker.v1",
            "skill_name": "stealer",
            "file_count": 3,
            "total_bytes": 128,
            "findings": [{
                "severity": "blocked",
                "rule": "script.has_side_effects",
                "path": "scripts/steal.sh",
                "detail": "executes a process",
            }],
        }));
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("blocked"));
        assert!(rows[1].contains("skill-checker.v1"));
        assert!(rows[2].contains("script.has_side_effects"));
        assert!(rows[2].contains("scripts/steal.sh"));
    }

    #[test]
    fn human_install_lists_every_target_outcome_and_the_operation() {
        // The renderer is exercised against the real DTOs, so a renamed backend
        // field breaks this test instead of silently printing `-` (V-8).
        use chatspeed_lib::capability::skill::installer::{
            InstallSummary, TargetOutcome, TargetOutcomeStatus,
        };
        use chatspeed_lib::capability::skill::orchestrator::SkillMutationResult;

        let summary = InstallSummary {
            plan_id: "plan-1".to_string(),
            skill_name: "demo".to_string(),
            content_digest: "digest".to_string(),
            outcomes: vec![
                TargetOutcome {
                    target_id: "chatspeed".to_string(),
                    status: TargetOutcomeStatus::Installed,
                    install_path: Some("/home/u/.chatspeed/skills/demo".to_string()),
                    installation_id: Some("ins-1".to_string()),
                    detail: None,
                },
                TargetOutcome {
                    target_id: "codex".to_string(),
                    status: TargetOutcomeStatus::Unsupported,
                    install_path: None,
                    installation_id: None,
                    detail: Some("path_not_verified".to_string()),
                },
            ],
        };
        let value = serde_json::to_value(SkillMutationResult {
            operation_id: "op-1".to_string(),
            replayed: true,
            // The recorded projection nests the install summary under `install`.
            result: serde_json::json!({
                "stage": "applied",
                "skill_name": "demo",
                "install": serde_json::to_value(&summary).expect("summary json"),
            }),
        })
        .expect("mutation json");

        let rows = human_install(&value);
        assert_eq!(rows.len(), 4);
        assert!(rows[0].contains("demo"));
        assert!(rows[1].contains("op-1"));
        assert!(rows[1].contains("replayed"));
        assert!(rows[2].contains("installed"));
        assert!(rows[3].contains("unsupported"));
        assert!(rows[3].contains("path_not_verified"));
    }

    #[test]
    fn human_uninstall_surfaces_a_refusal_instead_of_hiding_it() {
        use chatspeed_lib::capability::skill::orchestrator::SkillMutationResult;
        use chatspeed_lib::capability::skill::uninstaller::{
            UninstallOutcome, UninstallOutcomeStatus,
        };

        let outcomes = vec![UninstallOutcome {
            target_id: "chatspeed".to_string(),
            skill_name: "demo".to_string(),
            status: UninstallOutcomeStatus::Refused,
            install_path: Some("/home/u/.chatspeed/skills/demo".to_string()),
            quarantine_path: None,
            detail: Some("content_drifted".to_string()),
        }];
        let value = serde_json::to_value(SkillMutationResult {
            operation_id: "op-2".to_string(),
            replayed: false,
            result: serde_json::json!({ "skill_name": "demo", "outcomes": outcomes }),
        })
        .expect("mutation json");

        let rows = human_uninstall(&value);
        assert!(rows[1].contains("applied"));
        assert!(rows[2].contains("refused"));
        assert!(rows[2].contains("content_drifted"));
    }

    #[test]
    fn a_generated_key_keeps_the_callers_key_when_one_was_given() {
        assert_eq!(
            generated_idempotency_key(&Some("mine".to_string())),
            "mine"
        );
        // A blank key would be refused by the server, so one is minted.
        let generated = generated_idempotency_key(&Some("   ".to_string()));
        assert!(generated.starts_with("cs-skill-"));
        assert_ne!(generated, generated_idempotency_key(&None));
    }
}
