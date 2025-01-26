use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;

use log::error;
use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::Shortcut;

use crate::db::MainStore;
use crate::open_note_window;
use crate::window::{toggle_assistant_window, toggle_main_window};
use crate::CFG_NOTE_WINDOW_VISIBLE_SHORTCUT;
use crate::DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT;
use crate::{
    CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT, CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
    DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT, DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT,
};

/// Retrieves current shortcuts from the configuration store
///
/// # Arguments
/// * `config_store` - Reference to the configuration store containing shortcut settings
///
/// # Returns
/// Returns a HashMap containing shortcut types as keys and their corresponding shortcut strings as values.
/// If a shortcut is not set in the configuration, it will use the default value.
fn get_shortcuts(config_store: &Arc<Mutex<MainStore>>) -> HashMap<String, String> {
    let mut shortcuts = HashMap::new();

    if let Ok(c) = config_store.lock() {
        // Main window shortcut
        shortcuts.insert(
            CFG_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string(),
            c.get_config(
                CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
                DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string(),
            ),
        );

        // Assistant window shortcut
        shortcuts.insert(
            CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string(),
            c.get_config(
                CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
                DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string(),
            ),
        );

        // Note window shortcut
        shortcuts.insert(
            CFG_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
            c.get_config(
                CFG_NOTE_WINDOW_VISIBLE_SHORTCUT,
                DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
            ),
        );

        // Add new shortcuts here if needed
        // shortcuts.insert("new_window_shortcut".to_string(), c.get_config("new_window_shortcut", "default_value".to_string()));
    } else {
        shortcuts.insert(
            CFG_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string(),
            DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string(),
            DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
            DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
    }

    shortcuts
}

/// Executes the appropriate action for a given shortcut type
///
/// # Arguments
/// * `app` - Application handle for window management
/// * `shortcut_type` - The type of shortcut that was triggered
///
/// This function maps shortcut types to their corresponding window toggle actions
fn handle_shortcut(app: &AppHandle, shortcut_type: &str) {
    log::debug!("handle_shortcut: {}", shortcut_type);
    match shortcut_type {
        CFG_MAIN_WINDOW_VISIBLE_SHORTCUT => toggle_main_window(app),
        CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT => toggle_assistant_window(app),
        CFG_NOTE_WINDOW_VISIBLE_SHORTCUT => open_note_window(app.app_handle().clone()),
        // Add new shortcut handlers here
        // "new_window_shortcut" => toggle_new_window(app),
        _ => error!("Unknown shortcut type: {}", shortcut_type),
    }
}

/// Registers the provided shortcuts with the application
///
/// This function validates each shortcut string, converts valid ones to Shortcut objects,
/// and registers them with the application's global shortcut system
///
/// # Arguments
/// * `app` - Application handle for registering shortcuts
/// * `shortcuts` - HashMap containing shortcut types and their corresponding key combinations
///
/// # Returns
/// Returns Ok(()) if registration is successful, or an error if registration fails
///
fn register_shortcuts(
    app: &AppHandle,
    shortcuts: HashMap<String, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let shortcut_manager = app.global_shortcut();

    // Process all shortcuts
    for (shortcut_type, shortcut) in shortcuts {
        if !shortcut.is_empty() {
            if let Ok(hotkey) = Shortcut::from_str(&shortcut) {
                log::debug!("Registering shortcut: {} for {}", shortcut, shortcut_type);

                shortcut_manager.on_shortcut(hotkey, move |app_handle, _shortcut, _event| {
                    handle_shortcut(&app_handle, &shortcut_type);
                })?;
            } else {
                log::error!("Invalid shortcut '{}' for {}", shortcut, shortcut_type);
            }
        }
    }

    Ok(())
}

/// Registers all configured desktop shortcuts during application startup
///
/// # Arguments
/// * `app` - Application handle for shortcut registration
///
/// # Returns
/// Returns Ok(()) if registration is successful, or an error if registration fails
pub fn register_desktop_shortcut(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let config_store = app.state::<Arc<Mutex<MainStore>>>();
    let shortcuts = get_shortcuts(&config_store);
    register_shortcuts(app, shortcuts)
}

/// Updates a specific shortcut configuration
///
/// This function:
/// 1. Unregisters the old shortcut if it exists
/// 2. Registers the new shortcut if provided
/// 3. Leaves other shortcuts untouched
///
/// # Arguments
/// * `app` - Application handle for shortcut management
/// * `new_shortcut` - The new shortcut key combination
/// * `shortcut_type` - The type of shortcut to update
///
/// # Returns
/// Returns Ok(()) if the update is successful, or an error if it fails
pub fn update_shortcut(
    app: &AppHandle,
    new_shortcut: &str,
    shortcut_type: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    log::debug!(
        "Updating shortcut: type={}, new_value={}",
        shortcut_type,
        new_shortcut
    );

    let config_store = app.state::<Arc<Mutex<MainStore>>>();
    let shortcuts = get_shortcuts(&config_store);
    let shortcut_manager = app.global_shortcut();
    dbg!(&shortcuts);

    // unregister old shortcut
    if let Some(old_shortcut) = shortcuts.get(shortcut_type) {
        if !old_shortcut.is_empty() {
            if let Ok(old_hotkey) = Shortcut::from_str(old_shortcut) {
                log::debug!("Unregistering old shortcut: {}", old_shortcut);
                shortcut_manager.unregister(old_hotkey)?;
            }
        }
    }

    // register new shortcut
    if !new_shortcut.is_empty() {
        if let Ok(hotkey) = Shortcut::from_str(new_shortcut) {
            log::debug!("Registering new shortcut: {}", new_shortcut);
            let shortcut_type = shortcut_type.to_string();

            shortcut_manager.on_shortcut(hotkey, move |app_handle, _shortcut, _event| {
                handle_shortcut(app_handle, &shortcut_type);
            })?;
        } else {
            return Err(format!("Invalid shortcut format: {}", new_shortcut).into());
        }
    }

    Ok(())
}
