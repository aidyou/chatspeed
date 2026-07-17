use std::sync::{Arc, RwLock};
use tauri::Manager;

use crate::{commands::window::quit_window, db::MainStore};

/// Create system tray menu
///
/// # Arguments
/// - `app`: The app handle
/// - `tray_id`: Optional tray id, if not provided, will use default tray id
///
/// # Returns
/// - `Result<(), String>`: A result indicating the success or failure of the operation
pub fn create_tray(app: &tauri::AppHandle, tray_id: Option<String>) -> Result<(), String> {
    let main_store = match app.try_state::<Arc<RwLock<MainStore>>>() {
        Some(store) => store,
        None => {
            log::warn!("MainStore not available yet during tray creation");
            return Err("MainStore not initialized".to_string());
        }
    };
    let (
        main_window_visible_shortcut,
        assistant_window_visible_shortcut,
        note_window_visible_shortcut,
        proxy_switcher_window_visible_shortcut,
        workflow_window_visible_shortcut,
    ) = if let Ok(c) = main_store.read() {
        (
            c.config
                .get_setting(crate::constants::CFG_MAIN_WINDOW_VISIBLE_SHORTCUT)
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    crate::shortcut::get_default_shortcut(
                        crate::constants::CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
                    )
                    .unwrap_or_default()
                    .to_string()
                }),
            c.config
                .get_setting(crate::constants::CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT)
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    crate::shortcut::get_default_shortcut(
                        crate::constants::CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
                    )
                    .unwrap_or_default()
                    .to_string()
                }),
            c.config
                .get_setting(crate::constants::CFG_NOTE_WINDOW_VISIBLE_SHORTCUT)
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    crate::shortcut::get_default_shortcut(
                        crate::constants::CFG_NOTE_WINDOW_VISIBLE_SHORTCUT,
                    )
                    .unwrap_or_default()
                    .to_string()
                }),
            c.config
                .get_setting(crate::constants::CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT)
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    crate::shortcut::get_default_shortcut(
                        crate::constants::CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT,
                    )
                    .unwrap_or_default()
                    .to_string()
                }),
            c.config
                .get_setting(crate::constants::CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT)
                .and_then(|value| value.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    crate::shortcut::get_default_shortcut(
                        crate::constants::CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT,
                    )
                    .unwrap_or_default()
                    .to_string()
                }),
        )
    } else {
        (
            crate::shortcut::get_default_shortcut(
                crate::constants::CFG_MAIN_WINDOW_VISIBLE_SHORTCUT,
            )
            .unwrap_or_default()
            .to_string(),
            crate::shortcut::get_default_shortcut(
                crate::constants::CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT,
            )
            .unwrap_or_default()
            .to_string(),
            crate::shortcut::get_default_shortcut(
                crate::constants::CFG_NOTE_WINDOW_VISIBLE_SHORTCUT,
            )
            .unwrap_or_default()
            .to_string(),
            crate::shortcut::get_default_shortcut(
                crate::constants::CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT,
            )
            .unwrap_or_default()
            .to_string(),
            crate::shortcut::get_default_shortcut(
                crate::constants::CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT,
            )
            .unwrap_or_default()
            .to_string(),
        )
    };

    let main_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "main",
        &rust_i18n::t!("tray.chat"),
        true,
        if main_window_visible_shortcut.is_empty() {
            None
        } else {
            Some(main_window_visible_shortcut)
        },
    )
    .map_err(|e| e.to_string())?;

    let assistant_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "assistant",
        &rust_i18n::t!("tray.assistant"),
        true,
        if assistant_window_visible_shortcut.is_empty() {
            None
        } else {
            Some(assistant_window_visible_shortcut)
        },
    )
    .map_err(|e| e.to_string())?;

    let workflow_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "workflow",
        &rust_i18n::t!("tray.workflow"),
        true,
        if workflow_window_visible_shortcut.is_empty() {
            None
        } else {
            Some(workflow_window_visible_shortcut)
        },
    )
    .map_err(|e| e.to_string())?;

    let note_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "note",
        &rust_i18n::t!("tray.note"),
        true,
        if note_window_visible_shortcut.is_empty() {
            None
        } else {
            Some(note_window_visible_shortcut)
        },
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

    let proxy_switcher_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "proxy_switcher",
        &rust_i18n::t!("tray.proxy_switcher"),
        true,
        if proxy_switcher_window_visible_shortcut.is_empty() {
            None
        } else {
            Some(proxy_switcher_window_visible_shortcut)
        },
    )
    .map_err(|e| e.to_string())?;

    let agent_window_menu_item = tauri::menu::MenuItem::with_id(
        app,
        "agent",
        &rust_i18n::t!("tray.agent"),
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
        .item(&workflow_window_menu_item)
        .item(&note_window_menu_item)
        .item(&proxy_switcher_window_menu_item)
        .separator()
        .item(&settings_window_menu_item)
        .item(&model_window_menu_item)
        .item(&skill_window_menu_item)
        .item(&mcp_window_menu_item)
        .item(&proxy_window_menu_item)
        .item(&agent_window_menu_item)
        .separator()
        .item(&about_window_menu_item)
        .item(&quit_item)
        .build()
        .map_err(|e| e.to_string())?;

    let tray_id = tray_id.unwrap_or(crate::constants::TRAY_ID.to_string());
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
    let menu_id = event.id().as_ref().to_string();
    match menu_id.as_str() {
        "main" | "assistant" | "workflow" => {
            crate::window::show_and_focus_window(&app, &menu_id);
        }
        "note" => {
            if let Err(e) = crate::open_note_window(app.clone()).await {
                log::error!("Failed to open note window: {}", e);
            }
        }
        "proxy_switcher" => {
            crate::window::toggle_proxy_switcher_window(app);
        }
        "settings" | "agent" | "mcp" | "model" | "proxy" | "skill" | "about" => {
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
