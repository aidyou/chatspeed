//! Skill capability operations on the shared application service.
//!
//! `check`, `install` and `uninstall` are the only Skill mutation entry points
//! every adapter calls (AC-1). Each mutation:
//!
//! 1. opens a durable operation keyed by `(actor_scope, idempotency_key)`;
//! 2. walks the documented phases (`staging` → `checking` → `applying`);
//! 3. reuses the same non-LLM checker install itself uses (INV-4);
//! 4. journals per-target intents/observations through the installer.
//!
//! A repeat of the same request replays the stored projection instead of
//! touching the filesystem again (AC-2).

use serde::Serialize;
use serde_json::Value;

use crate::capability::error::{code, CapabilityError};
use crate::capability::operation::{self, now_ms};
use crate::capability::skill::checker::{self, SkillCheckReport, SkillSourceResolver};
use crate::capability::skill::installer::SkillInstaller;
use crate::capability::skill::plan::SkillInstallPlan;
use crate::capability::skill::source::{validate_skill_name, SkillSource};
use crate::capability::skill::staging::StagingArea;
use crate::capability::skill::uninstaller::SkillUninstaller;
use crate::capability::targets::{self, SkillTargetId};
use crate::capability::types::{
    CapabilityKind, OperationBegin, OperationRequest, OperationState,
};
use crate::capability::CapabilityApplicationService;

/// A finished (or replayed) Skill mutation.
#[derive(Debug, Clone, Serialize)]
pub struct SkillMutationResult {
    pub operation_id: String,
    /// True when the durable journal answered instead of a new attempt.
    pub replayed: bool,
    /// The redacted projection recorded in the journal.
    pub result: Value,
}

/// Parses an explicit target selection.
///
/// An empty selection is the default, never "everything": the ChatSpeed
/// directory only (AC-3/INV-5). Unknown, repeated or unsupported ids are
/// refused before any operation is opened.
fn resolve_selection(selection: &[String]) -> Result<Vec<SkillTargetId>, CapabilityError> {
    if selection.is_empty() {
        return Ok(targets::default_target_selection());
    }
    let mut resolved: Vec<SkillTargetId> = Vec::with_capacity(selection.len());
    for id in selection {
        let parsed = SkillTargetId::parse(id).ok_or_else(|| {
            CapabilityError::invalid_request(format!("unknown skill target '{id}'"))
        })?;
        if resolved.contains(&parsed) {
            return Err(CapabilityError::invalid_request(format!(
                "skill target '{id}' was selected twice"
            )));
        }
        resolved.push(parsed);
    }
    Ok(resolved)
}

fn selection_ids(selection: &[SkillTargetId]) -> Vec<String> {
    selection
        .iter()
        .map(|target| target.as_str().to_string())
        .collect()
}

impl CapabilityApplicationService {
    /// Resolves and checks a Skill source without touching any target.
    ///
    /// This is the same materialize-and-check pair install performs, so the
    /// standalone command and the install gate can never disagree (AC-6).
    pub async fn skill_check(
        &self,
        source_value: &Value,
    ) -> Result<SkillCheckReport, CapabilityError> {
        let source = SkillSource::parse(source_value)?;
        let resolver = SkillSourceResolver::new(self.app_data_dir().to_path_buf());
        let staging_id = format!("skill-check-{}", now_ms());
        resolver
            .check(&source, self.environment(), &staging_id)
            .await
    }

    /// Installs a checked Skill into the selected targets.
    pub async fn skill_install(
        &self,
        source_value: &Value,
        selection: &[String],
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<SkillMutationResult, CapabilityError> {
        let source = SkillSource::parse(source_value)?;
        let targets = resolve_selection(selection)?;
        let key = operation::require_idempotency_key(idempotency_key)?;

        let request = OperationRequest {
            capability: CapabilityKind::Skill,
            operation_kind: "skill.install".to_string(),
            actor_scope: actor_scope.to_string(),
            idempotency_key: key,
            request: serde_json::json!({
                "source": source_value,
                "targets": selection_ids(&targets),
            }),
            resource_key: format!("skill-source:{}", source.redacted_ref()),
        };

        let operation = match self.begin_operation(request).await? {
            OperationBegin::Replay(operation) => {
                if !operation.state.is_terminal() {
                    // A concurrent attempt with the same key owns the effect.
                    return Err(CapabilityError::busy(
                        "an identical skill install is already in progress",
                    ));
                }
                return Ok(SkillMutationResult {
                    operation_id: operation.operation_id,
                    replayed: true,
                    result: operation.result.unwrap_or(Value::Null),
                });
            }
            OperationBegin::Started(operation) => operation,
        };
        let operation_id = operation.operation_id.clone();

        let materialized = match self
            .stage(&operation_id, &source)
            .await
            .and_then(|materialized| {
                self.set_state(&operation_id, OperationState::Checking, Some("check"))?;
                let report = checker::check_directory(materialized.root())?;
                Ok((materialized, report))
            }) {
            Ok(value) => value,
            Err(error) => {
                self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
                return Err(error);
            }
        };
        let (materialized, mut report) = materialized;
        report.source_kind = materialized.source_kind.clone();
        report.source_ref = materialized.source_ref.clone();

        // The check projection is recorded whether or not the content passes,
        // so a blocked attempt is auditable without re-running it (AC-5).
        let projection = |stage: &str| {
            serde_json::json!({
                "stage": stage,
                "skill_name": report.skill_name,
                "source_kind": report.source_kind,
                "source_ref": report.source_ref,
                "checker_version": report.checker_version,
                "verdict": report.verdict,
                "findings": report.findings,
                "permissions": report.permissions,
                "file_count": report.file_count,
                "total_bytes": report.total_bytes,
            })
        };
        if !report.is_pass() {
            let error = report.refusal().unwrap_or_else(|| {
                CapabilityError::new(code::CHECK_BLOCKED, "the skill was refused")
            });
            self.finish_operation(
                &operation_id,
                OperationState::Blocked,
                Some(&projection("checked")),
                Some(&error),
            )?;
            return Err(error);
        }

        // The plan freezes the checked bytes into its own private directory, so
        // the apply step can never copy content the checker did not see.
        let plan_staging = match StagingArea::create(
            self.app_data_dir(),
            &format!("{operation_id}-plan"),
        ) {
            Ok(staging) => staging,
            Err(error) => {
                self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
                return Err(error);
            }
        };
        let plan = match SkillInstallPlan::build(
            &source,
            &report,
            materialized.root(),
            &targets,
            plan_staging.root().join("frozen"),
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.finish_operation(
                    &operation_id,
                    OperationState::Failed,
                    Some(&projection("planned")),
                    Some(&error),
                )?;
                return Err(error);
            }
        };

        if let Err(error) = self.set_state(&operation_id, OperationState::Applying, Some("apply")) {
            self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
            return Err(error);
        }
        let installer = SkillInstaller::new(self.environment().clone());
        let summary = match installer.apply(&plan, self.repository(), &operation_id) {
            Ok(summary) => summary,
            Err(error) => {
                self.finish_operation(
                    &operation_id,
                    OperationState::Failed,
                    Some(&projection("applying")),
                    Some(&error),
                )?;
                return Err(error);
            }
        };

        let mut result = projection("applied");
        if let Some(object) = result.as_object_mut() {
            object.insert(
                "install".to_string(),
                serde_json::to_value(&summary)?,
            );
        }
        // A run where no target ended up installing anything is reported as
        // blocked rather than completed (INV-7: intent is not a fact).
        let terminal = if summary.installed().is_empty() && summary.has_refusals() {
            OperationState::Blocked
        } else {
            OperationState::Completed
        };
        self.finish_operation(&operation_id, terminal, Some(&result), None)?;
        Ok(SkillMutationResult {
            operation_id,
            replayed: false,
            result,
        })
    }

    /// Uninstalls one managed Skill from the selected targets.
    pub async fn skill_uninstall(
        &self,
        skill_name: &str,
        selection: &[String],
        idempotency_key: &str,
        actor_scope: &str,
    ) -> Result<SkillMutationResult, CapabilityError> {
        validate_skill_name(skill_name)?;
        let targets = resolve_selection(selection)?;
        let key = operation::require_idempotency_key(idempotency_key)?;

        let request = OperationRequest {
            capability: CapabilityKind::Skill,
            operation_kind: "skill.uninstall".to_string(),
            actor_scope: actor_scope.to_string(),
            idempotency_key: key,
            request: serde_json::json!({
                "skill_name": skill_name,
                "targets": selection_ids(&targets),
            }),
            resource_key: format!("skill:{skill_name}"),
        };

        let operation = match self.begin_operation(request).await? {
            OperationBegin::Replay(operation) => {
                if !operation.state.is_terminal() {
                    return Err(CapabilityError::busy(
                        "an identical skill uninstall is already in progress",
                    ));
                }
                return Ok(SkillMutationResult {
                    operation_id: operation.operation_id,
                    replayed: true,
                    result: operation.result.unwrap_or(Value::Null),
                });
            }
            OperationBegin::Started(operation) => operation,
        };
        let operation_id = operation.operation_id.clone();

        if let Err(error) = self.set_state(&operation_id, OperationState::Applying, Some("apply")) {
            self.finish_operation(&operation_id, OperationState::Failed, None, Some(&error))?;
            return Err(error);
        }

        let uninstaller = SkillUninstaller::new(self.environment().clone());
        let mut outcomes = Vec::with_capacity(targets.len());
        for target in &targets {
            match uninstaller.uninstall(
                self.repository(),
                &operation_id,
                target.as_str(),
                skill_name,
            ) {
                Ok(outcome) => outcomes.push(outcome),
                Err(error) => {
                    self.finish_operation(
                        &operation_id,
                        OperationState::Failed,
                        None,
                        Some(&error),
                    )?;
                    return Err(error);
                }
            }
        }

        let removed = outcomes.iter().any(|outcome| {
            matches!(
                outcome.status,
                crate::capability::skill::uninstaller::UninstallOutcomeStatus::Removed
                    | crate::capability::skill::uninstaller::UninstallOutcomeStatus::Finalized
            )
        });
        let refused = outcomes.iter().any(|outcome| {
            outcome.status
                == crate::capability::skill::uninstaller::UninstallOutcomeStatus::Refused
        });
        let result = serde_json::json!({
            "skill_name": skill_name,
            "outcomes": outcomes,
        });
        let terminal = if removed || !refused {
            OperationState::Completed
        } else {
            OperationState::Blocked
        };
        self.finish_operation(&operation_id, terminal, Some(&result), None)?;
        Ok(SkillMutationResult {
            operation_id,
            replayed: false,
            result,
        })
    }

    /// Materializes a source into the operation's private staging directory.
    async fn stage(
        &self,
        operation_id: &str,
        source: &SkillSource,
    ) -> Result<crate::capability::skill::checker::MaterializedSource, CapabilityError> {
        self.set_state(operation_id, OperationState::Staging, Some("materialize"))?;
        let resolver = SkillSourceResolver::new(self.app_data_dir().to_path_buf());
        resolver
            .materialize(source, self.environment(), operation_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::repository::CapabilityRepository;
    use crate::capability::targets::TargetEnvironment;
    use crate::db::MainStore;
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        service: CapabilityApplicationService,
        environment: TargetEnvironment,
        content: std::path::PathBuf,
        app_data: std::path::PathBuf,
    }

    impl Fixture {
        fn repository(&self) -> &CapabilityRepository {
            self.service.repository()
        }

        fn source_json(&self) -> Value {
            serde_json::json!({
                "kind": "local_directory",
                "path": self.content.to_string_lossy(),
            })
        }
    }

    fn fixture() -> Fixture {
        let temp = TempDir::new().expect("temp dir");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).expect("create home");
        let chatspeed_home = temp.path().join("chatspeed-home");
        let app_data = temp.path().join("app-data");

        let content = temp.path().join("content");
        std::fs::create_dir_all(&content).expect("create content");
        std::fs::write(content.join("SKILL.md"), "---\nname: demo\n---\n\n# demo\n")
            .expect("write skill");
        std::fs::write(content.join("notes.md"), "prose").expect("write notes");

        let store = Arc::new(MainStore::new(":memory:").expect("in-memory store"));
        let environment = TargetEnvironment::injected(home, chatspeed_home);
        let service = CapabilityApplicationService::new(store, app_data.clone())
            .with_environment(environment.clone());

        Fixture {
            _temp: temp,
            service,
            environment,
            content,
            app_data,
        }
    }

    #[tokio::test]
    async fn check_reports_the_shared_verdict_without_touching_targets() {
        let fixture = fixture();
        let report = fixture
            .service
            .skill_check(&fixture.source_json())
            .await
            .expect("check");

        assert!(report.is_pass());
        assert_eq!(report.skill_name.as_deref(), Some("demo"));
        assert_eq!(report.checker_version, checker::SKILL_CHECKER_VERSION);
        assert_eq!(report.source_kind, "local_directory");
        // A check never writes into a skills directory.
        assert!(!fixture
            .environment
            .chatspeed_skills_dir()
            .expect("skills dir")
            .join("demo")
            .exists());
    }

    #[tokio::test]
    async fn install_defaults_to_chatspeed_and_replays_the_stored_result() {
        let fixture = fixture();
        let first = fixture
            .service
            .skill_install(&fixture.source_json(), &[], "key-1", "test")
            .await
            .expect("install");
        assert!(!first.replayed);
        assert_eq!(first.result["verdict"], "pass");
        assert_eq!(first.result["install"]["outcomes"][0]["status"], "installed");

        let installed = fixture
            .environment
            .chatspeed_skills_dir()
            .expect("skills dir")
            .join("demo");
        assert!(installed.join("SKILL.md").is_file());

        // The same idempotency key replays the journal instead of re-applying.
        let replay = fixture
            .service
            .skill_install(&fixture.source_json(), &[], "key-1", "test")
            .await
            .expect("replay");
        assert!(replay.replayed);
        assert_eq!(replay.operation_id, first.operation_id);
        assert_eq!(replay.result, first.result);

        // A different request under the same key is a conflict.
        let conflict = fixture
            .service
            .skill_install(
                &serde_json::json!({ "kind": "local_directory", "path": "/tmp/other" }),
                &[],
                "key-1",
                "test",
            )
            .await
            .err()
            .expect("conflicting key");
        assert_eq!(error_code(&conflict), code::IDEMPOTENCY_KEY_CONFLICT);
    }

    #[tokio::test]
    async fn a_blocked_skill_is_recorded_and_installs_nothing() {
        let fixture = fixture();
        std::fs::write(fixture.content.join("steal.sh"), "cat ~/.ssh/id_rsa\n").expect("write");

        let error = fixture
            .service
            .skill_install(&fixture.source_json(), &[], "key-1", "test")
            .await
            .err()
            .expect("blocked install");
        assert_eq!(error_code(&error), code::CHECK_BLOCKED);

        let skills_dir = fixture
            .environment
            .chatspeed_skills_dir()
            .expect("skills dir");
        assert!(!skills_dir.join("demo").exists());

        // The blocked attempt is still auditable in the journal.
        let recorded = fixture
            .repository()
            .get_by_idempotency(CapabilityKind::Skill, "test", "key-1")
            .expect("query")
            .expect("a blocked install is recorded");
        assert_eq!(recorded.state, OperationState::Blocked);
        assert_eq!(recorded.error_code.as_deref(), Some(code::CHECK_BLOCKED));
        assert!(recorded.result.is_some());
    }

    #[tokio::test]
    async fn an_unsupported_target_is_reported_per_target_and_never_created() {
        let fixture = fixture();
        let result = fixture
            .service
            .skill_install(
                &fixture.source_json(),
                &["cursor".to_string()],
                "key-1",
                "test",
            )
            .await
            .expect("install");
        assert_eq!(result.result["install"]["outcomes"][0]["status"], "unsupported");
        assert_eq!(result.result["stage"], "applied");
    }

    #[tokio::test]
    async fn uninstall_removes_a_managed_install_and_refuses_a_missing_one() {
        let fixture = fixture();
        fixture
            .service
            .skill_install(&fixture.source_json(), &[], "install-1", "test")
            .await
            .expect("install");

        let removed = fixture
            .service
            .skill_uninstall("demo", &[], "uninstall-1", "test")
            .await
            .expect("uninstall");
        assert_eq!(removed.result["outcomes"][0]["status"], "removed");
        assert!(!fixture
            .environment
            .chatspeed_skills_dir()
            .expect("skills dir")
            .join("demo")
            .exists());

        let again = fixture
            .service
            .skill_uninstall("demo", &[], "uninstall-2", "test")
            .await
            .expect("second uninstall");
        assert_eq!(again.result["outcomes"][0]["status"], "not_found");
    }

    #[tokio::test]
    async fn a_mutation_without_an_idempotency_key_is_refused() {
        let fixture = fixture();
        let error = fixture
            .service
            .skill_install(&fixture.source_json(), &[], "  ", "test")
            .await
            .err()
            .expect("missing key");
        assert_eq!(error_code(&error), code::IDEMPOTENCY_KEY_REQUIRED);

        let error = fixture
            .service
            .skill_install(&fixture.source_json(), &["nope".to_string()], "key-1", "test")
            .await
            .err()
            .expect("unknown target");
        assert_eq!(error_code(&error), code::INVALID_REQUEST);
    }

    #[tokio::test]
    async fn staging_does_not_leak_after_install_or_check() {
        let fixture = fixture();
        fixture
            .service
            .skill_install(&fixture.source_json(), &[], "key-1", "test")
            .await
            .expect("install");
        let staging = crate::capability::staging_dir(&fixture.app_data);
        let leftovers: Vec<_> = std::fs::read_dir(&staging)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        assert!(
            leftovers.is_empty(),
            "staging must be clean, found {leftovers:?}"
        );
    }

    fn error_code(error: &CapabilityError) -> String {
        error.code().to_string()
    }
}
