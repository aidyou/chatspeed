use std::sync::{Arc, RwLock};
use tauri::Manager;

use crate::{
    commands::window::quit_window,
    constants::{
        CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT, CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
        CFG_NOTE_WINDOW_VISIBLE_SHORTCUT, DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
        DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT, DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT,
    },
    db::MainStore,
};

/// Create system tray menu
///
/// # Arguments
/// - `app`: The app handle
/// - `tray_id`: Optional tray id, if not provided, will use default tray id
///
/// # Returns
/// - `Result<(), String>`: A result indicating the success or failure of the operation
pub fn create_tray(app: &tauri::AppHandle, tray_id: Option<String>) -> Result<(), String> {
    let main_store = app.state::<Arc<RwLock<MainStore>>>();
    let mut main_window_visible_shortcut = DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string();
    let mut assistant_window_visible_shortcut =
        DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string();
    let mut note_window_visible_shortcut = DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string();
    // Get shortcut config
    if let Ok(c) = main_store.read() {
        // Main window shortcut
        main_window_visible_shortcut = c.get_config(
            CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
            DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
        assistant_window_visible_shortcut = c.get_config(
            CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
            DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
        note_window_visible_shortcut = c.get_config(
            CFG_NOTE_WINDOW_VISIBLE_SHORTCUT,
            DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT.to_string(),
        );
    }

    let main_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "main",
        &rust_i18n::t!("tray.chat"),
        true,
        Some(main_window_visible_shortcut),
    )
    .map_err(|e| e.to_string())?;

    let assistant_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "assistant",
        &rust_i18n::t!("tray.assistant"),
        true,
        Some(assistant_window_visible_shortcut),
    )
    .map_err(|e| e.to_string())?;

    let note_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "note",
        &rust_i18n::t!("tray.note"),
        true,
        Some(note_window_visible_shortcut),
    )
    .map_err(|e| e.to_string())?;

    let settings_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "settings",
        &rust_i18n::t!("tray.settings"),
        true,
        Some("CmdOrCtrl+,"),
    )
    .map_err(|e| e.to_string())?;

    let model_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "model",
        &rust_i18n::t!("tray.model"),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let skill_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "skill",
        &rust_i18n::t!("tray.skill"),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let mcp_window_menu_item =
        tauri::menu::MenuItem::with_id(app, "mcp", &rust_i18n::t!("tray.mcp"), true, None::<&str>)
            .map_err(|e| e.to_string())?;

    let proxy_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "proxy",
        &rust_i18n::t!("tray.proxy"),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let about_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "about",
        &rust_i18n::t!("tray.about"),
        true,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let quit_item = tauri::menu::MenuItem::with_id(
        app,
        "quit",
        &rust_i18n::t!("tray.quit"),
        true,
        Some("CmdOrCtrl+Q"),
    )
    .map_err(|e| e.to_string())?;

    let menu = tauri::menu::MenuBuilder::new(app)
        .item(&main_window_menu_item)
        .item(&assistant_window_menu_item)
        .item(&note_window_menu_item)
        .separator()
        .item(&settings_window_menu_item)
        .item(&model_window_menu_item)
        .item(&skill_window_menu_item)
        .item(&mcp_window_menu_item)
        .item(&proxy_window_menu_item)
        .separator()
        .item(&about_window_menu_item)
        .item(&quit_item)
        .build()
        .map_err(|e| e.to_string())?;

    let tray_id = tray_id.unwrap_or(crate::TRAY_ID.to_string());
    // Remove existing tray if exists
    if let Some(tray) = app.tray_by_id(&tray_id) {
        #[cfg(debug_assertions)]
        log::debug!(
            "Updating tray menu, current lang: {}",
            rust_i18n::locale().to_string()
        );

        tray.set_menu(Some(menu.clone()))
            .map_err(|e| e.to_string())?;
    } else {
        #[cfg(debug_assertions)]
        log::debug!(
            "Creating tray, current lang: {}",
            rust_i18n::locale().to_string()
        );

        let app_clone = app.clone();
        let tray = tauri::tray::TrayIconBuilder::with_id(&tray_id)
            .icon_as_template(true)
            .tooltip("Chatspeed")
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(move |_ic, e| {
                let app = app_clone.clone();
                tauri::async_runtime::spawn(async move {
                    handle_tray_event(&app, e).await;
                });
            })
            .build(app)
            .map_err(|e| e.to_string())?;
        if let Some(icon) = app.default_window_icon() {
            let _ = tray.set_icon(Some(icon.clone()));
        } else {
            log::warn!("No icon found for tray");
            let _ = tray.set_title(Some("Chatspeed"));
        }
    }

    Ok(())
}

/// Handle system tray events
async fn handle_tray_event(app: &tauri::AppHandle, event: tauri::menu::MenuEvent) {
    dbg!(&event);
    let menu_id = event.id().as_ref().to_string();
    match menu_id.as_str() {
        "main" | "assistant" => {
            if let Some(_) = app.get_webview_window(&menu_id) {
                crate::window::show_and_focus_window(&app, &menu_id);
                // if let Ok(is_visible) = window.is_visible() {
                //     if !is_visible {
                //         if let Err(e) = window.show() {
                //             log::error!("Failed to show {:?} window: {}", menu_id, e);
                //         }
                //     }
                // }
                // if let Err(e) = window.set_focus() {
                //     log::error!("Failed to focus {:?} window: {}", menu_id, e);
                // }
            }
        }
        "note" => {
            if let Err(e) = crate::open_note_window(app.clone()).await {
                log::error!("Failed to open note window: {}", e);
            }
        }
        "settings" | "mcp" | "model" | "proxy" | "skill" | "about" => {
            let setting_type = if menu_id.as_str() == "settings" {
                "general"
            } else {
                menu_id.as_str()
            };
            if let Err(e) =
                crate::open_setting_window(app.clone(), Some(setting_type.to_owned())).await
            {
                log::error!("Failed to open setting window: {}", e);
            }
        }
        "quit" => {
            if let Err(e) = quit_window(app.clone()) {
                log::error!("Failed to quit application: {}", e);
            }
        }
        _ => {}
    }
}
