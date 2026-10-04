use crate::runtime_client::RuntimeSupervisor;
use crate::runtime_terminal;
use crate::terminal::{TerminalSessionMetadata, TerminalShell};
use std::sync::Arc;
use tauri::{State, Window};

const WORKFLOW_WINDOW_LABEL: &str = "workflow";

fn is_workflow_window_label(label: &str) -> bool {
    label == WORKFLOW_WINDOW_LABEL
}

fn ensure_workflow_window(window: &Window) -> Result<(), String> {
    if is_workflow_window_label(window.label()) {
        Ok(())
    } else {
        Err("terminal_workflow_window_required".to_string())
    }
}
#[cfg(test)]
mod tests {
    use super::is_workflow_window_label;

    #[test]
    fn rejects_non_workflow_window_labels() {
        assert!(is_workflow_window_label("workflow"));
        assert!(!is_workflow_window_label("main"));
        assert!(!is_workflow_window_label("assistant"));
    }
}

#[tauri::command]
pub async fn terminal_list_shells(
    window: Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<TerminalShell>, String> {
    ensure_workflow_window(&window)?;
    runtime_terminal::list_shells(supervisor.inner()).await
}

#[tauri::command]
pub async fn terminal_list_sessions(
    window: Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<TerminalSessionMetadata>, String> {
    ensure_workflow_window(&window)?;
    runtime_terminal::list_sessions(window, supervisor.inner().clone()).await
}

#[tauri::command]
pub async fn terminal_create(
    window: Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    cwd: Option<String>,
    shell_path: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<TerminalSessionMetadata, String> {
    ensure_workflow_window(&window)?;
    runtime_terminal::create(
        window,
        supervisor.inner().clone(),
        cwd,
        shell_path,
        cols,
        rows,
    )
    .await
}

#[tauri::command]
pub async fn terminal_write(
    window: Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    session_id: String,
    input: String,
) -> Result<(), String> {
    ensure_workflow_window(&window)?;
    runtime_terminal::write(supervisor.inner(), &session_id, input).await
}

#[tauri::command]
pub async fn terminal_resize(
    window: Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    ensure_workflow_window(&window)?;
    runtime_terminal::resize(supervisor.inner(), &session_id, cols, rows).await
}

#[tauri::command]
pub async fn terminal_close(
    window: Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    session_id: String,
) -> Result<(), String> {
    ensure_workflow_window(&window)?;
    runtime_terminal::close(supervisor.inner(), &session_id).await
}
