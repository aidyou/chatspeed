use arboard::Clipboard;
use chatspeed_runtime_backend::error_diagnostics::{ErrorOrigin, FailureMessage};
use rust_i18n::t;

use crate::error::{AppError, Result};

/// Read text content from clipboard
#[tauri::command]
pub fn read_clipboard() -> Result<String> {
    let mut clipboard = Clipboard::new().map_err(|e| AppError::General {
        message: t!(
            "command.clipboard.failed_to_initialize",
            error = e.to_string()
        )
        .to_string()
        .into(),
    })?;
    clipboard.get_text().map_err(|e| AppError::General {
        message: FailureMessage::leaf(
            t!("command.clipboard.failed_to_read", error = e.to_string()).to_string(),
        )
        .with_origin(ErrorOrigin::from_error(&e)),
    })
}

/// Write text content to clipboard
#[tauri::command]
pub fn write_clipboard(text: String) -> Result<()> {
    let mut clipboard = Clipboard::new().map_err(|e| AppError::General {
        message: t!(
            "command.clipboard.failed_to_initialize",
            error = e.to_string()
        )
        .to_string()
        .into(),
    })?;
    clipboard.set_text(text).map_err(|e| AppError::General {
        message: FailureMessage::leaf(
            t!("command.clipboard.failed_to_write", error = e.to_string()).to_string(),
        )
        .with_origin(ErrorOrigin::from_error(&e)),
    })
}
