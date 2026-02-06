// modules
mod ai;
mod ccproxy;
mod commands;
mod constants;
mod db;
mod environment;
pub mod error;
mod http;
mod libs;
mod logger;
mod mcp;
mod scraper;
mod search;
mod sensitive;
mod shortcut;
mod tools;
mod tray;
mod updater;
mod window;
mod workflow;

#[cfg(test)]
pub mod test;

use log::{error, warn};
use rust_i18n::{i18n, set_locale};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

use commands::updater::install_and_restart;
use tauri::async_runtime::{spawn, JoinHandle};
use tauri::Manager;

use crate::error::AppError;

// use commands::toolbar::*;
use ai::interaction::chat_completion::ChatState;
use commands::ccproxy::*;
use commands::chat::*;
use commands::clipboard::*;
use commands::dev_tool::*;
use commands::env::*;
use commands::fs::*;
use commands::mcp::*;
use commands::message::*;
use commands::note::*;
use commands::proxy_group::*;
use commands::sensitive::*;
use commands::setting::*;
use commands::window::*;
use commands::workflow::*;
use constants::*;
use db::MainStore;
use http::server::start_http_server;
use libs::window_channels::WindowChannels;
use logger::setup_logger;
use shortcut::register_desktop_shortcut;
// use tools::*;
use scraper::pool::ScraperPool;
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
static MOVE_TIMER: StdMutex<Option<JoinHandle<()>>> = StdMutex::new(None);
static LAST_MOVE: StdMutex<Option<Instant>> = StdMutex::new(None);

pub async fn run() -> crate::error::Result<()> {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&str>>,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build());

    // Only enable single instance plugin in release builds
    // This allows development and production versions to run simultaneously
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
        log::info!(
            "Another instance was started with args: {:?} and cwd: {}. Focusing existing window.",
            argv,
            cwd
        );
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.set_focus();
        }
    }));

    builder
        .plugin(tauri_plugin_process::init())
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
            // sensitive
            get_sensitive_config,
            update_sensitive_config,
            get_supported_filters,
            // clipboard
            read_clipboard,
            write_clipboard,
            // chat
            list_models,
            chat_completion,
            stop_chat,
            sync_state,
            detect_language,
            // ccproxy stats
            delete_ccproxy_stats,
            get_ccproxy_daily_stats,
            get_ccproxy_provider_stats_by_date,
            get_ccproxy_error_stats_by_date,
            get_ccproxy_model_usage_stats,
            get_ccproxy_model_token_usage_stats,
            get_ccproxy_error_distribution_stats,
            // mcp
            list_mcp_servers,
            add_mcp_server,
            update_mcp_server,
            delete_mcp_server,
            enable_mcp_server,
            disable_mcp_server,
            restart_mcp_server,
            refresh_mcp_server,
            get_mcp_server_tools,
            update_mcp_tool_status,
            // proxy group
            proxy_group_list,
            proxy_group_add,
            proxy_group_update,
            proxy_group_batch_update,
            proxy_group_delete,
            set_active_proxy_group,
            get_active_proxy_group,
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
            get_env,
            // fs
            image_preview,
            read_text_file,

            // window
            open_setting_window,
            open_note_window,
            open_url,
            show_window,
            open_proxy_switcher_window,
            toggle_window_always_on_top,
            get_window_always_on_top,
            quit_window,
            set_mouse_event_state,
            move_window_to_screen_edge,
            center_window,

            // workflow
            // run_dag_workflow,
            add_agent,
            update_agent,
            delete_agent,
            get_agent,
            get_all_agents,
            get_available_tools,
            workflow_chat_completion,
            workflow_call_tool,
            create_workflow,
            add_workflow_message,
            delete_workflow,
            get_workflow_snapshot,
            list_workflows,
            update_workflow_status,
            update_workflow_title,
            update_workflow_todo_list,

            // dev tools
            test_scrape,
            // updater
            install_and_restart,
        ])
        .plugin(tauri_plugin_opener::init())
                .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(focused) => {
                // This logic is specifically for the "assistant" window.
                if window.label() == "assistant" {
                    // When the window gains focus...
                    if *focused {
                        // It's crucial to cancel any pending hide timer.
                        // This prevents the window from disappearing after being re-focused.
                        if let Ok(mut timer) = HIDE_TIMER.lock() {
                            if let Some(handle) = timer.take() {
                                handle.abort();
                                log::debug!("Assistant window gained focus, hide timer cancelled.");
                            }
                        }
                    } else {
                        // When the window loses focus...
                        // We don't hide it immediately. We start a delayed task.
                        // This robustly handles cases like dragging, where focus is lost but the window should stay visible.
                        if let Ok(mut timer) = HIDE_TIMER.lock() {
                            // Always cancel any previous timer to ensure only one hide task is active.
                            if let Some(handle) = timer.take() {
                                handle.abort();
                            }

                            let window_clone = window.clone();
                            *timer = Some(spawn(async move {
                                #[cfg(target_os = "macos")]
                                let hide_duration = Duration::from_millis(10);

                                #[cfg(not(target_os = "macos"))]
                                let hide_duration = Duration::from_millis(200);

                                // Wait for a short duration. This delay is the key to solving the race condition.
                                // It allows the `mousedown` event from the frontend (which indicates a drag start)
                                // to be processed and set the ON_MOUSE_EVENT flag before we check it.
                                tokio::time::sleep(hide_duration).await;

                                // After the delay, we perform the definitive checks.
                                let is_pinned =
                                    ASSISTANT_ALWAYS_ON_TOP.load(Ordering::Relaxed);
                                let is_mouse_interacting =
                                    crate::constants::ON_MOUSE_EVENT.load(Ordering::Relaxed);

                                // Do not hide the window if it's pinned (always on top) or if a mouse drag is in progress.
                                if is_pinned || is_mouse_interacting {
                                    log::debug!(
                                        "Hiding assistant window cancelled. Pinned: {}, Mouse Event: {}",
                                        is_pinned,
                                        is_mouse_interacting
                                    );
                                    return;
                                }

                                // Final check: only hide if the window is still visible and has NOT regained focus during our delay.
                                if window_clone.is_visible().unwrap_or(false)
                                    && !window_clone.is_focused().unwrap_or(false)
                                {
                                    if let Err(e) = window_clone.hide() {
                                        warn!("Failed to hide assistant window: {}", e);
                                    }
                                }
                            }));
                        }
                    }
                }
            }
            // When the user clicks on the close button of a window, everything except the settings window is only hidden.
            tauri::WindowEvent::CloseRequested { api, .. } => {
                match window.label() {
                    "main" => {
                        api.prevent_close();
                        // Check if the window is valid before trying to hide it.
                        if window.is_visible().unwrap_or(false) {
                            if let Err(e) = window.hide() {
                                warn!("Failed to hide window '{}': {}", window.label(), e);
                            } else {
                                log::debug!("Window '{}' hidden", window.label());
                            }
                        } else {
                            log::debug!(
                                "Window '{}' is not visible, skipping hide",
                                window.label()
                            );
                        }
                    }
                    // For these windows, we just hide them.
                    "assistant" | "toolbar" | "workflow" => {
                        api.prevent_close();
                        // Only hide the window if it's currently visible.
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
                // Do nothing if the window is not yet fully initialized.
                if !WINDOW_READY.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let window_label = window.label();
                if window_label == "main" || window_label == "assistant" || window_label == "workflow" {
                    if let Some(config_state) = window.try_state::<Arc<RwLock<MainStore>>>() {
                        let window_size = get_saved_window_size(config_state.inner().clone(), window_label).unwrap_or_default();
                        if (window_size.width != size.width as f64
                            || window_size.height != size.height as f64)
                            && (size.width > 0 && size.height > 0)
                        {
                            // Get the current window's scale factor.
                            let scale_factor = window.scale_factor().unwrap_or(1.0);
                            // Convert physical size to logical size.
                            let logical_size = size.to_logical(scale_factor);
                            // Store the window size when the user resizes it to remember for the next startup.
                            if let Ok(mut store) = config_state.write() {
                                if let Err(e) = store.set_window_size(WindowSize {
                                    width: logical_size.width,
                                    height: logical_size.height,
                                }, window_label) {
                                    error!("Failed to set window size: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            tauri::WindowEvent::Moved(position) => {
                if !WINDOW_READY.load(Ordering::Relaxed) {
                    return;
                }

                if window.label() == "main" {
                    // Save the main window position when it is moved.
                    if let Some(config_store) = window.try_state::<Arc<RwLock<MainStore>>>() {
                        save_window_position(
                            window,
                            &config_store,
                            position,
                            get_saved_window_position,
                            |store, pos| store.save_window_position(pos),
                        );
                    }
                } else if window.label() == "workflow" {
                    // Save the workflow window position when it is moved.
                    if let Some(config_store) = window.try_state::<Arc<RwLock<MainStore>>>() {
                        save_window_position(
                            window,
                            &config_store,
                            position,
                            get_saved_workflow_window_position,
                            |store, pos| store.save_workflow_window_position(pos),
                        );
                    }
                } else if window.label() == "assistant" {
                    constants::ON_MOUSE_EVENT.store(true, Ordering::Relaxed);

                    // Update last move time, log error if mutex is poisoned
                    if let Ok(mut last_move) = LAST_MOVE.lock() {
                        *last_move = Some(Instant::now());
                    } else {
                        error!("LAST_MOVE mutex is poisoned");
                    }

                    // Cancel any pending hide timer immediately when window movement starts
                    if let Ok(mut hide_timer) = HIDE_TIMER.lock() {
                        if let Some(handle) = hide_timer.take() {
                            handle.abort();
                            log::debug!("Hide timer cancelled due to window movement");
                        }
                    }

                    // Reset the movement detection timer
                    if let Ok(mut move_timer) = MOVE_TIMER.lock() {
                        if let Some(handle) = move_timer.take() {
                            handle.abort();
                        }
                    } else {
                        error!("MOVE_TIMER mutex is poisoned");
                        return;
                    }

                    let window_clone = window.clone();
                    let new_timer = spawn(async move {
                        // Increase detection time to 1 second to accommodate actual dragging
                        tokio::time::sleep(Duration::from_secs(1)).await;

                        // Check if movement has ended
                        let movement_ended = if let Ok(last_move) = LAST_MOVE.lock() {
                            last_move.map_or(false, |t| t.elapsed() >= Duration::from_secs(1))
                        } else {
                            error!("LAST_MOVE mutex is poisoned in timer task");
                            false
                        };

                        if movement_ended {
                            constants::ON_MOUSE_EVENT.store(false, Ordering::Relaxed);
                            log::debug!("Window move ended");

                            // After movement ends, check if window should be hidden
                            if !window_clone.is_focused().unwrap_or(false)
                                && !ASSISTANT_ALWAYS_ON_TOP.load(Ordering::Relaxed) {
                                if let Err(e) = window_clone.hide() {
                                    warn!("Failed to hide window: {}", e);
                                }
                            }
                        }
                    });

                    // Store the new timer handle
                    if let Ok(mut move_timer) = MOVE_TIMER.lock() {
                        *move_timer = Some(new_timer);
                    } else {
                        error!("MOVE_TIMER mutex is poisoned when storing new timer");
                        new_timer.abort(); // Clean up the timer if we can't store it
                    }
                }
            }
            _ => {
                return;
            }
        })

        // Setup the application with necessary configurations and state management
        .setup(|app| {
            // Initialize the logger
            setup_logger(&app);

            // Initialize environment
            tauri::async_runtime::spawn(async move {
                environment::init_environment().await;
            });

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
                    .unwrap_or_else(|e| {
                        eprintln!("CRITICAL: Failed to get app data dir: {}", e);
                        std::path::PathBuf::from("./") // Fallback to current dir
                    });
                if let Err(e) = std::fs::create_dir_all(&app_local_data_dir) {
                    eprintln!("CRITICAL: Failed to create app data dir at {:?}: {}", app_local_data_dir, e);
                }
                app_local_data_dir.join("chatspeed.db")
            };

            println!("Initializing database at {:?}", db_path);
            let main_store_res = MainStore::new(db_path);
            
            let main_store = match main_store_res {
                Ok(store) => Arc::new(RwLock::new(store)),
                Err(e) => {
                    eprintln!("CRITICAL: Failed to create main store: {}", e);
                    // Create an in-memory database as fallback to prevent app from crashing immediately
                    let fallback_res = MainStore::new(":memory:");
                    match fallback_res {
                        Ok(s) => Arc::new(RwLock::new(s)),
                        Err(fe) => {
                            eprintln!("FATAL: Even in-memory DB failed: {}", fe);
                            return Err(Box::new(AppError::Db(e))); // Last resort crash
                        }
                    }
                }
            };

            // Add MainStore to the app's managed state for shared access
            app.manage(main_store.clone());

            // Setup language
            if let Ok(c) = main_store.clone().read() {
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
                let _ = register_desktop_shortcut(&app.handle()).map_err(|e|{
                    log::error!("Error on register dasktop shortcut, error:{:?}", e);
                    AppError::General{message:e.to_string()}
                });

                // initialize text monitor
                // let monitor = Arc::new(Mutex::new(TextMonitorManager::new()?));
                // app.manage(monitor);
                // start_text_monitor(app.handle().clone(), None)?;
            }

            // Setup ChatState and manage it
            let app_handle_for_chat_state = app.handle().clone();
            let chat_state = ChatState::new(
                Arc::new(WindowChannels::new()),
                Some(app_handle_for_chat_state),
                main_store.clone(),
            );
            let tm = chat_state.tool_manager.clone();
            let app_handle_for_tm = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = tm
                    .register_available_tools(app_handle_for_tm)
                    .await
                    .map_err(|e| {
                        log::error!("Failed to register available tools: {}", e);
                        AppError::Tool(e)
                    });
            });
            app.manage(chat_state.clone());

            // Initialize FilterManager with pre-compiled regexes
            let filter_manager = crate::sensitive::manager::FilterManager::new().map_err(|e| {
                error!("Failed to initialize FilterManager: {}", e);
                AppError::General {
                    message: format!("Sensitive Data Filter initialization failed: {}", e),
                }
            })?;
            app.manage(filter_manager);

            // Listen for the frontend to be ready before starting a workflow chat
            crate::workflow::helper::listen_for_workflow_ready(app.handle());

            // Initialize the scraper pool and manage it
            let scraper_pool = ScraperPool::new(app.handle().clone());
            app.manage(scraper_pool);

            // Read and set the main window size from the configuration
            if let Some(main_window) = app.get_webview_window("main") {
                restore_window_config(&main_window, main_store.clone());
            }

            if let Some(assistant_window) = app.get_webview_window("assistant") {
                restore_window_config(&assistant_window, main_store.clone());
            }

            if let Some(workflow_window) = app.get_webview_window("workflow") {
                restore_window_config(&workflow_window, main_store.clone());
            }

            let handle = app.handle().clone();
            let main_store_clone = main_store.clone();
            // Start the HTTP server using Tauri's asynchronous runtime
            let chat_state_for_http = chat_state.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = start_http_server(&handle, main_store_clone, chat_state_for_http).await.map_err(|e| AppError::Http(crate::http::error::HttpError::StartUp(e.to_string()))) {
                    error!("Failed to start HTTP server: {}", e);
                }
            });

            //===== update check =======
            // Create and manage the UpdateManager
            let update_manager = Arc::new(UpdateManager::new(app.handle().clone()));
            app.manage(update_manager.clone());

            let auto_update = if let Ok(c) = main_store.read() {
                c.get_config(CFG_AUTO_UPDATE, true)
            } else {
                true
            };

            if auto_update {
                let update_manager_clone = update_manager.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

                    loop {
                        if let Err(e) = update_manager_clone.check_and_download_update().await.map_err(|e| AppError::Updater(e)) {
                            log::error!("Failed to check for updates: {}", e);
                        }

                        // Check once every 24 hours
                        tokio::time::sleep(tokio::time::Duration::from_secs(24 * 60 * 60)).await;
                    }
                });
            }

            // create tray with delay
            let app_handle_clone = app.app_handle().clone();
            if let Err(e) = create_tray(&app_handle_clone, None).map_err(|e| AppError::General{message:e.to_string()}) {
                error!("Failed to create tray: {}", e);
            }

            // Register window creation event handlers
            window::setup_window_creation_handlers(app_handle_clone);

            WINDOW_READY.store(true, Ordering::SeqCst);

            // copy scrape resource to app data dir
            let app_handle_clone = app.app_handle().clone();
            let _ = scraper::ensure_default_configs_exist(&app_handle_clone).map_err(|e| {
                error!("Failed to ensure default scrape configs exist: {}", e);
            });

            Ok(())
        })
        // Run the Tauri application with the generated context
        .run(tauri::generate_context!()).map_err(|e| AppError::General{message:e.to_string()})?;
    Ok(())
}

/// Get the saved window size from the configuration
///
/// # Arguments
/// - `config_store`: A reference to the configuration store.
///
/// # Returns
/// A tuple containing the saved window width and height.
fn get_saved_window_size(
    config_store: Arc<RwLock<MainStore>>,
    window_label: &str,
) -> Option<WindowSize> {
    if let Ok(c) = config_store.read() {
        let key = if window_label == "main" {
            CFG_WINDOW_SIZE
        } else if window_label == "assistant" {
            CFG_ASSISTANT_WINDOW_SIZE
        } else if window_label == "workflow" {
            CFG_WORKFLOW_WINDOW_SIZE
        } else {
            return None;
        };
        c.get_config(key, Some(WindowSize::default()))
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
fn get_saved_window_position(config_store: &Arc<RwLock<MainStore>>) -> Option<MainWindowPosition> {
    if let Ok(c) = config_store.read() {
        c.get_config(CFG_WINDOW_POSITION, Some(MainWindowPosition::default()))
    } else {
        None
    }
}

/// Get the saved workflow window position from the configuration
///
/// # Arguments
/// - `config_store`: A reference to the configuration store.
///
/// # Returns
/// A tuple containing the saved window x and y positions.
fn get_saved_workflow_window_position(
    config_store: &Arc<RwLock<MainStore>>,
) -> Option<MainWindowPosition> {
    if let Ok(c) = config_store.read() {
        c.get_config(
            CFG_WORKFLOW_WINDOW_POSITION,
            Some(MainWindowPosition::default()),
        )
    } else {
        None
    }
}

/// Helper function to save window position for main and workflow windows
///
/// # Arguments
/// - `window`: The window whose position is being saved
/// - `config_store`: The configuration store
/// - `current_position`: The current position from the window event
/// - `get_saved_pos`: Function to get the saved position for this window type
/// - `save_pos`: Function to save the position for this window type
fn save_window_position<F, G>(
    window: &tauri::Window,
    config_store: &Arc<RwLock<MainStore>>,
    current_position: &tauri::PhysicalPosition<i32>,
    get_saved_pos: F,
    save_pos: G,
) where
    F: FnOnce(&Arc<RwLock<MainStore>>) -> Option<MainWindowPosition>,
    G: FnOnce(&mut MainStore, MainWindowPosition) -> std::result::Result<(), db::StoreError>,
{
    let old_pos = get_saved_pos(config_store);
    let screen_name = get_screen_name(window);

    if old_pos.map_or(true, |p| {
        screen_name != p.screen_name || current_position.x != p.x || current_position.y != p.y
    }) {
        let pos = MainWindowPosition {
            screen_name,
            x: current_position.x,
            y: current_position.y,
        };
        if let Ok(mut store) = config_store.write() {
            if let Err(e) = save_pos(&mut store, pos) {
                error!("Failed to save window position: {}", e);
            }
        }
    }
}

// fn setup_text_monitor(state: State<Arc<Mutex<TextMonitorManager>>>) -> Result<(), String> {
//     let monitor = state.get_mut();
//     // Process received events in a new async task
//     tauri::async_runtime::spawn(async move {
//         while let Ok(event) = rx.recv().await {
//             // Process selected text
//             println!("Selected text: {}", event.text);

//             // Send event to frontend
//             if let Err(e) = app_handle.emit("text-selected", &event) {
//                 eprintln!("Failed to emit text event: {}", e);
//             }
//         }
//     });

//     Ok(())
// }
