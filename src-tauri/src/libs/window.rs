use crate::db::MainStore;
use crate::{constants::*, window::WindowSize};
use log::warn;
use std::sync::{Arc, Mutex};
use tauri::{LogicalSize, WebviewWindow};

/// Applies window size configuration to a window
///
/// # Arguments
/// * `window` - The window to apply configuration to
/// * `main_store` - The main store
pub fn apply_window_config(window: &WebviewWindow, main_store: &Arc<Mutex<MainStore>>) {
    if let Ok(c) = main_store.lock() {
        let width = c.get_config(CFG_WINDOW_WIDTH, 0u32);
        let height = c.get_config(CFG_WINDOW_HEIGHT, 0u32);

        if width > 0 && height > 0 {
            if let Err(e) = window.set_size(tauri::Size::Logical(LogicalSize::new(
                width as f64,
                height as f64,
            ))) {
                warn!("Failed to set window size: {}", e);
            }
            #[cfg(debug_assertions)]
            log::debug!("Window size set to: {}x{} (logical)", width, height);
        }
        let window_clone = window.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) =
                crate::window::fix_window_visual(&window_clone, Some(WindowSize { width, height }))
                    .await
            {
                log::error!("Failed to fix window visual: {}", e);
            }
        });
    }
}
