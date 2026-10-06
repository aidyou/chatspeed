//! Retained desktop-only Tauri automation scheduler hook (unwired).
//!
//! This was the desktop's `tauri::setup` scheduler hook that drove the local
//! automation tick through an `AppHandle`. It is not declared in `lib.rs`, so
//! the desktop no longer starts a second scheduler: the standalone runtime's
//! Tauri-free background tick (`chatspeed_runtime_backend::background`) is the
//! single automation scheduler. The file is kept in the desktop crate — the
//! only crate allowed to link Tauri — as a retained source.
//!
//! The tick is deliberately thin: it advances the clock and hands every due slot
//! to the single canonical `automation_dispatch_due` facade path.

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
