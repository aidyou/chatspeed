use arboard::Clipboard;
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
        .to_string(),
    })?;
    clipboard.get_text().map_err(|e| AppError::General {
        message: t!("command.clipboard.failed_to_read", error = e.to_string()).to_string(),
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
        .to_string(),
    })?;
    clipboard.set_text(text).map_err(|e| AppError::General {
        message: t!("command.clipboard.failed_to_write", error = e.to_string()).to_string(),
    })
}
