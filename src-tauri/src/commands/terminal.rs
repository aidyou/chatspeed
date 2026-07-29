use crate::terminal::{TerminalManager, TerminalSessionMetadata, TerminalShell};
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
pub fn terminal_list_shells(
    window: Window,
    terminal_manager: State<'_, Arc<TerminalManager>>,
) -> Result<Vec<TerminalShell>, String> {
    ensure_workflow_window(&window)?;
    Ok(terminal_manager.list_shells())
}

#[tauri::command]
pub fn terminal_list_sessions(
    window: Window,
    terminal_manager: State<'_, Arc<TerminalManager>>,
) -> Result<Vec<TerminalSessionMetadata>, String> {
    ensure_workflow_window(&window)?;
    Ok(terminal_manager.list_sessions())
}

#[tauri::command]
pub fn terminal_create(
    window: Window,
    terminal_manager: State<'_, Arc<TerminalManager>>,
    cwd: Option<String>,
    shell_path: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<TerminalSessionMetadata, String> {
    ensure_workflow_window(&window)?;
    terminal_manager.create(cwd.as_deref(), shell_path.as_deref(), cols, rows)
}

#[tauri::command]
pub fn terminal_write(
    window: Window,
    terminal_manager: State<'_, Arc<TerminalManager>>,
    session_id: String,
    input: String,
) -> Result<(), String> {
    ensure_workflow_window(&window)?;
    terminal_manager.write(&session_id, &input)
}

#[tauri::command]
pub fn terminal_resize(
    window: Window,
    terminal_manager: State<'_, Arc<TerminalManager>>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    ensure_workflow_window(&window)?;
    terminal_manager.resize(&session_id, cols, rows)
}

#[tauri::command]
pub fn terminal_close(
    window: Window,
    terminal_manager: State<'_, Arc<TerminalManager>>,
    session_id: String,
) -> Result<(), String> {
    ensure_workflow_window(&window)?;
    terminal_manager.close(&session_id)
}
