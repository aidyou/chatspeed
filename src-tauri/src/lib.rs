mod ai;
mod builtin_agents;
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
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex as StdMutex;
use std::sync::RwLock;
use std::time::Duration;
use std::time::Instant;

use tauri::async_runtime::{spawn, JoinHandle};
use tauri::Manager;

// use commands::toolbar::*;
use crate::error::AppError;
use ai::interaction::chat_completion::ChatState;
use commands::agent::*;
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
use commands::updater::install_and_restart;
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

// Store auto-hide timers by window label so assistant and proxy switcher do not interfere.
static HIDE_TIMERS: LazyLock<StdMutex<HashMap<String, JoinHandle<()>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
static MOVE_TIMERS: LazyLock<StdMutex<HashMap<String, JoinHandle<()>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
static LAST_MOVES: LazyLock<StdMutex<HashMap<String, Instant>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

fn should_auto_hide_on_focus_loss(label: &str) -> bool {
    matches!(label, "assistant" | "proxy_switcher")
}

fn should_preserve_visibility_while_dragging(label: &str) -> bool {
    matches!(label, "assistant" | "proxy_switcher")
}

fn should_keep_window_visible(label: &str) -> bool {
    match label {
        "assistant" => {
            ASSISTANT_ALWAYS_ON_TOP.load(Ordering::Relaxed)
                || crate::constants::ON_MOUSE_EVENT.load(Ordering::Relaxed)
        }
        _ => false,
    }
}

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
            // agent command
            add_agent,
            update_agent,
            delete_agent,
            get_agent,
            get_all_agents,
            update_agent_order,
            get_available_tools,
            get_default_shell_policy,
            get_default_image_recognition_prompt,

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
            get_sensitive_status,
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
            get_ccproxy_grouped_stats,
            get_ccproxy_provider_stats_by_date,
            get_ccproxy_error_stats_by_date,
            get_ccproxy_model_usage_stats,
            get_ccproxy_model_token_usage_stats,
            get_ccproxy_error_distribution_stats,
            get_ccproxy_provider_token_usage_stats,
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
            image_source_url,
            read_text_file,
            read_git_base_text_file,
            get_git_status,
            list_dir,
            open_path_in_file_manager,

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
            add_workflow_message,
            create_workflow,
            delete_last_workflow_message,
            delete_workflow,
            get_system_skills,
            get_workflow_agent_config,
            get_workflow_snapshot,
            get_workflow_session_key,
            list_workflows,
            search_workspace_files,
            update_workflow_allowed_paths,
            update_workflow_final_audit,
            update_workflow_auto_compress,
            update_workflow_approval_level,
            update_workflow_model_config,
            update_workflow_skills_config,
            update_workflow_phase,
            update_workflow_agent_config,
            update_workflow_agent_id,
            get_auto_approved_tools,
            remove_auto_approved_tool,
            remove_shell_policy_item,
            update_workflow_status,
            update_workflow_title,
            update_workflow_title_and_query,
            update_workflow_query,
            update_workflow_todo_list,
            workflow_approve_plan,
            workflow_get_tasks,
            workflow_signal,
            workflow_start,
            workflow_stop,
            get_workflow_events,
            get_workflow_dispatcher_metrics,
            get_workflow_efficiency_report,
            get_workflow_memory_diagnostics,

            // dev tools
            test_scrape,
            // updater
            install_and_restart,
        ])
        .plugin(tauri_plugin_opener::init())
                .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(focused) => {
                let label = window.label().to_string();
                if should_auto_hide_on_focus_loss(&label) {
                    if *focused {
                        if let Ok(mut timers) = HIDE_TIMERS.lock() {
                            if let Some(handle) = timers.remove(&label) {
                                handle.abort();
                                log::debug!("Window '{}' gained focus, hide timer cancelled.", label);
                            }
                        }
                    } else if let Ok(mut timers) = HIDE_TIMERS.lock() {
                        if let Some(handle) = timers.remove(&label) {
                            handle.abort();
                        }

                        let window_clone = window.clone();
                        let label_clone = label.clone();
                        let timer = spawn(async move {
                            #[cfg(target_os = "macos")]
                            let hide_duration = Duration::from_millis(10);

                            #[cfg(not(target_os = "macos"))]
                            let hide_duration = Duration::from_millis(200);

                            tokio::time::sleep(hide_duration).await;

                            if should_keep_window_visible(&label_clone) {
                                log::debug!(
                                    "Hiding window '{}' cancelled because it should remain visible.",
                                    label_clone
                                );
                                return;
                            }

                            if window_clone.is_visible().unwrap_or(false)
                                && !window_clone.is_focused().unwrap_or(false)
                            {
                                if let Err(e) = window_clone.hide() {
                                    warn!("Failed to hide window '{}': {}", label_clone, e);
                                }
                            }
                        });
                        timers.insert(label, timer);
                    }
                }
            }
            // When the user clicks on the close button, main/assistant/workflow are hidden.
            // The settings window is briefly hidden and then force-destroyed on macOS to avoid
            // WKWebView close-time layer tree races while still releasing resources.
            tauri::WindowEvent::CloseRequested { api, .. } => {
                match window.label() {
                    // For these windows, we just hide them.
                    "main" | "assistant" | "workflow" => {
                        api.prevent_close();
                        // Check if the window is valid before trying to hide it.
                        if window.is_visible().unwrap_or(false) {
                            if let Err(e) = window.hide() {
                                warn!("Failed to hide window '{}': {}", window.label(), e);
                            } else {
                                log::debug!("Window '{}' hidden", window.label());
                            }
                        } else {
                            #[cfg(debug_assertions)]
                            log::debug!("Window '{}' is already hidden", window.label());
                        }
                    }
                    "settings" | "note" => {
                        api.prevent_close();

                        if window.is_visible().unwrap_or(false) {
                            if let Err(e) = window.hide() {
                                warn!("Failed to hide window '{}': {}", window.label(), e);
                            }
                        }

                        let app_handle = window.app_handle().clone();
                        spawn(async move {
                            #[cfg(target_os = "macos")]
                            let destroy_delay = Duration::from_millis(120);

                            #[cfg(not(target_os = "macos"))]
                            let destroy_delay = Duration::from_millis(0);

                            tokio::time::sleep(destroy_delay).await;

                            if let Some(settings_window) = app_handle.get_webview_window("settings")
                            {
                                if settings_window.is_visible().unwrap_or(false) {
                                    log::debug!(
                                        "Skip destroying settings window because it became visible again"
                                    );
                                    return;
                                }

                                if let Err(e) = settings_window.destroy() {
                                    warn!("Failed to destroy window 'settings': {}", e);
                                } else {
                                    log::debug!("Window 'settings' destroyed after delayed hide");
                                }
                            }
                        });
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
                if window_label == "main" || window_label == "assistant" ||
                    window_label == "workflow" || window_label == "proxy_switcher" {
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
                } else if should_preserve_visibility_while_dragging(window.label()) {
                    let label = window.label().to_string();

                    if label == "assistant" {
                        constants::ON_MOUSE_EVENT.store(true, Ordering::Relaxed);
                    }

                    if let Ok(mut last_moves) = LAST_MOVES.lock() {
                        last_moves.insert(label.clone(), Instant::now());
                    } else {
                        error!("LAST_MOVES mutex is poisoned");
                    }

                    if let Ok(mut hide_timers) = HIDE_TIMERS.lock() {
                        if let Some(handle) = hide_timers.remove(&label) {
                            handle.abort();
                            log::debug!("Hide timer for '{}' cancelled due to window movement", label);
                        }
                    }

                    if let Ok(mut move_timers) = MOVE_TIMERS.lock() {
                        if let Some(handle) = move_timers.remove(&label) {
                            handle.abort();
                        }
                    } else {
                        error!("MOVE_TIMERS mutex is poisoned");
                        return;
                    }

                    let window_clone = window.clone();
                    let label_clone = label.clone();
                    let new_timer = spawn(async move {
                        tokio::time::sleep(Duration::from_secs(1)).await;

                        let movement_ended = if let Ok(last_moves) = LAST_MOVES.lock() {
                            last_moves
                                .get(&label_clone)
                                .map_or(false, |t| t.elapsed() >= Duration::from_secs(1))
                        } else {
                            error!("LAST_MOVES mutex is poisoned in timer task");
                            false
                        };

                        if movement_ended {
                            if label_clone == "assistant" {
                                constants::ON_MOUSE_EVENT.store(false, Ordering::Relaxed);
                            }
                            log::debug!("Window '{}' move ended", label_clone);

                            if !window_clone.is_focused().unwrap_or(false)
                                && !should_keep_window_visible(&label_clone)
                            {
                                if let Err(e) = window_clone.hide() {
                                    warn!("Failed to hide window '{}': {}", label_clone, e);
                                }
                            }
                        }
                    });

                    if let Ok(mut move_timers) = MOVE_TIMERS.lock() {
                        move_timers.insert(label, new_timer);
                    } else {
                        error!("MOVE_TIMERS mutex is poisoned when storing new timer");
                        new_timer.abort();
                    }
                }
            }
            _ => {
                return;
            }
        })

        // Setup the application with necessary configurations and state management
        .setup(|app| {
            // Initialize the logger - this is critical and must stay here
            setup_logger(&app);

            // Initialize RESOURCE_DIR for production
            #[cfg(not(debug_assertions))]
            {
                if let Ok(res_path) = app.path().resource_dir() {
                    *crate::RESOURCE_DIR.write() = res_path;
                    log::info!("RESOURCE_DIR initialized at: {:?}", *crate::RESOURCE_DIR.read());
                }
            }

            // Initialize the main store
            #[cfg(debug_assertions)]
            let db_path = {
                let dev_dir = &*crate::STORE_DIR.read();
                dev_dir.join("chatspeed.db")
            };

            #[cfg(not(debug_assertions))]
            let db_path = {
                let app_local_data_dir = app.path().app_data_dir().unwrap_or_else(|e| {
                    eprintln!("CRITICAL: Failed to get app data dir: {}", e);
                    std::path::PathBuf::from("./") // Fallback to current dir
                });
                if let Err(e) = std::fs::create_dir_all(&app_local_data_dir) {
                    eprintln!(
                        "CRITICAL: Failed to create app data dir at {:?}: {}",
                        app_local_data_dir, e
                    );
                }
                app_local_data_dir.join("chatspeed.db")
            };

            println!("========================================");
            println!("Initializing database at: {:?}", db_path);
            println!("Platform: {}", std::env::consts::OS);
            println!("========================================");

            let main_store_res = MainStore::new(&db_path);

            let main_store = match main_store_res {
                Ok(store) => {
                    println!("✓ Database initialized successfully");
                    Arc::new(RwLock::new(store))
                },
                Err(e) => {
                    eprintln!("========================================");
                    eprintln!("CRITICAL: Failed to create main store: {}", e);
                    eprintln!("Database path: {:?}", db_path);
                    eprintln!("Parent directory exists: {}", db_path.parent().map_or(false, |p| p.exists()));
                    eprintln!("Attempting fallback to in-memory database...");
                    eprintln!("========================================");

                    // Create an in-memory database as fallback to prevent app from crashing immediately
                    let fallback_res = MainStore::new(":memory:");
                    match fallback_res {
                        Ok(s) => {
                            eprintln!("WARNING: Using in-memory database. All data will be lost on exit!");
                            Arc::new(RwLock::new(s))
                        },
                        Err(fe) => {
                            eprintln!("========================================");
                            eprintln!("FATAL: Even in-memory DB failed: {}", fe);
                            eprintln!("The application cannot continue.");
                            eprintln!("========================================");
                            return Err(Box::new(AppError::Db(e))); // Last resort crash
                        }
                    }
                }
            };

            // CRITICAL: Add MainStore to managed state IMMEDIATELY, before any other initialization
            // This must be done first because windows may be created concurrently during setup,
            // and frontend code may call commands that require MainStore state before setup completes.
            // See: https://github.com/tauri-apps/tauri/issues/xxxx (race condition with window creation)
            app.manage(main_store.clone());

            if let Err(e) =
                builtin_agents::sync_builtin_agents_if_needed(&app.handle(), main_store.clone())
            {
                log::error!("Failed to synchronize built-in agents: {}", e);
            }

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
                if let Err(e) = register_desktop_shortcut(&app.handle()) {
                    log::error!("Error on register desktop shortcut, error: {:?}", e);
                }
            }

            // === STATE REGISTRATION SECTION ===
            // IMPORTANT: The order of state registration matters!
            // States must be registered before any code (including event listeners) tries to access them.
            // See: docs/STATE_MANAGEMENT_REVIEW.md for the complete dependency graph

            // State 2: ChatState
            // Depends on: MainStore (already registered above)
            // Required by: workflow listeners, commands, HTTP server
            let app_handle_for_chat_state = app.handle().clone();
            let chat_state = ChatState::new(
                Arc::new(WindowChannels::new()),
                Some(app_handle_for_chat_state),
                main_store.clone(),
            );
            app.manage(chat_state.clone());

            // State 3: FilterManager
            // Depends on: None (self-contained)
            // Required by: sensitive data filtering commands
            let filter_manager = crate::sensitive::manager::FilterManager::new();
            if !filter_manager.is_healthy {
                if let Some(err) = &filter_manager.error_message {
                    log::error!("Failed to initialize FilterManager: {}. Sensitive info filtering will be disabled.", err);
                }
            }
            app.manage(filter_manager);

            // State 4: ScraperPool
            // Depends on: AppHandle
            // Required by: web scraping commands
            let scraper_pool = ScraperPool::new(app.handle().clone());
            app.manage(scraper_pool);

            // State 5: UpdateManager
            // Depends on: AppHandle
            // Required by: updater commands and background update task
            let update_manager = Arc::new(UpdateManager::new(app.handle().clone()));
            app.manage(update_manager.clone());

            // State 6: TsidGenerator
            let tsid_generator = Arc::new(crate::libs::tsid::TsidGenerator::new(1).expect("Failed to init TSID generator"));
            app.manage(tsid_generator.clone());

            // State 7: TauriGateway (Singleton for ReAct signals)
            let gateway = Arc::new(crate::workflow::react::gateway::TauriGateway::new(app.handle().clone()));
            app.manage(gateway.clone());

            // State 8: WorkflowManager (Session lifecycle manager)
            let workflow_manager = Arc::new(crate::workflow::react::manager::WorkflowManager::new());
            app.manage(workflow_manager.clone());

            // State 9: SubAgentFactory
            let factory: Arc<dyn crate::workflow::react::orchestrator::SubAgentFactory> = Arc::new(crate::workflow::react::orchestrator::DefaultSubAgentFactory {
                main_store: main_store.clone(),
                chat_state: chat_state.clone(),
                gateway: gateway.clone(),
                app_data_dir: app.path().app_data_dir().unwrap_or_default(),
                tsid_generator: tsid_generator.clone(),
            });
            app.manage(factory);

            // === END STATE REGISTRATION SECTION ===

            // === EVENT LISTENERS SECTION ===
            // IMPORTANT: Event listeners must be registered AFTER all states are managed!
            // Reason: Event handlers may immediately try to access states via handle.state()
            // If states are not yet registered, this will cause a panic
            // See: src-tauri/src/workflow/helper.rs for state usage in listeners
            // === END EVENT LISTENERS SECTION ===

            // === BACKGROUND TASKS SECTION ===
            // Critical tasks run in background, send event when ready
            let handle = app.handle().clone();
            let main_store_clone = main_store.clone();
            let chat_state_clone = chat_state.clone();
            let update_manager_clone = update_manager.clone();

            // 1. Initialize environment synchronously (Critical for get_env command)
            // This must run before background tasks to ensure PATH is ready for any spawned processes
            environment::init_environment();

            tauri::async_runtime::spawn(async move {
                // 1. Register tools (Can be async)
                let tm = chat_state_clone.tool_manager.clone();
                let _ = tm.register_available_tools(handle.clone()).await;

                // 2. Start the HTTP server
                // The HTTP server includes:
                // - Static file serving
                // - CCProxy (OpenAI-compatible chat completion proxy)
                // - MCP server management
                let handle_for_server = handle.clone();
                let main_store_for_server = main_store_clone.clone();
                let chat_state_for_server = chat_state_clone.clone();

                tauri::async_runtime::spawn(async move {
                    if let Err(e) = start_http_server(&handle_for_server, main_store_for_server, chat_state_for_server).await {
                        error!("Failed to start HTTP server: {}", e);
                    }
                });

                // 3. Update check (2 minutes later, non-critical)
                let auto_update = if let Ok(c) = main_store_clone.read() {
                    c.get_config(CFG_AUTO_UPDATE, true)
                } else {
                    true
                };

                if auto_update {
                    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                    loop {
                        if let Err(e) = update_manager_clone.check_and_download_update().await {
                            log::error!("Failed to check for updates: {}", e);
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
                    }
                }
            });

            // IMPORTANT: Manual window creation sequence.
            // This is critical for Windows compatibility to resolve race conditions where the frontend
            // process might launch and invoke commands before the backend's `setup` hook has finished
            // registering managed states. Auto-creating windows in `tauri.conf.json` via `"create": true`
            // can lead to "state not managed" panics or UI initialization failures in high-performance builds.
            // For any future windows, ensure `"create": false` is set in the configuration and
            // initialize them manually here or via specific logic after backend readiness is guaranteed.

            // 1. Main Window (Hidden by default)
            match window::create_main_window(&app.handle(), false) {
                Ok(win) => { restore_window_config(&win, main_store.clone()); },
                Err(e) => { log::error!("Failed to create main window: {}", e); }
            }

            // 2. Assistant Window (Hidden)
            match window::create_assistant_window(&app.handle(), false) {
                Ok(win) => { restore_window_config(&win, main_store.clone()); },
                Err(e) => { log::error!("Failed to create assistant window: {}", e); }
            }

            // 3. Workflow Window (Visible by default)
            match window::create_workflow_window(&app.handle(), true) {
                Ok(win) => { restore_window_config(&win, main_store.clone()); },
                Err(e) => { log::error!("Failed to create workflow window: {}", e); }
            }

            // 4. Proxy Switcher Window (Hidden)
            match window::create_proxy_switcher_window(&app.handle(), false) {
                Ok(win) => {
                    restore_window_config(&win, main_store.clone());
                }
                Err(e) => {
                    log::error!("Failed to create proxy switcher window: {}", e);
                }
            }

            // create tray
            let app_handle_clone = app.app_handle().clone();
            let _ = create_tray(&app_handle_clone, None);

            // Register window creation event handlers
            window::setup_window_creation_handlers(app_handle_clone.clone());

            WINDOW_READY.store(true, Ordering::SeqCst);

            // copy scrape resource
            let _ = scraper::ensure_default_configs_exist(&app_handle_clone);

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
        } else if window_label == "proxy_switcher" {
            CFG_PROXY_SWITCHER_WINDOW_SIZE
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
