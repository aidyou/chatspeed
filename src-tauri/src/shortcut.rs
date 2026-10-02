use lazy_static::lazy_static;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex;
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
use crate::runtime_config::{RuntimeConfigCache, RuntimeConfigSnapshot};
use crate::window::toggle_window_activate;
use crate::window::{activate_window, toggle_assistant_window};
use crate::{
    constants::*, CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT, CFG_CENTER_WINDOW_SHORTCUT,
    CFG_MAIN_WINDOW_VISIBLE_SHORTCUT, CFG_MOVE_WINDOW_LEFT_SHORTCUT,
    CFG_MOVE_WINDOW_RIGHT_SHORTCUT, CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT,
    DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT, DEFAULT_CENTER_WINDOW_SHORTCUT,
    DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT, DEFAULT_MOVE_WINDOW_LEFT_SHORTCUT,
    DEFAULT_MOVE_WINDOW_RIGHT_SHORTCUT, DEFAULT_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT,
};

/// Retrieves the current shortcuts from the runtime configuration snapshot.
///
/// # Arguments
/// * `snapshot` - The runtime configuration snapshot containing shortcut settings
///
/// # Returns
/// Returns a HashMap containing effective shortcut values used for registration.
/// Missing configuration falls back to defaults, while empty strings remain empty to indicate disabled shortcuts.
fn get_shortcuts(snapshot: &RuntimeConfigSnapshot) -> HashMap<String, String> {
    let mut shortcuts = HashMap::new();

    for shortcut_key in SHORTCUT_KEYS {
        shortcuts.insert(
            shortcut_key.to_string(),
            get_effective_shortcut(snapshot, shortcut_key),
        );
    }

    shortcuts
}

const SHORTCUT_KEYS: [&str; 9] = [
    CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
    CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
    CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT,
    CFG_NOTE_WINDOW_VISIBLE_SHORTCUT,
    CFG_MOVE_WINDOW_LEFT_SHORTCUT,
    CFG_MOVE_WINDOW_RIGHT_SHORTCUT,
    CFG_CENTER_WINDOW_SHORTCUT,
    CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT,
    CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT,
];

pub fn is_shortcut_key(key: &str) -> bool {
    SHORTCUT_KEYS.contains(&key)
}

pub fn get_default_shortcut(key: &str) -> Option<&'static str> {
    match key {
        CFG_MAIN_WINDOW_VISIBLE_SHORTCUT => Some(DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT),
        CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT => Some(DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT),
        CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT => {
            Some(DEFAULT_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT)
        }
        CFG_NOTE_WINDOW_VISIBLE_SHORTCUT => Some(DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT),
        CFG_MOVE_WINDOW_LEFT_SHORTCUT => Some(DEFAULT_MOVE_WINDOW_LEFT_SHORTCUT),
        CFG_MOVE_WINDOW_RIGHT_SHORTCUT => Some(DEFAULT_MOVE_WINDOW_RIGHT_SHORTCUT),
        CFG_CENTER_WINDOW_SHORTCUT => Some(DEFAULT_CENTER_WINDOW_SHORTCUT),
        CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT => Some(DEFAULT_WORKFLOW_WINDOW_VISIBLE_SHORTCUT),
        CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT => {
            Some(DEFAULT_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT)
        }
        _ => None,
    }
}

fn get_effective_shortcut(snapshot: &RuntimeConfigSnapshot, key: &str) -> String {
    // A value stored in the runtime configuration wins, including an explicit
    // empty string that disables the shortcut; otherwise the desktop default.
    snapshot.get_string(key, get_default_shortcut(key).unwrap_or(""))
}

lazy_static! {
    static ref LAST_CALLS: Mutex<HashMap<String, Instant>> = Mutex::new(HashMap::new());
    /// The hotkey currently bound to each shortcut type.
    ///
    /// A previous binding cannot be read back from the runtime configuration
    /// snapshot because that snapshot is refreshed only when the supervisor
    /// connects, so it can lag a just-issued update. Tracking the binding the
    /// desktop itself registered keeps an update able to unregister the old
    /// hotkey. This is desktop-local registration state, not runtime state.
    static ref REGISTERED_SHORTCUTS: Mutex<HashMap<String, String>> = Mutex::new(HashMap::new());
}
const DEBOUNCE_DURATION: Duration = Duration::from_millis(200);

/// Records the hotkey now bound to `shortcut_type`.
fn remember_shortcut(shortcut_type: &str, shortcut: &str) {
    match REGISTERED_SHORTCUTS.lock() {
        Ok(mut registered) => {
            registered.insert(shortcut_type.to_string(), shortcut.to_string());
        }
        Err(poisoned) => {
            log::warn!("Registered-shortcut mutex poisoned, recovering.");
            poisoned
                .into_inner()
                .insert(shortcut_type.to_string(), shortcut.to_string());
        }
    }
}

/// Removes and returns the recorded binding for `shortcut_type`, if any.
fn take_registered_shortcut(shortcut_type: &str) -> Option<String> {
    match REGISTERED_SHORTCUTS.lock() {
        Ok(mut registered) => registered.remove(shortcut_type),
        Err(poisoned) => poisoned.into_inner().remove(shortcut_type),
    }
}

/// Unregisters whatever hotkey is currently bound to `shortcut_type`.
fn unregister_registered_shortcut(app: &AppHandle, shortcut_type: &str) -> Result<(), String> {
    let Some(previous) = take_registered_shortcut(shortcut_type) else {
        return Ok(());
    };
    if previous.is_empty() {
        return Ok(());
    }
    let Ok(hotkey) = Shortcut::from_str(&previous) else {
        return Ok(());
    };

    let shortcut_manager = app.global_shortcut();
    if shortcut_manager.is_registered(hotkey.clone()) {
        log::debug!("Unregistering old shortcut: {}", previous);
        shortcut_manager.unregister(hotkey).map_err(|err| {
            t!(
                "main.shortcut.failed_to_unregister_old",
                error = err.to_string()
            )
            .to_string()
        })?;
    } else {
        log::debug!(
            "Old shortcut {} for type {} was not registered or empty",
            previous,
            shortcut_type
        );
    }

    Ok(())
}

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
            activate_window(app, "main");
        }
        CFG_MOVE_WINDOW_LEFT_SHORTCUT => {
            if let Err(e) =
                crate::commands::window::move_window_to_screen_edge(app.clone(), "workflow", "left")
            {
                log::error!("Failed to move workflow window left: {}", e);
            }
            activate_window(app, "workflow");
        }
        CFG_MOVE_WINDOW_RIGHT_SHORTCUT => {
            if let Err(e) = crate::commands::window::move_window_to_screen_edge(
                app.clone(),
                "workflow",
                "right",
            ) {
                log::error!("Failed to move workflow window right: {}", e);
            }
            activate_window(app, "workflow");
        }
        CFG_CENTER_WINDOW_SHORTCUT => {
            if let Err(e) = crate::commands::window::center_window(app.clone(), "workflow") {
                log::error!("Failed to center workflow window: {}", e);
            }
            activate_window(app, "workflow");
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
                if let Err(e) = crate::window::create_or_focus_note_window(app_handle).await {
                    log::error!("Failed to open note window: {}", e);
                }
            });
        }
        CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT => {
            toggle_window_activate(app, "workflow", true);
        }
        CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT => {
            crate::window::toggle_proxy_switcher_window(app);
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
        if shortcut.is_empty() {
            // An empty value disables the shortcut: drop whatever was bound for
            // this type instead of leaving a stale binding active.
            if let Err(err) = unregister_registered_shortcut(app, &shortcut_type) {
                log::error!("{}", err);
            }
            continue;
        }

        let Ok(hotkey) = Shortcut::from_str(&shortcut) else {
            log::error!("Invalid shortcut '{}' for {}", shortcut, shortcut_type);
            continue;
        };

        // Replace the binding this type previously held, then make sure the
        // requested hotkey is not still bound from an earlier registration.
        if let Err(err) = unregister_registered_shortcut(app, &shortcut_type) {
            log::error!("{}", err);
        }
        if let Err(err) = shortcut_manager.unregister(hotkey.clone()) {
            log::info!("Failed to unregister shortcut '{}': {}", shortcut, err);
        }

        log::debug!("Registering shortcut: {} for {}", shortcut, shortcut_type);
        let registered_key = shortcut_type.clone();
        match shortcut_manager.on_shortcut(hotkey, move |app_handle, _shortcut, _event| {
            handle_shortcut(&app_handle, &shortcut_type);
        }) {
            Ok(()) => remember_shortcut(&registered_key, &shortcut),
            Err(e) => log::error!(
                "Error on register shortcut, shortcut:{}, error:{:?}",
                shortcut,
                e
            ),
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
    let Some(cache) = app.try_state::<Arc<RuntimeConfigCache>>() else {
        log::warn!("Runtime config cache not found; desktop shortcuts are unavailable");
        return Ok(());
    };
    let Some(snapshot) = cache.current() else {
        // The runtime owns the shortcut configuration. Until the supervisor has
        // published a snapshot there is nothing to register, so the desktop
        // fails closed instead of reading a local database. The caller is
        // expected to retry once the snapshot arrives.
        log::warn!("Runtime configuration is not available yet; desktop shortcuts are unavailable");
        return Ok(());
    };
    let shortcuts = get_shortcuts(snapshot.as_ref());
    register_shortcuts(app, shortcuts)
}

/// Updates a specific shortcut configuration
///
/// This function:
/// 1. Unregisters the shortcut this type currently holds, if any
/// 2. Registers the new shortcut if provided
/// 3. Leaves other shortcuts untouched
///
/// The old binding is taken from the desktop's own registration record rather
/// than the runtime configuration snapshot, which is only refreshed when the
/// supervisor connects and can therefore lag a just-issued update.
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

    let shortcut_manager = app.global_shortcut();

    // unregister old shortcut
    unregister_registered_shortcut(app, shortcut_type)?;

    // register new shortcut
    if !new_shortcut.is_empty() {
        let hotkey = match Shortcut::from_str(new_shortcut) {
            Ok(hotkey) => hotkey,
            Err(_) => {
                return Err(t!("main.shortcut.invalid_format", shortcut = new_shortcut).into());
            }
        };

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
        let registered_key = shortcut_type.to_string();

        shortcut_manager.on_shortcut(hotkey, move |app_handle, _shortcut, _event| {
            handle_shortcut(app_handle, &registered_key);
        })?;
        remember_shortcut(shortcut_type, new_shortcut);
    }

    Ok(())
}
