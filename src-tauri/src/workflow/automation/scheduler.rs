use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{MainStore, WorkflowAutomation};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::automation::service::{
    advance_automation_after_scheduler_tick, normalize_datetime_for_db, run_automation_now,
};
use crate::workflow::react::gateway::TauriGateway;
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::orchestrator::SubAgentFactory;
use chrono::Local;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

fn automation_is_due(automation: &WorkflowAutomation, now: &str) -> bool {
    automation.enabled
        && automation
            .next_run_at
            .as_deref()
            .map(|next| next <= now)
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn automation(
        schedule_kind: &str,
        enabled: bool,
        next_run_at: Option<&str>,
    ) -> WorkflowAutomation {
        WorkflowAutomation {
            id: format!("automation-{}", schedule_kind),
            title: "Test automation".to_string(),
            prompt: Some("Run task".to_string()),
            prompt_file_path: None,
            agent_id: "agent-1".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: schedule_kind.to_string(),
            schedule_config: "{}".to_string(),
            continuous_context: false,
            current_workflow_session_id: None,
            self_review: false,
            enabled,
            next_run_at: next_run_at.map(str::to_string),
            last_run_at: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn scheduler_treats_due_daily_automation_as_triggerable() {
        let item = automation("daily", true, Some("2026-06-25 09:00:00"));

        assert!(automation_is_due(&item, "2026-06-25 09:00:00"));
    }

    #[test]
    fn scheduler_treats_due_interval_automation_as_triggerable() {
        let item = automation("interval", true, Some("2026-06-25 12:00:00"));

        assert!(automation_is_due(&item, "2026-06-25 12:01:00"));
    }

    #[test]
    fn scheduler_treats_due_once_automation_as_triggerable() {
        let item = automation("once", true, Some("2026-06-25 11:00:00"));

        assert!(automation_is_due(&item, "2026-06-25 11:00:00"));
    }

    #[test]
    fn scheduler_does_not_trigger_disabled_or_unscheduled_automation() {
        let disabled = automation("daily", false, Some("2026-06-25 09:00:00"));
        let unscheduled = automation("once", true, None);

        assert!(!automation_is_due(&disabled, "2026-06-25 10:00:00"));
        assert!(!automation_is_due(&unscheduled, "2026-06-25 10:00:00"));
    }
}

pub fn spawn_workflow_automation_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            let now = normalize_datetime_for_db(Local::now());
            let state = app.state::<Arc<std::sync::RwLock<MainStore>>>();
            let due_automations = {
                let Ok(store) = state.read() else {
                    log::error!("[WorkflowAutomation][scheduler] Failed to acquire store");
                    continue;
                };
                match store.list_workflow_automations() {
                    Ok(items) => items
                        .into_iter()
                        .filter(|item| automation_is_due(item, &now))
                        .collect::<Vec<_>>(),
                    Err(error) => {
                        log::error!(
                            "[WorkflowAutomation][scheduler] Failed to list automations: {}",
                            error
                        );
                        continue;
                    }
                }
            };

            for automation in due_automations {
                let state = app.state::<Arc<std::sync::RwLock<MainStore>>>();
                {
                    let Ok(store) = state.read() else {
                        log::error!("[WorkflowAutomation][scheduler] Failed to acquire store");
                        continue;
                    };
                    if let Err(error) = advance_automation_after_scheduler_tick(&store, &automation)
                    {
                        log::error!(
                            "[WorkflowAutomation][automation={}][scheduler] Failed to advance schedule: {}",
                            automation.id,
                            error
                        );
                        continue;
                    }
                }

                let result = run_automation_now(
                    app.clone(),
                    app.state::<Arc<std::sync::RwLock<MainStore>>>(),
                    app.state::<Arc<ChatState>>(),
                    app.state::<Arc<TsidGenerator>>(),
                    app.state::<Arc<TauriGateway>>(),
                    app.state::<Arc<dyn SubAgentFactory>>(),
                    app.state::<Arc<WorkflowManager>>(),
                    automation.id.clone(),
                )
                .await;

                if let Err(error) = result {
                    log::error!(
                        "[WorkflowAutomation][automation={}][scheduler] Failed to start automation: {}",
                        automation.id,
                        error
                    );
                }
            }
        }
    });
}
