//! Background scheduler tick for local workflow automation.
//!
//! The tick is deliberately thin: it advances the clock and hands every due slot
//! to the single canonical `automation_dispatch_due` facade path. All claim,
//! dedupe, next-slot advance and run creation happen inside that facade in one
//! durable transaction, so two ticks (or a tick racing a manual run) can never
//! produce two scheduled runs for the same slot (AC-6/INV-6).

use crate::workflow::automation::service::normalize_datetime_for_db;
use crate::workflow::react::application::WorkflowApplicationService;
use chrono::Local;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub fn spawn_workflow_automation_scheduler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;

            let now = normalize_datetime_for_db(Local::now());
            let svc = app.state::<Arc<WorkflowApplicationService>>();
            if let Err(error) = svc.automation_dispatch_due(&now).await {
                log::error!("[WorkflowAutomation][scheduler] dispatch_due failed: {error}");
            }
        }
    });
}
