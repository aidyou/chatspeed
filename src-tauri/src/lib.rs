// modules
mod ai;
mod commands;
mod constants;
mod db;
mod http;
mod libs;
mod shortcut;
mod tray;
mod updater;
mod window;
// mod plugins;
// mod workflow;
// mod snap;

use crate::constants::*;
use crate::db::MainStore;
use anyhow::anyhow;
use log::{error, warn};
use rust_i18n::{i18n, set_locale, t};
use simplelog::*;
use tray::create_tray;
// use snap::text_monitor::TextMonitorManager;

use std::fs::File;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

use tauri::Manager;

// use commands::toolbar::*;
use commands::chat::*;
use commands::clipboard::*;
use commands::download::*;
use commands::fs::*;
use commands::message::*;
use commands::note::*;
use commands::os::*;
use commands::setting::*;
use commands::window::*;
use http::server::start_http_server;
use libs::window::apply_window_config;
use libs::window_channels::WindowChannels;
use shortcut::register_desktop_shortcut;
use updater::*;

// Initialize internationalization with the "i18n" directory
// - Base directory is src-tauri/, so this will look for translations in src-tauri/i18n/
// - When using i18n! in subdirectories, use relative path, e.g., "../../../../i18n" in plugins/core/store/
i18n!("i18n", fallback = "en");

/// The entry point for the Tauri application.
///
/// This function sets up the Tauri application by initializing plugins,
/// setting up command handlers, and configuring global shortcuts.
/// It also manages the application state using `MainStore`.
///
/// # Example
///
/// The frontend can interact with the backend by invoking the following commands:
///
/// ```js
/// // Open the settings window
/// await invoke('open_setting_window');
///
/// // Get all configuration settings
/// const config = await invoke('get_all_config');
///
/// // Set a configuration value
/// await invoke('set_config', { key: 'theme', value: 'dark' });
///
/// // Manage AI models and skills
/// const aiModels = await invoke('get_all_ai_models');
/// const newModelId = await invoke('add_ai_model', { model: { name: 'GPT-4', ... } });
/// await invoke('update_ai_model', { model: { id: 1, name: 'GPT-4 Updated', ... } });
/// await invoke('delete_ai_model', { id: 1 });
/// ```
#[cfg_attr(mobile, tauri::mobile_entry_point)]

static WINDOW_READY: AtomicBool = AtomicBool::new(false);

pub async fn run() -> Result<()> {
    // let system_locale = libs::lang::get_system_locale();
    // if system_locale != "" && system_locale != "en" {
    //     set_locale(&system_locale);
    // }

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(ChatState::new(Arc::new(WindowChannels::new())))
        // Initialize the shell plugin
        .plugin(tauri_plugin_shell::init())
        // Register command handlers that can be invoked from the frontend
        .invoke_handler(tauri::generate_handler![
            // settings
            get_all_config,
            set_config,
            reload_config,
            add_ai_model,
            get_ai_model_by_id,
            get_all_ai_models,
            update_ai_model,
            update_ai_model_order,
            delete_ai_model,
            add_ai_skill,
            get_ai_skill_by_id,
            get_all_ai_skills,
            update_ai_skill,
            update_ai_skill_order,
            delete_ai_skill,
            update_shortcut,
            backup_setting,
            get_all_backups,
            restore_setting,
            update_tray,
            // clipboard
            read_clipboard,
            write_clipboard,
            // chat
            chat_with_ai,
            stop_chat,
            sync_state,
            detect_language,
            // message
            get_conversation_by_id,
            get_all_conversations,
            get_messages_for_conversation,
            add_conversation,
            update_conversation,
            delete_conversation,
            add_message,
            delete_message,
            send_message,
            // node
            get_tags,
            add_note,
            get_note,
            get_notes,
            delete_note,
            search_notes,
            // os
            get_os_info,
            // fs
            image_preview,
            // window
            open_setting_window,
            open_note_window,
            open_url,
            show_window,
            toggle_window_always_on_top,
            get_window_always_on_top,
            quit_window,
            // toolbar
            // open_screenshot_permission_settings,
            // open_text_selection_permission_settings,
            // check_text_selection_permission,
            // check_screenshot_permission,
            // start_text_monitor,
            // stop_text_monitor,

            // updater
            check_update,
            confirm_update,
            install_update,
            restart_app,
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(focused) => {
                // Hide window whenever it loses focus
                if window.label() == "toolbar" {
                    if !focused {
                        // Try to hide the window and log a warning if it fails
                        if let Err(e) = window.hide() {
                            warn!("Failed to hide toolbar window: {}", e);
                        }
                    }
                } else if window.label() == "assistant" {
                    log::debug!(
                        "Assistant window focus changed: focused={}, always_on_top={}",
                        focused,
                        ASSISTANT_ALWAYS_ON_TOP.load(Ordering::Relaxed)
                    );
                    if ASSISTANT_ALWAYS_ON_TOP.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    if !focused {
                        if let Err(e) = window.hide() {
                            warn!("Failed to hide assistant window: {}", e);
                        }
                    }
                }
            }
            // When the user clicks on the close button of a window, everything except the settings window is only hidden
            tauri::WindowEvent::CloseRequested { api, .. } => {
                match window.label() {
                    "main" => {
                        api.prevent_close();
                        if let Err(e) = window.minimize() {
                            warn!("Failed to minimize window '{}': {}", window.label(), e);
                        }
                        log::debug!("Window '{}' minimized", window.label());
                    }
                    // we just hide the main window
                    "assistant" | "toolbar" => {
                        api.prevent_close();
                        if let Err(e) = window.hide() {
                            warn!("Failed to hide window '{}': {}", window.label(), e);
                        }
                        log::debug!("Window '{}' hidden", window.label());
                    }
                    _ => {
                        log::debug!("Window '{}' closed", window.label());
                    }
                }
            }
            tauri::WindowEvent::Resized(size) => {
                if !WINDOW_READY.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                if window.label() == "main" {
                    let (old_width, old_height) =
                        get_saved_window_size(&window.state::<Arc<Mutex<MainStore>>>());
                    if (old_width != size.width || old_height != size.height)
                        && (size.width > 0 && size.height > 0)
                    {
                        // 获取当前窗口的缩放因子
                        let scale_factor = window.scale_factor().unwrap_or(1.0);
                        // 转换为逻辑尺寸
                        let logical_size = size.to_logical(scale_factor);
                        // Store the window size when the user resizes it to remember for the next startup
                        let cs = window.state::<Arc<Mutex<MainStore>>>();
                        if let Ok(mut store) = cs.clone().lock() {
                            if let Err(e) =
                                store.set_window_size(logical_size.width, logical_size.height)
                            {
                                error!("Failed to set window size: {}", e);
                            }
                        }
                    }
                }
            }
            _ => {}
        })
        // Setup the application with necessary configurations and state management
        .setup(|app| {
            // Initialize the logger
            let log_dir = app
                .path()
                .app_log_dir()
                .expect(&t!("main.failed_to_retrieve_log_directory"));
            let log_file_path = log_dir.join("chatspeed.log");
            // Ensure the log directory exists
            std::fs::create_dir_all(&log_dir).expect(&t!("main.failed_to_create_log_directory"));
            // Create the log file
            let log_file =
                File::create(&log_file_path).expect(&t!("main.failed_to_create_log_file"));
            let console_config = ConfigBuilder::new()
                .set_target_level(LevelFilter::Debug)
                .set_location_level(LevelFilter::Debug)
                .set_time_level(LevelFilter::Info)
                .build();
            let file_config = ConfigBuilder::new()
                .set_target_level(LevelFilter::Info)
                .set_location_level(LevelFilter::Info)
                .set_time_level(LevelFilter::Info)
                .build();

            CombinedLogger::init(vec![
                TermLogger::new(
                    LevelFilter::Debug,
                    console_config,
                    TerminalMode::Mixed,
                    ColorChoice::Auto,
                ),
                WriteLogger::new(LevelFilter::Info, file_config, log_file),
            ])
            .expect(&t!("main.failed_to_initialize_logger"));

            log::info!(
                "Logger initialized successfully, log file path: {:?}",
                log_file_path
            );

            // Initialize the main store
            let main_store = Arc::new(Mutex::new(MainStore::new(app.handle()).map_err(|e| {
                error!("Create main store error: {}", e);
                anyhow!(t!(
                    "main.failed_to_create_main_store",
                    error = e.to_string()
                ))
            })?));
            // Add MainStore to the app's managed state for shared access
            app.manage(main_store.clone());

            // Setup language
            if let Ok(c) = main_store.clone().lock() {
                let user_lang =
                    c.get_config(CFG_INTERFACE_LANGUAGE, libs::lang::get_system_locale());
                if !user_lang.is_empty() {
                    set_locale(&user_lang);
                    log::info!("Set interace language to {}", user_lang);
                }
            }

            // handle desktop shortcut
            #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
            {
                register_desktop_shortcut(&app.handle())?;

                // initialize text monitor
                // let monitor = Arc::new(Mutex::new(TextMonitorManager::new()?));
                // app.manage(monitor);
                // start_text_monitor(app.handle().clone(), None)?;
            }

            // Read and set the main window size from the configuration
            if let Some(main_window) = app.get_webview_window("main") {
                apply_window_config(&main_window, &main_store);
            }

            let handle = app.handle().clone();
            // Start the HTTP server using Tauri's asynchronous runtime
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_http_server(&handle).await {
                    error!("Failed to start HTTP server: {}", e);
                }
            });

            // 启动自动更新检查
            let auto_update = if let Ok(c) = main_store.clone().lock() {
                c.get_config(CFG_AUTO_UPDATE, true)
            } else {
                true
            };
            if auto_update {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let update_manager = UpdateManager::new(app_handle);

                    loop {
                        match update_manager.check_update().await {
                            Ok(Some(version_info)) => {
                                log::info!("New version {} available", version_info.version);
                                update_manager.notify_update_available(&version_info);
                            }
                            Ok(None) => log::debug!("No updates available"),
                            Err(e) => log::error!("Failed to check for updates: {}", e),
                        }

                        // 24小时检查一次
                        tokio::time::sleep(tokio::time::Duration::from_secs(24 * 60 * 60)).await;
                    }
                });
            }

            // create tray with delay
            if let Err(e) = create_tray(&app, None) {
                error!("Failed to create tray: {}", e);
            }

            // Listen for note window creation events
            let app_handle_clone = app.app_handle().clone();
            app.get_webview_window("main")
                .unwrap()
                .listen("create-note-window", move |_| {
                    let app_handle = app_handle_clone.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = crate::commands::window::create_or_focus_note_window(app_handle).await {
                            log::error!("Failed to create note window: {}", e);
                        }
                    });
                });

            WINDOW_READY.store(true, Ordering::SeqCst);

            Ok(())
        })
        // Run the Tauri application with the generated context
        .run(tauri::generate_context!())
        // Handle potential errors during the application run
        .expect(&t!("main.failed_to_start_up_application"));

    Ok(())
}

/// Get the saved window size from the configuration
///
/// # Parameters
/// - `config_store`: A reference to the configuration store.
///
/// # Returns
/// A tuple containing the saved window width and height.
fn get_saved_window_size(config_store: &Arc<Mutex<MainStore>>) -> (u32, u32) {
    if let Ok(c) = config_store.lock() {
        (
            c.get_config(CFG_WINDOW_WIDTH, 0u32),
            c.get_config(CFG_WINDOW_HEIGHT, 0u32),
        )
    } else {
        (0, 0)
    }
}

// fn setup_text_monitor(state: State<Arc<Mutex<TextMonitorManager>>>) -> Result<(), String> {
//     let monitor = state.get_mut();
//     // 在新的异步任务中处理接收到的事件
//     tauri::async_runtime::spawn(async move {
//         while let Ok(event) = rx.recv().await {
//             // 处理选中的文本
//             println!("Selected text: {}", event.text);

//             // 发送事件到前端
//             if let Err(e) = app_handle.emit("text-selected", &event) {
//                 eprintln!("Failed to emit text event: {}", e);
//             }
//         }
//     });

//     Ok(())
// }
