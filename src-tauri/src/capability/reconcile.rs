//! Durable, service-owned capability reconciliation.
//!
//! Doctor identifies drift; this module converges it. Reconcile is the only
//! place a `needs_reconcile` operation may be advanced after a crash, and it
//! does so strictly from proven durable evidence (AC-2/AC-7/INV-8):
//!
//! - a Skill quarantine that never finalized is completed (the moved-aside
//!   directory is deleted and the ownership row recorded as removed);
//! - a private staging tree no live operation owns is discarded;
//! - an interrupted install whose committed content still matches its ownership
//!   proof is finalized; one that does not is left untouched;
//! - an MCP effect whose persistence and runtime both prove the terminal result
//!   is recorded and its operation completed.
//!
//! Anything whose effect state cannot be proven is deliberately left in
//! `needs_reconcile` — never blind-retried, and never deleted on a guess.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::capability::error::CapabilityError;
use crate::capability::mcp::runtime::ObservedMcpRuntime;
use crate::capability::skill::staging::StagingArea;
use crate::capability::skill::uninstaller::SkillUninstaller;
use crate::capability::types::{
    CapabilityKind, CapabilityOperation, EffectOutcome, OperationState, SkillInstallationState,
};
use crate::capability::{staging_dir, CapabilityApplicationService};

/// A staging entry this recent may still be owned by a live mutation, so
/// reconcile leaves it for the next pass rather than racing it.
const STAGING_GRACE: Duration = Duration::from_secs(600);

/// Stable finding codes reconcile reports, so adapters can branch on structure.
pub mod reconcile_finding {
    pub const QUARANTINE_FINALIZED: &str = "quarantine_finalized";
    pub const INSTALL_COMMIT_RECOVERED: &str = "install_commit_recovered";
    pub const STAGING_RESIDUE_REMOVED: &str = "staging_residue_removed";
    pub const MCP_EFFECT_RECOVERED: &str = "mcp_effect_recovered";
    pub const STILL_NEEDS_RECONCILE: &str = "still_needs_reconcile";
}

/// The MCP effect keys reconcile may prove from persistence + runtime.
const EFFECT_START: &str = "mcp.start";
const EFFECT_STOP: &str = "mcp.stop";
const EFFECT_DELETE: &str = "mcp.delete";

/// What one reconcile pass converged, plus what it intentionally left alone.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CapabilityReconcileReport {
    /// Skill quarantines completed (of the form `target:name`).
    pub quarantines_finalized: Vec<String>,
    /// Interrupted installs finalized to `Installed` (`target:name`).
    pub installs_recovered: Vec<String>,
    /// Orphaned private staging directories removed.
    pub staging_residue_removed: usize,
    /// MCP effects rolled forward to a proven terminal state
    /// (`mcp:<name>:<effect>`).
    pub mcp_effects_recovered: Vec<String>,
    /// `needs_reconcile` operations left untouched because their effect state
    /// could not be proven.
    pub still_needs_reconcile: Vec<String>,
    /// Stable finding codes, in report order.
    pub findings: Vec<String>,
}

impl CapabilityReconcileReport {
    /// Whether reconcile changed nothing at all.
    pub fn is_noop(&self) -> bool {
        self.quarantines_finalized.is_empty()
            && self.installs_recovered.is_empty()
            && self.staging_residue_removed == 0
            && self.mcp_effects_recovered.is_empty()
            && self.still_needs_reconcile.is_empty()
    }
}

impl CapabilityApplicationService {
    /// Converges every capability effect that durable evidence proves, and
    /// preserves the rest as `needs_reconcile`.
    ///
    /// Safe to run repeatedly: each convergence is one-way and guarded by the
    /// durable state it reads, so a second pass is a no-op. Startup recovery is
    /// folded in first so crash-interrupted operations are already classified
    /// before their effects are proven.
    pub async fn reconcile(&self) -> Result<CapabilityReconcileReport, CapabilityError> {
        let mut report = CapabilityReconcileReport::default();

        // Idempotent; a no-op when startup recovery already ran.
        let _ = self.recover_interrupted_operations()?;

        self.reconcile_skill_residue(&mut report)?;
        self.reconcile_staging_residue(&mut report)?;
        self.reconcile_operations(&mut report).await?;

        if !report.quarantines_finalized.is_empty() {
            report
                .findings
                .push(reconcile_finding::QUARANTINE_FINALIZED.to_string());
        }
        if !report.installs_recovered.is_empty() {
            report
                .findings
                .push(reconcile_finding::INSTALL_COMMIT_RECOVERED.to_string());
        }
        if report.staging_residue_removed > 0 {
            report
                .findings
                .push(reconcile_finding::STAGING_RESIDUE_REMOVED.to_string());
        }
        if !report.mcp_effects_recovered.is_empty() {
            report
                .findings
                .push(reconcile_finding::MCP_EFFECT_RECOVERED.to_string());
        }
        if !report.still_needs_reconcile.is_empty() {
            report
                .findings
                .push(reconcile_finding::STILL_NEEDS_RECONCILE.to_string());
        }

        if !report.is_noop() {
            log::info!(
                "[Capability][reconcile] finalized {} quarantine(s), recovered {} install(s) and {} mcp effect(s), removed {} staging residue, left {} needing reconcile",
                report.quarantines_finalized.len(),
                report.installs_recovered.len(),
                report.mcp_effects_recovered.len(),
                report.staging_residue_removed,
                report.still_needs_reconcile.len(),
            );
        }

        Ok(report)
    }

    /// Finalizes quarantined and provably-committed interrupted installations.
    fn reconcile_skill_residue(
        &self,
        report: &mut CapabilityReconcileReport,
    ) -> Result<(), CapabilityError> {
        let installations = self.repository().list_installations()?;
        let uninstaller = SkillUninstaller::new(self.environment().clone());

        // Only targets that resolve to a real, verified directory are converged;
        // an unverified path never receives a filesystem probe (INV-5/D-5).
        let supported: HashSet<String> = self
            .skill_targets()
            .iter()
            .filter(|target| target.supported)
            .map(|target| target.id.clone())
            .collect();

        for installation in &installations {
            let is_quarantined =
                matches!(installation.state, SkillInstallationState::Quarantined);
            if !is_quarantined && !matches!(installation.state, SkillInstallationState::Installing) {
                continue;
            }
            if !supported.contains(&installation.target_id) {
                continue;
            }
            // An unreadable marker or manifest is not proof, so an `Err` here
            // leaves the row untouched rather than deleting on a guess.
            let converged = uninstaller
                .reconcile_owned_directory(
                    self.repository(),
                    &installation.target_id,
                    &installation.skill_name,
                )
                .unwrap_or(None);
            if converged.is_some() {
                let key = format!("{}:{}", installation.target_id, installation.skill_name);
                if is_quarantined {
                    report.quarantines_finalized.push(key);
                } else {
                    report.installs_recovered.push(key);
                }
            }
        }
        Ok(())
    }

    /// Removes private staging trees no live operation owns.
    fn reconcile_staging_residue(
        &self,
        report: &mut CapabilityReconcileReport,
    ) -> Result<(), CapabilityError> {
        let staging_root = staging_dir(self.app_data_dir());
        if !staging_root.exists() {
            return Ok(());
        }

        // The staging trees a currently in-flight operation may still own (its
        // raw tree and its `<id>-plan` tree). The grace window below still
        // protects a freshly created directory from racing a live mutation.
        let mut live = HashSet::new();
        for operation in self.repository().list_interrupted()? {
            let id = &operation.operation_id;
            live.insert(StagingArea::sanitize_operation(id));
            live.insert(StagingArea::sanitize_operation(&format!("{id}-plan")));
        }

        let entries = std::fs::read_dir(&staging_root)
            .map_err(|error| CapabilityError::internal(format!("cannot read staging: {error}")))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if live.contains(&name) || entry_is_recent(&path) {
                continue;
            }
            match std::fs::remove_dir_all(&path) {
                Ok(()) => report.staging_residue_removed += 1,
                Err(error) => {
                    log::debug!(
                        "[Capability][reconcile] could not remove stale staging '{name}': {error}"
                    );
                }
            }
        }
        Ok(())
    }

    /// Rolls forward operations whose effects durable evidence now proves.
    async fn reconcile_operations(
        &self,
        report: &mut CapabilityReconcileReport,
    ) -> Result<(), CapabilityError> {
        let pending = self.repository().list_needing_reconcile()?;
        for operation in pending {
            // An `mcp.update` that bailed at the stop gate needs more than
            // proving effects: once the old runtime is proven gone, the updated
            // record's desired state (running or stopped) must actually be
            // reached before the operation may report completion (AC-10/AC-12/
            // INV-7/INV-8). The generic effect-prover only observes, so update is
            // handled by a dedicated roll-forward that may itself start the new
            // configuration from the persisted (un-redacted) record.
            if operation.capability == CapabilityKind::Mcp
                && operation.operation_kind == "mcp.update"
            {
                self.reconcile_update_operation(&operation, report).await?;
                continue;
            }

            let proven = match operation.capability {
                CapabilityKind::Skill => self.prove_skill_effects(&operation)?,
                CapabilityKind::Mcp => self.prove_mcp_effects(&operation, report).await?,
            };

            let remaining = self
                .repository()
                .count_unproven_effects(&operation.operation_id)?;
            if proven && remaining == 0 {
                let result = serde_json::json!({
                    "status": "reconciled",
                    "operation": operation.operation_kind,
                });
                self.finish_operation(
                    &operation.operation_id,
                    OperationState::Completed,
                    Some(&result),
                    None,
                )?;
            } else {
                report.still_needs_reconcile.push(operation.operation_id);
            }
        }
        Ok(())
    }

    /// Records proven outcomes for the Skill effects of one uninstall operation.
    fn prove_skill_effects(&self, operation: &CapabilityOperation) -> Result<bool, CapabilityError> {
        let Some(skill_name) = operation.resource_key.strip_prefix("skill:") else {
            return Ok(false);
        };
        let mut all_proven = true;
        for effect in self.repository().list_effects(&operation.operation_id)? {
            let Some(target_id) = effect.effect_key.strip_prefix("skill.uninstall:") else {
                if matches!(effect.outcome, EffectOutcome::Unknown | EffectOutcome::Pending) {
                    all_proven = false;
                }
                continue;
            };
            let installation = self
                .repository()
                .get_installation(target_id, skill_name)?;
            let converged = matches!(
                installation.as_ref().map(|row| row.state),
                Some(SkillInstallationState::Removed)
            );
            if converged {
                self.repository().record_effect_outcome(
                    &operation.operation_id,
                    &effect.effect_key,
                    EffectOutcome::Applied,
                    Some(&serde_json::json!({ "reconciled": true })),
                )?;
            } else {
                all_proven = false;
            }
        }
        Ok(all_proven)
    }

    /// Records proven outcomes for the MCP effects of one operation.
    async fn prove_mcp_effects(
        &self,
        operation: &CapabilityOperation,
        report: &mut CapabilityReconcileReport,
    ) -> Result<bool, CapabilityError> {
        let Some(name) = operation.resource_key.strip_prefix("mcp:") else {
            return Ok(false);
        };
        let mut all_proven = true;
        let record_present = self.mcp_repository().find_by_name(name)?.is_some();
        for effect in self.repository().list_effects(&operation.operation_id)? {
            if is_terminal_outcome(effect.outcome) {
                continue;
            }
            let observed = self.mcp_observe(name).await;
            let proven = match effect.effect_key.as_str() {
                EFFECT_DELETE if !record_present && proven_not_running(&observed) => {
                    Some(EffectOutcome::Applied)
                }
                EFFECT_STOP if proven_not_running(&observed) => Some(EffectOutcome::Applied),
                EFFECT_START if proven_running(&observed) => Some(EffectOutcome::Applied),
                _ => None,
            };
            match proven {
                Some(outcome) => {
                    self.repository().record_effect_outcome(
                        &operation.operation_id,
                        &effect.effect_key,
                        outcome,
                        Some(&serde_json::json!({ "reconciled": true })),
                    )?;
                    report
                        .mcp_effects_recovered
                        .push(format!("mcp:{name}:{}", effect.effect_key));
                }
                None => all_proven = false,
            }
        }
        Ok(all_proven)
    }

    /// Converges one interrupted `mcp.update` operation to its durable desired
    /// end state, then records the honest terminal outcome.
    ///
    /// Ordering is the safety property: the previous runtime must be *proven*
    /// gone before the updated configuration is applied to the runtime, so a
    /// reconcile never leaves two live clients for one record (AC-10/AC-12). The
    /// start uses the persisted record (real secrets from the DB, never the
    /// redacted journal, AC-13). A start that cannot be proven running keeps the
    /// operation in `needs_reconcile`; a start that definitively fails ends as a
    /// structured `Failed`. In neither case is the update reported complete.
    async fn reconcile_update_operation(
        &self,
        operation: &CapabilityOperation,
        report: &mut CapabilityReconcileReport,
    ) -> Result<(), CapabilityError> {
        let id = operation.operation_id.clone();
        let Some(old_name) = operation.resource_key.strip_prefix("mcp:") else {
            report.still_needs_reconcile.push(id);
            return Ok(());
        };

        // 1. Prove the outstanding stop of the previous runtime.
        let mut stop_seen = false;
        let mut stop_proven = false;
        for effect in self.repository().list_effects(&id)? {
            if effect.effect_key != EFFECT_STOP {
                continue;
            }
            stop_seen = true;
            if matches!(
                effect.outcome,
                EffectOutcome::Applied | EffectOutcome::Skipped
            ) {
                stop_proven = true;
                continue;
            }
            if proven_not_running(&self.mcp_observe(old_name).await) {
                self.repository().record_effect_outcome(
                    &id,
                    EFFECT_STOP,
                    EffectOutcome::Applied,
                    Some(&serde_json::json!({ "reconciled": true })),
                )?;
                report
                    .mcp_effects_recovered
                    .push(format!("mcp:{old_name}:mcp.stop"));
                stop_proven = true;
            }
        }
        if !(stop_seen && stop_proven) {
            // Without a proven stop the swap is not safely convergable; preserve
            // it rather than risk a second live client or a false completion.
            report.still_needs_reconcile.push(id);
            return Ok(());
        }

        // 2. Drive the runtime to the persisted (updated) record's desired state.
        let new_name = operation
            .request
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or(old_name);
        let Some(record) = self.mcp_repository().find_by_name(new_name)? else {
            // The updated record vanished before the swap could be proven; the
            // desired effect cannot be reconstructed, so preserve the operation.
            report.still_needs_reconcile.push(id);
            return Ok(());
        };

        let observed = self.mcp_observe(&record.name).await;

        if record.disabled {
            // Desired stopped. A live server under the updated name contradicts
            // the desired state and is not safely convergable here.
            if proven_running(&observed) {
                report.still_needs_reconcile.push(id);
                return Ok(());
            }
            self.complete_reconciled_update(&id, &record.name, false)?;
            return Ok(());
        }

        // Desired running with the updated configuration.
        if proven_running(&observed) {
            // Already up: record the applied start without a second effect call.
            self.repository().record_effect_outcome(
                &id,
                EFFECT_START,
                EffectOutcome::Applied,
                Some(&serde_json::json!({ "reconciled": true })),
            )?;
            self.complete_reconciled_update(&id, &record.name, true)?;
            return Ok(());
        }

        // Idempotent roll-forward: honor a prior outcome, never start twice.
        let effects = self.repository().list_effects(&id)?;
        if effects.iter().any(|effect| {
            effect.effect_key == EFFECT_START
                && matches!(
                    effect.outcome,
                    EffectOutcome::Applied | EffectOutcome::Skipped
                )
        }) {
            // A start was already proven applied but the runtime no longer shows
            // it: that is new drift beyond this update, so preserve for a human.
            report.still_needs_reconcile.push(id);
            return Ok(());
        }
        if effects
            .iter()
            .any(|effect| effect.effect_key == EFFECT_START && effect.outcome == EffectOutcome::Failed)
        {
            let error = CapabilityError::new(
                crate::capability::error::code::INTERNAL,
                "the updated MCP server failed to start during reconciliation",
            );
            self.repository().record_effect_outcome(
                &id,
                EFFECT_START,
                EffectOutcome::Failed,
                None,
            )?;
            self.finish_operation(&id, OperationState::Failed, None, Some(&error))?;
            return Ok(());
        }

        self.repository().record_effect_intent(
            &id,
            EFFECT_START,
            Some(&record.name),
            None,
        )?;
        let timing = self.mcp_timing();
        let started = tokio::time::timeout(
            timing.effect_timeout,
            self.mcp_effects().start(record.config.clone()),
        )
        .await;
        match started {
            Err(_) => {
                self.repository().record_effect_outcome(
                    &id,
                    EFFECT_START,
                    EffectOutcome::Unknown,
                    None,
                )?;
                self.repository()
                    .mark_needs_reconcile(&id, "update_reconcile_start_unconfirmed", None)?;
                report.still_needs_reconcile.push(id);
            }
            Ok(Err(error)) => {
                self.repository().record_effect_outcome(
                    &id,
                    EFFECT_START,
                    EffectOutcome::Failed,
                    None,
                )?;
                self.finish_operation(&id, OperationState::Failed, None, Some(&error))?;
            }
            Ok(Ok(())) => {
                if proven_running(&self.mcp_observe(&record.name).await) {
                    self.repository().record_effect_outcome(
                        &id,
                        EFFECT_START,
                        EffectOutcome::Applied,
                        Some(&serde_json::json!({ "reconciled": true })),
                    )?;
                    report
                        .mcp_effects_recovered
                        .push(format!("mcp:{}:mcp.start", record.name));
                    self.complete_reconciled_update(&id, &record.name, true)?;
                } else {
                    self.repository().record_effect_outcome(
                        &id,
                        EFFECT_START,
                        EffectOutcome::Unknown,
                        None,
                    )?;
                    self.repository()
                        .mark_needs_reconcile(&id, "update_reconcile_start_not_observable", None)?;
                    report.still_needs_reconcile.push(id);
                }
            }
        }
        Ok(())
    }

    /// Ends a converged update as `Completed` with an honest reconciled result.
    fn complete_reconciled_update(
        &self,
        operation_id: &str,
        name: &str,
        started: bool,
    ) -> Result<(), CapabilityError> {
        let result = serde_json::json!({
            "status": "reconciled",
            "operation": "mcp.update",
            "name": name,
            "runtime_started": started,
        });
        self.finish_operation(operation_id, OperationState::Completed, Some(&result), None)?;
        Ok(())
    }
}

/// Whether an outcome already records a proven terminal state.
fn is_terminal_outcome(outcome: EffectOutcome) -> bool {
    matches!(
        outcome,
        EffectOutcome::Applied | EffectOutcome::Skipped | EffectOutcome::Blocked | EffectOutcome::Failed
    )
}

/// Whether the runtime proves the server is up.
fn proven_running(observed: &Result<Option<ObservedMcpRuntime>, CapabilityError>) -> bool {
    matches!(
        observed,
        Ok(Some(observed)) if matches!(observed.state.as_str(), "running" | "connected")
    )
}

/// Whether the runtime proves the server is not running (an answer, not a
/// timeout). A timeout is unknown, never treated as proven stopped.
fn proven_not_running(observed: &Result<Option<ObservedMcpRuntime>, CapabilityError>) -> bool {
    match observed {
        Ok(None) => true,
        Ok(Some(observed)) => matches!(observed.state.as_str(), "stopped" | "error"),
        Err(_) => false,
    }
}

/// Whether a staging directory was modified inside the grace window.
fn entry_is_recent(path: &Path) -> bool {
    match std::fs::metadata(path).and_then(|meta| meta.modified()) {
        // If the mtime cannot be read, treat it as recent and leave it.
        Err(_) => true,
        Ok(modified) => modified.elapsed().unwrap_or(Duration::from_secs(0)) < STAGING_GRACE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::skill::checker::check_directory;
    use crate::capability::skill::installer::{SkillInstaller, TargetOutcomeStatus};
    use crate::capability::skill::plan::SkillInstallPlan;
    use crate::capability::skill::source::SkillSource;
    use crate::capability::targets::{SkillTargetId, TargetEnvironment};
    use crate::capability::types::{CapabilityKind, OperationRequest};
    use crate::db::MainStore;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        service: Arc<CapabilityApplicationService>,
        content: PathBuf,
        frozen: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        let chatspeed = temp.path().join("chatspeed-home");
        std::fs::create_dir_all(&home).expect("create home");

        let content = temp.path().join("content");
        std::fs::create_dir_all(&content).expect("create content");
        std::fs::write(content.join("SKILL.md"), "---\nname: demo\n---\n\n# demo\n")
            .expect("write skill");
        std::fs::write(content.join("notes.md"), "prose").expect("write notes");

        let store = Arc::new(MainStore::new(":memory:").expect("in-memory store"));
        let app_data = temp.path().to_path_buf();
        let frozen = temp.path().join("frozen");
        let service = Arc::new(
            CapabilityApplicationService::new(store, app_data)
                .with_environment(TargetEnvironment::injected(home, chatspeed)),
        );
        Fixture {
            service,
            content,
            frozen,
            _temp: temp,
        }
    }

    /// Begins a Skill operation and returns its id (used as the effect owner).
    async fn begin(fixture: &Fixture, kind: &str, resource_key: &str, key: &str) -> String {
        fixture
            .service
            .begin_operation(OperationRequest {
                capability: CapabilityKind::Skill,
                operation_kind: kind.to_string(),
                actor_scope: "test".to_string(),
                idempotency_key: key.to_string(),
                request: serde_json::json!({ "skill_name": "demo" }),
                resource_key: resource_key.to_string(),
            })
            .await
            .expect("begin")
            .into_operation()
            .operation_id
    }

    /// Installs the fixture skill into the ChatSpeed target and returns its path.
    async fn installed(fixture: &Fixture) -> PathBuf {
        let report = check_directory(&fixture.content).expect("check");
        let source = SkillSource::LocalDirectory {
            path: fixture.content.to_string_lossy().to_string(),
        };
        let plan = SkillInstallPlan::build(
            &source,
            &report,
            &fixture.content,
            &[SkillTargetId::Chatspeed],
            fixture.frozen.clone(),
        )
        .expect("plan");
        let installer = SkillInstaller::new(fixture.service.environment().clone());
        let op = begin(fixture, "skill.install", "skill:demo", "install-1").await;
        let summary = installer
            .apply(&plan, fixture.service.repository(), &op)
            .expect("apply");
        assert_eq!(summary.outcomes[0].status, TargetOutcomeStatus::Installed);
        PathBuf::from(summary.outcomes[0].install_path.clone().unwrap())
    }

    fn row(fixture: &Fixture) -> crate::capability::types::SkillInstallation {
        fixture
            .service
            .repository()
            .get_installation("chatspeed", "demo")
            .expect("lookup")
            .expect("row")
    }

    #[tokio::test]
    async fn finalizes_a_crashed_quarantine_and_completes_the_operation() {
        let fixture = fixture();
        let install_path = installed(&fixture).await;
        let target_root = install_path.parent().expect("parent").to_path_buf();
        let quarantine = target_root.join(".demo.cs-quarantine-crashed");
        std::fs::rename(&install_path, &quarantine).expect("simulate interrupted rename");
        let installation_id = row(&fixture).installation_id;
        fixture
            .service
            .repository()
            .set_installation_state(&installation_id, SkillInstallationState::Quarantined)
            .expect("mark quarantined");

        // The uninstall operation crashed with an unproven per-target effect.
        let op = begin(&fixture, "skill.uninstall", "skill:demo", "uninstall-1").await;
        fixture
            .service
            .record_effect_intent(&op, "skill.uninstall:chatspeed", Some("chatspeed"), None)
            .expect("intent");
        fixture
            .service
            .recover_interrupted_operations()
            .expect("recovery");

        let report = fixture.service.reconcile().await.expect("reconcile");

        assert_eq!(report.quarantines_finalized, vec!["chatspeed:demo".to_string()]);
        assert!(report.still_needs_reconcile.is_empty(), "operation must converge");
        assert!(!quarantine.exists(), "the moved-aside directory is deleted");
        assert_eq!(
            row(&fixture).state,
            SkillInstallationState::Removed,
            "the ownership row records removal"
        );
        assert_eq!(
            fixture.service.operation(&op).expect("op").state,
            OperationState::Completed,
            "a proven effect completes the needs_reconcile operation"
        );
    }

    #[tokio::test]
    async fn recovers_an_interrupted_install_whose_content_still_matches_proof() {
        let fixture = fixture();
        let install_path = installed(&fixture).await;
        let installation_id = row(&fixture).installation_id;
        fixture
            .service
            .repository()
            .set_installation_state(&installation_id, SkillInstallationState::Installing)
            .expect("mark installing");

        let report = fixture.service.reconcile().await.expect("reconcile");

        assert_eq!(report.installs_recovered, vec!["chatspeed:demo".to_string()]);
        assert!(install_path.exists(), "committed content is never deleted");
        assert_eq!(
            row(&fixture).state,
            SkillInstallationState::Installed,
            "a provable interrupted install is finalized"
        );
    }

    #[tokio::test]
    async fn leaves_a_drifted_interrupted_install_untouched() {
        let fixture = fixture();
        let install_path = installed(&fixture).await;
        // Drift the content so the manifest no longer proves the install.
        std::fs::write(install_path.join("notes.md"), "edited by hand").expect("edit");
        let installation_id = row(&fixture).installation_id;
        fixture
            .service
            .repository()
            .set_installation_state(&installation_id, SkillInstallationState::Installing)
            .expect("mark installing");

        let report = fixture.service.reconcile().await.expect("reconcile");

        assert!(
            report.installs_recovered.is_empty(),
            "drifted content is not proven, so nothing is finalized"
        );
        assert!(install_path.exists(), "drifted content is never deleted");
        assert_eq!(
            row(&fixture).state,
            SkillInstallationState::Installing,
            "an unproven install stays for the next pass"
        );
    }

    #[tokio::test]
    async fn removes_stale_staging_residue_but_keeps_recent_directories() {
        let fixture = fixture();
        let staging_root = crate::capability::staging_dir(fixture.service.app_data_dir());
        std::fs::create_dir_all(&staging_root).expect("create staging root");

        let stale = staging_root.join("stale-op-1");
        let fresh = staging_root.join("fresh-op-2");
        std::fs::create_dir_all(&stale).expect("stale dir");
        std::fs::create_dir_all(&fresh).expect("fresh dir");
        age_path_before_grace(&stale);

        let report = fixture.service.reconcile().await.expect("reconcile");

        assert_eq!(report.staging_residue_removed, 1);
        assert!(!stale.exists(), "an orphaned, aged staging tree is removed");
        assert!(fresh.exists(), "a recent staging tree may still be live and is kept");
    }

    #[test]
    fn a_no_op_reconcile_reports_nothing_changed() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let fixture = fixture();
        let report = runtime.block_on(fixture.service.reconcile()).expect("reconcile");
        assert!(report.is_noop(), "an idle journal converges nothing");
    }

    /// Moves a path's modification time to before the staging grace window so
    /// reconcile treats it as orphaned rather than possibly-live.
    #[cfg(unix)]
    fn age_path_before_grace(path: &Path) {
        let c_path = match std::ffi::CString::new(path.to_string_lossy().as_bytes()) {
            Ok(value) => value,
            // A path with an embedded NUL cannot be aged; leave it recent.
            Err(_) => return,
        };
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let times = [zero, zero];
        // Set the access and modification times to the epoch, well past the
        // grace window, using the directory file descriptor.
        let result = unsafe {
            libc::utimensat(
                libc::AT_FDCWD,
                c_path.as_ptr(),
                times.as_ptr(),
                0,
            )
        };
        assert_eq!(result, 0, "utimensat must age the staging directory");
    }

    #[cfg(not(unix))]
    fn age_path_before_grace(_path: &Path) {}
}
