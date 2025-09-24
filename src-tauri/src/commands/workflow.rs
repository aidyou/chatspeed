use tauri::{command, AppHandle};

use crate::workflow::dag::WorkflowEngine;

#[command]
pub async fn run_dag_workflow(app_handle: AppHandle, workflow_config: &str) -> Result<(), String> {
    let engine = WorkflowEngine::new(app_handle)
        .await
        .map_err(|e| e.to_string())?;

    let _ = engine
        .execute(workflow_config)
        .await
        .map_err(|e| e.to_string())?;

    println!(
        "Workflow execution result: {:#?}",
        engine.context.get_last_output().await
    );
    Ok(())
}
