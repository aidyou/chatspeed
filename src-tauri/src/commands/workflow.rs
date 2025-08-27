use tauri::{command, AppHandle};

use crate::workflow::{dag::WorkflowEngine, react::ReactExecutor};

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

#[command]
pub async fn run_react_workflow(app_handle: AppHandle, query: &str) -> Result<(), String> {
    let mut exe = ReactExecutor::new(app_handle.clone(), 10)
        .await
        .map_err(|e| e.to_string())?;
    let plan = exe
        .execute(query.to_string())
        .await
        .map_err(|e| e.to_string())?;
    println!("Plan: {:#?}", plan);
    Ok(())
}
