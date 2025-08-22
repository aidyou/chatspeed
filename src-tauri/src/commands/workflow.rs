use std::sync::{Arc, RwLock};

use tauri::{command, State};

use crate::{
    ai::interaction::chat_completion::ChatState,
    db::MainStore,
    workflow::{dag::WorkflowEngine, react::ReactExecutor},
};

#[command]
pub async fn run_dag_workflow(
    chat_state: State<'_, Arc<ChatState>>,
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    workflow_config: &str,
) -> Result<(), String> {
    let engine = WorkflowEngine::new(main_store.inner().clone(), chat_state.inner().clone())
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
pub async fn run_react_workflow(
    chat_state: State<'_, Arc<ChatState>>,
    main_store: State<'_, Arc<RwLock<MainStore>>>,
    query: &str,
) -> Result<(), String> {
    let mut exe = ReactExecutor::new(main_store.inner().clone(), chat_state.inner().clone(), 10)
        .await
        .map_err(|e| e.to_string())?;
    let plan = exe
        .execute(query.to_string())
        .await
        .map_err(|e| e.to_string())?;
    println!("Plan: {:#?}", plan);
    Ok(())
}
