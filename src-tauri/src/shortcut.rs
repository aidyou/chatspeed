use lazy_static::lazy_static;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use arboard::Clipboard;
use rust_i18n::t;
use serde_json::json;
use tauri::AppHandle;

use tauri::Emitter as _;
use tauri::Manager;
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use tauri_plugin_global_shortcut::Shortcut;

use crate::constants::CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT;
use crate::constants::DEFAULT_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT;
use crate::db::MainStore;
use crate::open_note_window;
use crate::window::toggle_window_activate;
use crate::window::{activate_window, toggle_assistant_window};
use crate::{
    constants::*, CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT, CFG_CENTER_WINDOW_SHORTCUT,
    CFG_MAIN_WINDOW_VISIBLE_SHORTCUT, CFG_MOVE_WINDOW_LEFT_SHORTCUT,
    CFG_MOVE_WINDOW_RIGHT_SHORTCUT, DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
    DEFAULT_CENTER_WINDOW_SHORTCUT, DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT,
    DEFAULT_MOVE_WINDOW_LEFT_SHORTCUT, DEFAULT_MOVE_WINDOW_RIGHT_SHORTCUT,
};

/// Retrieves current shortcuts from the configuration store
///
/// # Arguments
/// * `config_store` - Reference to the configuration store containing shortcut settings
///
/// # Returns
/// Returns a HashMap containing shortcut types as keys and their corresponding shortcut strings as values.
/// If a shortcut is not set in the configuration, it will use the default value.
fn get_shortcuts(config_store: Arc<std::sync::RwLock<MainStore>>) -> HashMap<String, String> {
    let mut shortcuts = HashMap::new();

    if let Ok(c) = config_store.read() {
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

        shortcuts.insert(
            CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT.to_string(),
            c.get_config(
                CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT,
                DEFAULT_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT.to_string(),
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

        shortcuts.insert(
            CFG_MOVE_WINDOW_LEFT_SHORTCUT.to_string(),
            c.get_config(
                CFG_MOVE_WINDOW_LEFT_SHORTCUT,
                DEFAULT_MOVE_WINDOW_LEFT_SHORTCUT.to_string(),
            ),
        );

        shortcuts.insert(
            CFG_MOVE_WINDOW_RIGHT_SHORTCUT.to_string(),
            c.get_config(
                CFG_MOVE_WINDOW_RIGHT_SHORTCUT,
                DEFAULT_MOVE_WINDOW_RIGHT_SHORTCUT.to_string(),
            ),
        );

        shortcuts.insert(
            CFG_CENTER_WINDOW_SHORTCUT.to_string(),
            c.get_config(
                CFG_CENTER_WINDOW_SHORTCUT,
                DEFAULT_CENTER_WINDOW_SHORTCUT.to_string(),
            ),
        );

        shortcuts.insert(
            CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT.to_string(),
            c.get_config(
                CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT,
                DEFAULT_WORKFLOW_WINDOW_VISIBLE_SHORTCUT.to_string(),
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
            CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT.to_string(),
            DEFAULT_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
            DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_MOVE_WINDOW_LEFT_SHORTCUT.to_string(),
            DEFAULT_MOVE_WINDOW_LEFT_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_MOVE_WINDOW_RIGHT_SHORTCUT.to_string(),
            DEFAULT_MOVE_WINDOW_RIGHT_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_CENTER_WINDOW_SHORTCUT.to_string(),
            DEFAULT_CENTER_WINDOW_SHORTCUT.to_string(),
        );
        shortcuts.insert(
            CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT.to_string(),
            DEFAULT_WORKFLOW_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
    }

    shortcuts
}

lazy_static! {
    static ref LAST_CALLS: Mutex<HashMap<String, Instant>> = Mutex::new(HashMap::new());
}
const DEBOUNCE_DURATION: Duration = Duration::from_millis(200);

/// Executes the appropriate action for a given shortcut type
///
/// # Arguments
/// * `app` - Application handle for window management
/// * `shortcut_key` - The type of shortcut that was triggered
///
/// This function maps shortcut types to their corresponding window toggle actions
fn handle_shortcut(app: &AppHandle, shortcut_key: &str) {
    let mut last_calls = match LAST_CALLS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::warn!("Shortcut debounce mutex poisoned, recovering.");
            poisoned.into_inner()
        }
    };
    let now = Instant::now();

    if let Some(prev_call) = last_calls.get(shortcut_key) {
        if now.duration_since(*prev_call) < DEBOUNCE_DURATION {
            log::debug!("Debouncing shortcut: {}", shortcut_key);
            return;
        }
    }

    last_calls.insert(shortcut_key.to_string(), now);

    log::debug!("handle_shortcut: {}", shortcut_key);
    match shortcut_key {
        CFG_MAIN_WINDOW_VISIBLE_SHORTCUT => {
            toggle_window_activate(app, "main", true);
        }
        CFG_MOVE_WINDOW_LEFT_SHORTCUT => {
            if let Err(e) =
                crate::commands::window::move_window_to_screen_edge(app.clone(), "main", "left")
            {
                log::error!("Failed to move window left: {}", e);
            }
            activate_window(app, "main");
        }
        CFG_MOVE_WINDOW_RIGHT_SHORTCUT => {
            if let Err(e) =
                crate::commands::window::move_window_to_screen_edge(app.clone(), "main", "right")
            {
                log::error!("Failed to move window right: {}", e);
            }
            activate_window(app, "main");
        }
        CFG_CENTER_WINDOW_SHORTCUT => {
            if let Err(e) = crate::commands::window::center_window(app.clone(), "main") {
                log::error!("Failed to center window: {}", e);
            }
            activate_window(app, "main");
        }
        CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT => toggle_assistant_window(app),
        CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT => {
            toggle_assistant_window(app);
            // get content from paste buffer
            if let Ok(mut clipboard) = Clipboard::new().map_err(|e| e.to_string()) {
                let content = clipboard.get_text().unwrap_or_default();
                if let Err(e) = app.emit(
                    "cs://assistant-paste",
                    json!({ "windowLabel": "assistant", "content": content }),
                ) {
                    log::error!("Failed to emit cs://assistant-paste event: {}", e);
                }
            } else {
                log::error!("Failed to initialize clipboard for paste shortcut.");
            }
        }
        CFG_NOTE_WINDOW_VISIBLE_SHORTCUT => {
            let app_handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = open_note_window(app_handle).await {
                    log::error!("Failed to open note window: {}", e);
                }
            });
        }
        CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT => {
            toggle_window_activate(app, "workflow", true);
        }
        _ => {}
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
                // Alaway unregister the old shortcut before registering a new one
                if let Err(err) = shortcut_manager.unregister(hotkey.clone()) {
                    log::info!("Failed to unregister shortcut '{}': {}", shortcut, err);
                }

                log::debug!("Registering shortcut: {} for {}", shortcut, shortcut_type);
                let _ = shortcut_manager
                    .on_shortcut(hotkey, move |app_handle, _shortcut, _event| {
                        handle_shortcut(&app_handle, &shortcut_type);
                    })
                    .map_err(|e| {
                        log::error!(
                            "Error on register shortcut, shortcut:{}, error:{:?}",
                            &shortcut,
                            e
                        );
                        e
                    });
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
    let config_store = app.state::<Arc<RwLock<MainStore>>>();
    let shortcuts = get_shortcuts(config_store.inner().clone());
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

    let config_store = app.state::<Arc<RwLock<MainStore>>>();
    let shortcuts = get_shortcuts(config_store.inner().clone());
    let shortcut_manager = app.global_shortcut();
    dbg!(&shortcuts);

    // unregister old shortcut
    if let Some(old_shortcut) = shortcuts.get(shortcut_type) {
        if !old_shortcut.is_empty() {
            if let Ok(old_hotkey) = Shortcut::from_str(old_shortcut) {
                if shortcut_manager.is_registered(old_hotkey.clone()) {
                    log::debug!("Unregistering old shortcut: {}", old_shortcut);
                    if let Err(err) = shortcut_manager.unregister(old_hotkey) {
                        log::error!(
                            "Failed to unregister old shortcut '{}': {}",
                            old_shortcut,
                            err
                        );
                        return Err(t!(
                            "main.shortcut.failed_to_unregister_old",
                            error = err.to_string()
                        )
                        .into());
                    }
                } else {
                    log::debug!(
                        "Old shortcut {} for type {} was not registered or empty",
                        old_shortcut,
                        shortcut_type
                    );
                }
            }
        }
    }

    // register new shortcut
    if !new_shortcut.is_empty() {
        if let Ok(hotkey) = Shortcut::from_str(new_shortcut) {
            // Check if the new shortcut is already registered
            if shortcut_manager.is_registered(hotkey.clone()) {
                log::debug!("Unregistering existing shortcut: {}", new_shortcut);
                if let Err(err) = shortcut_manager.unregister(hotkey.clone()) {
                    log::error!("Failed to unregister shortcut '{}': {}", new_shortcut, err);
                    return Err(t!(
                        "main.shortcut.failed_to_unregister_existing",
                        error = err.to_string()
                    )
                    .into());
                }
            }

            log::debug!("Registering new shortcut: {}", new_shortcut);
            let shortcut_type = shortcut_type.to_string();

            shortcut_manager.on_shortcut(hotkey, move |app_handle, _shortcut, _event| {
                handle_shortcut(app_handle, &shortcut_type);
            })?;
        } else {
            return Err(t!("main.shortcut.invalid_format", shortcut = new_shortcut).into());
        }
    }

    Ok(())
}
