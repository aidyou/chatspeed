// modules
mod ai;
mod commands;
mod constants;
mod db;
mod environment;
mod http;
mod libs;
mod logger;
mod mcp;
mod search;
mod shortcut;
mod test;
mod tray;
mod updater;
mod window;
mod workflow;

use anyhow::anyhow;
use log::{error, warn};
use rust_i18n::{i18n, set_locale, t};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Mutex as StdMutex;
use tokio::time::{sleep, Duration};

use tauri::async_runtime::{spawn, JoinHandle};
use tauri::Manager;

// use commands::toolbar::*;
use ai::interaction::chat_completion::ChatState;
use commands::chat::*;
use commands::clipboard::*;
use commands::fs::*;
use commands::mcp::*;
use commands::message::*;
use commands::note::*;
use commands::os::*;
use commands::setting::*;
use commands::update::*;
use commands::window::*;
use commands::workflow::*;
use constants::*;
use db::MainStore;
use http::server::start_http_server;
use libs::window_channels::WindowChannels;
use logger::setup_logger;
use shortcut::register_desktop_shortcut;
use tray::create_tray;
use updater::*;
use window::*;

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

// Define a static variable to track if the window is ready
static WINDOW_READY: AtomicBool = AtomicBool::new(false);

// Define a static variable outside the run function to store the timer handle
static HIDE_TIMER: StdMutex<Option<JoinHandle<()>>> = StdMutex::new(None);

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
        // .manage(Arc::new(ChatState::new(Arc::new(WindowChannels::new())))) // move chat state register to setup scope
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
            list_models,
            chat_completion,
            stop_chat,
            sync_state,
            detect_language,
            deep_search,
            stop_deep_search,
            // mcp
            list_mcp_servers,
            add_mcp_server,
            update_mcp_server,
            delete_mcp_server,
            enable_mcp_server,
            disable_mcp_server,
            restart_mcp_server,
            get_mcp_server_tools,
            update_mcp_tool_status,
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
            update_message_metadata,
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
            // workflow
            run_dag_workflow,
            run_react_workflow,
        ])
        .plugin(tauri_plugin_opener::init())
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
                        let window_clone = window.clone();
                        // Cancel the previous timer (if any)
                        if let Ok(mut timer) = HIDE_TIMER.lock() {
                            if let Some(handle) = timer.take() {
                                handle.abort();
                            }
                            // Create a new timer
                            *timer = Some(spawn(async move {
                                sleep(Duration::from_millis(100)).await;
                                // 在隐藏窗口前检查窗口是否可见
                                if window_clone
                                    .is_visible()
                                    .map_err(|e| warn!("Failed to check window visibility: {}", e))
                                    .unwrap_or(false)
                                {
                                    if let Err(e) = window_clone.hide() {
                                        warn!("Failed to hide assistant window: {}", e);
                                    }
                                }
                            }));
                        }
                    } else {
                        // Cancel the timer when the window gains focus
                        if let Ok(mut timer) = HIDE_TIMER.lock() {
                            if let Some(handle) = timer.take() {
                                handle.abort();
                            }
                        }
                    }
                }
            }
            // When the user clicks on the close button of a window, everything except the settings window is only hidden
            tauri::WindowEvent::CloseRequested { api, .. } => {
                match window.label() {
                    "main" => {
                        api.prevent_close();
                        // 检查窗口是否有效，然后再尝试最小化
                        if window.is_visible().unwrap_or(false) {
                            if let Err(e) = window.minimize() {
                                warn!("Failed to minimize window '{}': {}", window.label(), e);
                            } else {
                                log::debug!("Window '{}' minimized", window.label());
                            }
                        } else {
                            log::debug!(
                                "Window '{}' is not visible, skipping minimize",
                                window.label()
                            );
                        }
                    }
                    // we just hide the main window
                    "assistant" | "toolbar" => {
                        api.prevent_close();
                        // 检查窗口是否可见，只有可见的窗口才需要隐藏
                        if window.is_visible().unwrap_or(false) {
                            if let Err(e) = window.hide() {
                                warn!("Failed to hide window '{}': {}", window.label(), e);
                            } else {
                                log::debug!("Window '{}' hidden", window.label());
                            }
                        } else {
                            log::debug!("Window '{}' is already hidden", window.label());
                        }
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
                    let config_state = window.state::<Arc<Mutex<MainStore>>>();
                    let window_size = get_saved_window_size(&config_state).unwrap_or_default();
                    if (window_size.width != size.width as f64
                        || window_size.height != size.height as f64)
                        && (size.width > 0 && size.height > 0)
                    {
                        // 获取当前窗口的缩放因子
                        let scale_factor = window.scale_factor().unwrap_or(1.0);
                        // 转换为逻辑尺寸
                        let logical_size = size.to_logical(scale_factor);
                        // Store the window size when the user resizes it to remember for the next startup
                        if let Ok(mut store) = config_state.lock() {
                            if let Err(e) = store.set_window_size(WindowSize {
                                width: logical_size.width,
                                height: logical_size.height,
                            }) {
                                error!("Failed to set window size: {}", e);
                            }
                        }
                    }
                }
            }
            tauri::WindowEvent::Moved(position) => {
                if !WINDOW_READY.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                if window.label() == "main" {
                    let config_store = &window.state::<Arc<Mutex<MainStore>>>();
                    let old_pos = get_saved_window_position(config_store);
                    let screen_name = get_screen_name(&window);

                    if old_pos.map_or(true, |p| {
                        screen_name != p.screen_name || position.x != p.x || position.y != p.y
                    }) {
                        let pos = MainWindowPosition {
                            screen_name,
                            x: position.x,
                            y: position.y,
                        };
                        if let Ok(mut store) = config_store.lock() {
                            if let Err(e) = store.save_window_position(pos) {
                                error!("Failed to set window position: {}", e);
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
            setup_logger(&app);

            // Initialize environment
            environment::init_environment();

            // Initialize the main store
            #[cfg(debug_assertions)]
            let db_path = {
                let dev_dir = &*crate::STORE_DIR.read();
                dev_dir.join("chatspeed.db")
            };

            #[cfg(not(debug_assertions))]
            let db_path = {
                let app_local_data_dir = app
                    .path()
                    .app_data_dir()
                    .expect(t!("db.failed_to_get_app_data_dir").to_string().as_str());
                std::fs::create_dir_all(&app_local_data_dir)
                    .map_err(|e| db::StoreError::StringError(e.to_string()))?;
                app_local_data_dir.join("chatspeed.db")
            };
            let main_store = Arc::new(Mutex::new(MainStore::new(db_path).map_err(|e| {
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

            // Setup ChatState and manage it
            let app_handle_for_chat_state = app.handle().clone();
            let chat_state = ChatState::new_with_apphandle(
                Arc::new(WindowChannels::new()),
                Some(app_handle_for_chat_state),
            );
            let tm = chat_state.tool_manager.clone();
            let chat_state_clone = chat_state.clone();
            let main_store_clone = main_store.clone();
            tauri::async_runtime::spawn(async move {
                let _ = tm
                    .register_available_tools(chat_state_clone, main_store_clone)
                    .await
                    .map_err(|e| {
                        log::error!("Failed to register available tools: {}", e);
                    });
            });
            app.manage(chat_state.clone());

            // Read and set the main window size from the configuration
            if let Some(main_window) = app.get_webview_window("main") {
                restore_window_config(&main_window, &main_store.clone());
            }

            let handle = app.handle().clone();
            let main_store_clone = main_store.clone();
            // Start the HTTP server using Tauri's asynchronous runtime
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_http_server(&handle, main_store_clone).await {
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
            let app_handle_clone = app.app_handle().clone();
            if let Err(e) = create_tray(&app_handle_clone, None) {
                error!("Failed to create tray: {}", e);
            }

            // Register window creation event handlers
            window::setup_window_creation_handlers(app_handle_clone);

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
/// # Arguments
/// - `config_store`: A reference to the configuration store.
///
/// # Returns
/// A tuple containing the saved window width and height.
fn get_saved_window_size(config_store: &Arc<Mutex<MainStore>>) -> Option<WindowSize> {
    if let Ok(c) = config_store.lock() {
        c.get_config(CFG_WINDOW_SIZE, Some(WindowSize::default()))
    } else {
        None
    }
}

/// Get the saved window position from the configuration
///
/// # Arguments
/// - `config_store`: A reference to the configuration store.
///
/// # Returns
/// A tuple containing the saved window x and y positions.
fn get_saved_window_position(config_store: &Arc<Mutex<MainStore>>) -> Option<MainWindowPosition> {
    if let Ok(c) = config_store.lock() {
        c.get_config(CFG_WINDOW_POSITION, Some(MainWindowPosition::default()))
    } else {
        None
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
