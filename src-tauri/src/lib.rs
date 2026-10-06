// The canonical runtime modules live in the desktop-free
// `chatspeed-runtime-backend` crate, which this crate links as an ordinary Cargo
// dependency (U-6). The re-exports below keep every shared call site's
// `crate::ai::…`, `crate::db::…`, `crate::workflow::…` view without compiling a
// second copy of any canonical source. The modules declared with `mod` are the
// desktop's own adapters: the Tauri surface, the runtime control-plane client,
// the WebView-backed tools and the desktop error contract.
use chatspeed_runtime_backend::ai;
// The built-in agent synchronization is transport-neutral, so the desktop
// re-exports the runtime backend's canonical file.
/// The Phase 3 capability-management contract: one transport-neutral
/// application service owns every Agent Skill and MCP mutation, backed by the
/// runtime-owned journal. It never opens its own database connection and never
/// owns a runtime, so the standalone runtime is its only owner while the Tauri,
/// HTTP and CLI adapters delegate to it through the control plane.
pub use chatspeed_runtime_backend::capability;
use chatspeed_runtime_backend::ccproxy;
use chatspeed_runtime_backend::owner::builtin_agents;
pub mod chat_hub;
mod commands;
use chatspeed_runtime_backend::constants;
use chatspeed_runtime_backend::db;
use chatspeed_runtime_backend::environment;
/// The desktop Tauri-command error contract. It is a desktop adapter because it
/// names the Tauri/Wry/updater/HTTP error sources the runtime must not link.
pub mod error;
#[cfg(target_os = "linux")]
mod frame_edges;
// `runtime_client` (not to be confused with the `chatspeed_runtime_client` crate)
// is the desktop-side supervisor for the standalone runtime control plane.
pub mod runtime_client;
// `runtime_config` reads and writes the runtime-owned configuration the desktop
// still needs for startup and window geometry. It owns no database and is the
// only configuration access path the desktop has left.
mod runtime_config;
// `runtime_workflow` maps the Tauri workflow commands onto the runtime control
// plane's typed HTTP routes; only the desktop adapters can reach it.
#[cfg(feature = "desktop")]
mod runtime_workflow;
// `runtime_agent` maps the Tauri agent commands onto the runtime control
// plane's typed HTTP routes; only the desktop adapters can reach it.
#[cfg(feature = "desktop")]
mod runtime_agent;
// `runtime_data` holds the desktop transport adapters for the runtime-owned data
// commands. The transport-neutral command cores and their request bodies live in
// the runtime backend (`chatspeed_runtime_backend::data::runtime_data`).
mod http;
mod runtime_data;
// `libs` re-exports the runtime backend helpers and adds the desktop-only
// Tauri/Wry-bound `webview_proxy` adapter.
mod libs;
mod logger;
use chatspeed_runtime_backend::mcp;
#[cfg(feature = "desktop")]
mod runtime_web_bridge;
#[cfg(feature = "desktop")]
mod runtime_web_mcp_provider;
mod scraper;
mod search;
use chatspeed_runtime_backend::sensitive;
mod runtime_terminal;
mod shortcut;
mod terminal;
// `tools` re-exports the runtime backend tool contracts and adds the
// WebView-backed `web_fetch`/`web_search` implementations.
mod tools;
mod tray;
mod updater;
mod window;
use chatspeed_runtime_backend::workflow;

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
use std::time::Duration;
use std::time::Instant;

use tauri::async_runtime::{spawn, JoinHandle};
use tauri::Manager;
use tauri::PhysicalSize;
use tauri_plugin_autostart::ManagerExt;

// use commands::toolbar::*;
use crate::error::AppError;
use commands::agent::*;
use commands::capability::*;
use commands::ccproxy::*;
use commands::chat::*;
use commands::chat_hub::*;
use commands::clipboard::*;
use commands::config_transfer::*;
use commands::dev_tool::*;
use commands::env::*;
use commands::fs::*;
use commands::mcp::*;
use commands::message::*;
use commands::model_catalog::{
    list_models_dev_provider_models, list_models_dev_providers, resolve_model_profile,
};
use commands::note::*;
use commands::proxy_group::*;
use commands::sandbox::*;
use commands::sensitive::*;
use commands::setting::*;
use commands::terminal::*;
use commands::updater::{check_for_updates, install_and_restart};
use commands::window::*;
use commands::workflow::*;
use commands::workflow_automation::*;
use constants::*;
use http::server::start_http_server;
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
/// setting up command handlers, and configuring global shortcuts. The desktop
/// owns no runtime state: it connects to the standalone runtime through the
/// `RuntimeSupervisor` and reads the configuration it needs through the control
/// plane.
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

// Store auto-hide timers by window label so assistant interactions do not interfere.
static HIDE_TIMERS: LazyLock<StdMutex<HashMap<String, JoinHandle<()>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
static MOVE_TIMERS: LazyLock<StdMutex<HashMap<String, JoinHandle<()>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
static LAST_MOVES: LazyLock<StdMutex<HashMap<String, Instant>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

// Store the pending geometry writes by window label: a resize or a move replaces the
// write of the same window, so a drag stores the geometry it ended at.
static SIZE_TIMERS: LazyLock<StdMutex<HashMap<String, JoinHandle<()>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
static POSITION_TIMERS: LazyLock<StdMutex<HashMap<String, JoinHandle<()>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

fn should_auto_hide_on_focus_loss(label: &str) -> bool {
    matches!(label, "assistant")
}

fn should_preserve_visibility_while_dragging(label: &str) -> bool {
    matches!(label, "assistant")
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

fn hide_then_destroy_window(window: &tauri::Window) {
    if window.is_visible().unwrap_or(false) {
        if let Err(e) = window.hide() {
            warn!("Failed to hide window '{}': {}", window.label(), e);
        }
    }

    let app_handle = window.app_handle().clone();
    let window_label = window.label().to_string();
    spawn(async move {
        #[cfg(target_os = "macos")]
        let destroy_delay = Duration::from_millis(120);

        #[cfg(not(target_os = "macos"))]
        let destroy_delay = Duration::from_millis(0);

        tokio::time::sleep(destroy_delay).await;

        if let Some(target_window) = app_handle.get_webview_window(&window_label) {
            if target_window.is_visible().unwrap_or(false) {
                log::debug!(
                    "Skip destroying window '{}' because it became visible again",
                    window_label
                );
                return;
            }

            if let Err(e) = target_window.destroy() {
                warn!("Failed to destroy window '{}': {}", window_label, e);
            } else {
                log::debug!("Window '{}' destroyed after delayed hide", window_label);
            }
        }
    });
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
        window::show_and_focus_window(app, "main");
    }));

    builder
        .plugin(tauri_plugin_process::init())
        // Initialize the shell plugin
        .plugin(tauri_plugin_shell::init())
        // Register command handlers that can be invoked from the frontend
        .invoke_handler(tauri::generate_handler![
            // capability command (Phase 3 read-only surface)
            capability_skill_targets,
            capability_skill_inventory,
            capability_mcp_servers,
            capability_operation,
            capability_doctor,
            capability_reconcile,
            // capability command (Phase 3 Skill mutations)
            capability_skill_check,
            capability_skill_install,
            capability_skill_uninstall,
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
            get_sandbox_runtime_status,
            refresh_sandbox_runtime_status,

            // settings
            get_all_config,
            set_config,
            reload_config,
            get_api_key_encryption_status,
            activate_api_key_file,
            generate_api_key_file,
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
            export_config_package,
            inspect_config_package,
            import_config_package,
            get_all_backups,
            restore_setting,
            update_tray,
            // chat hub (web chat entries)
            get_all_chat_hubs,
            add_chat_hub,
            update_chat_hub,
            delete_chat_hub,
            update_chat_hub_order,
            show_chat_hub_page,
            hide_chat_hub_page,
            set_chat_hub_page_width,
            destroy_chat_hub_page,
            get_chat_hub_view_mode,
            get_chat_hub_page_limits,
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
            list_models_dev_providers,
            list_models_dev_provider_models,
            resolve_model_profile,
            chat_completion,
            stop_chat,
            sync_state,
            detect_language,
            // ccproxy stats
            delete_ccproxy_stats,
            get_ccproxy_daily_stats,
            get_ccproxy_grouped_stats,
            get_ccproxy_grouped_stats_by_date_range,
            get_ccproxy_today_cost_stats,
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
            run_mcp_tool,
            // proxy group
            proxy_group_list,
            proxy_group_add,
            proxy_group_update,
            proxy_group_batch_update,
            proxy_group_delete,
            set_active_proxy_group,
            get_active_proxy_group,
            // sandbox schemes
            get_sandbox_scheme_runtime_status,
            get_sandbox_schemes,
            add_sandbox_scheme,
            update_sandbox_scheme,
            delete_sandbox_scheme,
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
            get_available_terminal_shells,
            // workflow terminal (commands additionally enforce the workflow window label)
            terminal_list_shells,
            terminal_list_sessions,
            terminal_create,
            terminal_write,
            terminal_resize,
            terminal_close,
            // fs
            image_preview,
            image_source_url,
            read_text_file,
            get_text_file_info,
            read_text_file_for_editor,
            write_text_file_for_editor,
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
            get_earlier_workflow_message_page,
            get_earlier_workflow_messages,
            get_workflow_agent_config,
            workflow_begin_new_context_frame,
            get_workflow_snapshot,
            list_pending_sub_agent_approvals,
            get_workflow_session_key,
            list_workflows,
            search_workspace_files,
            update_workflow_allowed_paths,
            update_workflow_final_audit,
            update_workflow_auto_compress,
            update_workflow_personality,
            update_workflow_approval_level,
            update_workflow_model_config,
            update_workflow_skills_config,
            update_workflow_phase,
            update_workflow_sandbox_config,
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
            workflow_subscribe,
            workflow_start,
            workflow_stop,
            workflow_automation_delete,
            workflow_automation_draft,
            workflow_automation_apply,
            workflow_automation_get,
            workflow_automation_list,
            workflow_automation_list_runs,
            workflow_automation_run_now,
            workflow_automation_run_views,
            workflow_automation_save,
            workflow_automation_set_enabled,
            get_workflow_events,
            get_workflow_dispatcher_metrics,
            get_workflow_efficiency_report,

            // dev tools
            test_scrape,
            // updater
            check_for_updates,
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
            // When the user clicks the close button, assistant/workflow are hidden.
            // Main and proxy switcher are allowed to close so they can release resources.
            // Settings, note, and proxy switcher are briefly hidden and then force-destroyed on
            // macOS to avoid WKWebView close-time layer tree races while still releasing resources.
            tauri::WindowEvent::CloseRequested { api, .. } => {
                match window.label() {
                    // For these windows, we just hide them.
                    "assistant" | "workflow" => {
                        api.prevent_close();
                        // The workflow window is only hidden so running tasks survive,
                        // and the embedded ChatHub webview is deliberately left
                        // untouched: hiding the window hides the child with it, and the
                        // page (and its site session) is still there when the window is
                        // shown again. It is released only when the window is destroyed.
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
                    "settings" | "note" | "proxy_switcher" => {
                        api.prevent_close();
                        hide_then_destroy_window(window);
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
                    schedule_window_size_save(window);

                    // The page is docked inside the workflow window, so a window resize
                    // has to be forwarded to the carriers that place the page themselves.
                    // The reported size is passed on, because a resize is reported before the
                    // window has applied it: only the carrier knows whether the page may be
                    // laid out for that geometry.
                    if window_label == chat_hub::CHAT_HUB_HOST_WINDOW_LABEL {
                        if let Some(chat_hub_state) =
                            window.try_state::<chat_hub::ChatHubPageState>()
                        {
                            if let Err(e) =
                                chat_hub_state.sync_bounds(window.app_handle(), *size)
                            {
                                warn!("Failed to resize the ChatHub page: {}", e);
                            }
                        }
                    }
                }
            }
            tauri::WindowEvent::Moved(_position) => {
                if !WINDOW_READY.load(Ordering::Relaxed) {
                    return;
                }

                // The ChatHub page is docked inside the workflow window (see
                // `src/chat_hub`), so moving that window moves the page with it on every
                // platform, and there is nothing to do for it here.
                if window.label() == "main" {
                    // Save the main window position when it is moved.
                    schedule_window_position_save(window, CFG_WINDOW_POSITION);
                } else if window.label() == "workflow" {
                    // Save the workflow window position when it is moved.
                    schedule_window_position_save(window, CFG_WORKFLOW_WINDOW_POSITION);
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
            // Release the ChatHub page together with the Workflow window it is docked
            // into, so application exit leaves no page and no stale view state behind.
            tauri::WindowEvent::Destroyed => {
                if window.label() == chat_hub::CHAT_HUB_HOST_WINDOW_LABEL {
                    let app_handle = window.app_handle();
                    if let Some(chat_hub_state) = app_handle.try_state::<chat_hub::ChatHubPageState>()
                    {
                        chat_hub_state.release(app_handle);
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

            // The runtime owns the canonical database. The desktop deliberately
            // opens no local copy: the configuration it needs is read through the
            // runtime supervisor and applied once the configuration snapshot
            // arrives (see the startup task in the state registration section).

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
            //
            // The runtime owns the database, the chat/tool execution state, the
            // workflow hub and manager, the sub-agent factory, the application
            // services, the capability service and the control plane. The desktop
            // registers none of them; it reaches them through the
            // `RuntimeSupervisor` below.

            // FilterManager
            // Depends on: None (self-contained)
            // Required by: sensitive data filtering commands
            let filter_manager = crate::sensitive::manager::FilterManager::new();
            if !filter_manager.is_healthy {
                if let Some(err) = &filter_manager.error_message {
                    log::error!("Failed to initialize FilterManager: {}. Sensitive info filtering will be disabled.", err);
                }
            }
            app.manage(filter_manager);

            // ScraperPool
            // Depends on: AppHandle
            // Required by: web scraping commands
            let scraper_pool = ScraperPool::new(app.handle().clone());
            app.manage(scraper_pool);

            // UpdateManager
            // Depends on: AppHandle
            // Required by: updater commands and background update task
            let update_manager = Arc::new(UpdateManager::new(app.handle().clone()));
            app.manage(update_manager.clone());

            // The runtime owns all interactive PTYs. Desktop terminal commands
            // are typed RuntimeSupervisor adapters and keep no local process state.
            // ChatHubPageState
            // Owns the single ChatHub page docked inside the Workflow window.
            app.manage(chat_hub::ChatHubPageState::new());

            // RuntimeSupervisor: the desktop's client relationship with the
            // standalone `chatspeed-runtime` control plane (attach-or-start,
            // readiness handshake, client lease and heartbeat). It owns no
            // runtime state itself. `RuntimeConfigCache` holds the runtime
            // configuration the desktop mirrors locally for startup and window
            // geometry; it opens no database either. Connecting is spawned so
            // `setup` never blocks the first paint; a runtime that is not running
            // yet is reported through the supervisor's redacted status instead of
            // failing startup.
            let runtime_supervisor = Arc::new(crate::runtime_client::RuntimeSupervisor::new());
            app.manage(runtime_supervisor.clone());
            let runtime_config_cache = Arc::new(crate::runtime_config::RuntimeConfigCache::new());
            app.manage(runtime_config_cache.clone());
            {
                let app_handle = app.handle().clone();
                let supervisor = runtime_supervisor.clone();
                let cache = runtime_config_cache.clone();
                tauri::async_runtime::spawn(async move {
                    // Resolve the typed launch configuration in the background so
                    // a missing platform directory never blocks the first paint;
                    // it is reported as unavailable instead of a `.`-relative
                    // fallback.
                    let launch_config = match crate::runtime_client::default_launch_config() {
                        Ok(config) => config,
                        Err(error) => {
                            log::warn!("[RuntimeSupervisor] unavailable: {}", error);
                            return;
                        }
                    };
                    let runtime_dir = launch_config.runtime_dir().to_path_buf();
                    if let Err(error) = supervisor
                        .connect_or_start(
                            &launch_config,
                            crate::runtime_client::DESKTOP_CLIENT_ID,
                        )
                        .await
                    {
                        // The runtime owns the database and every startup value;
                        // without it the desktop reports the redacted supervisor
                        // status and leaves runtime-owned startup configuration
                        // and window restore unavailable instead of falling back
                        // to a local database.
                        log::warn!("[RuntimeSupervisor] unavailable: {}", error);
                        return;
                    }
                    log::info!(
                        "[RuntimeSupervisor] connected to runtime at {:?}",
                        runtime_dir
                    );

                    // Start the dedicated desktop Web MCP provider and register
                    // it with the runtime. The runtime reaches `web_fetch` and
                    // `web_search` as an ordinary MCP server, so the fixed web
                    // tools stay on the canonical ToolManager path. A failure is
                    // reported and leaves web capabilities unavailable; it never
                    // starts a second runtime or a local fallback.
                    match crate::runtime_web_mcp_provider::start(app_handle.clone(), &supervisor)
                        .await
                    {
                        Ok(()) => {
                            log::info!("[WebMcpProvider] loopback Web MCP provider started")
                        }
                        Err(error) => {
                            log::warn!("[WebMcpProvider] provider unavailable: {}", error)
                        }
                    }

                    match crate::runtime_config::load(supervisor.as_ref()).await {
                        Ok(snapshot) => {
                            // Cache before applying and restoring window geometry
                            // so the geometry the restore writes back is compared
                            // against the values it was read from.
                            cache.store(snapshot.clone());
                            if let Err(error) = crate::shortcut::register_desktop_shortcut(&app_handle) {
                                log::error!("Failed to register desktop shortcuts: {:?}", error);
                            }
                            let _ = crate::tray::create_tray(&app_handle, None);
                            apply_runtime_startup_config(&app_handle, supervisor.clone(), &snapshot)
                                .await;
                            restore_initial_windows(&app_handle, &snapshot);
                        }
                        Err(error) => log::error!(
                            "[RuntimeSupervisor] failed to load the runtime configuration: {}",
                            error
                        ),
                    }
                });
            }

            // === END STATE REGISTRATION SECTION ===

            // === EVENT LISTENERS SECTION ===
            // IMPORTANT: Event listeners must be registered AFTER all states are managed!
            // Reason: Event handlers may immediately try to access states via handle.state()
            // If states are not yet registered, this will cause a panic
            // See: src-tauri/src/workflow/helper.rs for state usage in listeners
            // === END EVENT LISTENERS SECTION ===

            // Create the workflow window as soon as every command dependency has been
            // registered. The Vue startup overlay keeps the window responsive while
            // non-essential synchronization continues in the background.
            let app_handle = app.handle().clone();
            window::setup_window_creation_handlers(app_handle.clone());
            if let Err(error) = window::create_workflow_window(&app_handle, true) {
                log::error!("Failed to create workflow window: {}", error);
            }
            if let Err(error) = window::create_assistant_window(&app_handle, false) {
                log::error!("Failed to create assistant window: {}", error);
            }
            WINDOW_READY.store(true, Ordering::SeqCst);

            // === BACKGROUND TASKS SECTION ===
            // Non-essential startup work must not delay the first paint.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(environment::init_environment);

            {
                // The runtime owns the agent table and the configured MCP
                // servers, so synchronizing them is no longer a desktop task;
                // only the local scraper-schema files stay here.
                let handle_for_startup = handle.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = scraper::ensure_default_configs_exist(&handle_for_startup) {
                        log::error!("Failed to synchronize scraper schemas: {}", error);
                    }
                });
            }

            // The static file server (theme/upload/tmp assets and `/save/png`) is
            // desktop-only and touches no runtime-owned state. The runtime owns
            // the OpenAI-compatible chat proxy, which is no longer started here.
            {
                let handle_for_server = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = start_http_server(&handle_for_server).await {
                        error!("Failed to start HTTP server: {}", e);
                    }
                });
            }

            // Update check (2 minutes later, non-critical).
            {
                let cache_for_update = runtime_config_cache.clone();
                let update_manager_for_loop = update_manager.clone();
                tauri::async_runtime::spawn(async move {
                    let auto_update = cache_for_update
                        .current()
                        .map(|snapshot| snapshot.get_bool(CFG_AUTO_UPDATE, true))
                        .unwrap_or(true);
                    if !auto_update {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(120)).await;
                    loop {
                        if let Err(e) = update_manager_for_loop.check_and_download_update().await {
                            log::error!("Failed to check for updates: {}", e);
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(24 * 60 * 60)).await;
                    }
                });
            }

            // create tray after runtime config is loaded; the startup task
            // recreates it with runtime-provided shortcut hints.
            let app_handle_clone = app.app_handle().clone();
            let _ = create_tray(&app_handle_clone, None);

            Ok(())
        })
        // Run the Tauri application with the generated context
        .build(tauri::generate_context!())
        .map_err(|e| AppError::General{message:e.to_string()})?
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                // Hand the runtime lease back. Releasing is I/O and cannot be
                // awaited on the event thread, so it is spawned best-effort; if
                // the process exits first, the lease TTL and the runtime's idle
                // grace end it. This never kills a runtime owned by another
                // client, and the runtime owns its own control-plane discovery
                // and shutdown.
                if let Some(supervisor) =
                    app_handle.try_state::<Arc<crate::runtime_client::RuntimeSupervisor>>()
                {
                    let supervisor = supervisor.inner().clone();
                    tauri::async_runtime::spawn(async move { supervisor.shutdown().await });
                }
            }
        });
    Ok(())
}

/// Applies the runtime-owned startup configuration to local desktop side effects.
///
/// The values live in the runtime configuration; the desktop only mirrors the
/// client-side consequences (interface locale, autostart registration and the
/// idle-sleep inhibitor) and never opens a database. The Models.dev catalog is
/// owned and refreshed by the runtime.
async fn apply_runtime_startup_config(
    app: &tauri::AppHandle,
    supervisor: Arc<crate::runtime_client::RuntimeSupervisor>,
    snapshot: &crate::runtime_config::RuntimeConfigSnapshot,
) {
    // Reconcile the interface language with the persisted preference, mirroring
    // the runtime's own normalization and writing the normalized value back.
    let stored_lang = snapshot.get_string(CFG_INTERFACE_LANGUAGE, &libs::lang::get_system_locale());
    let user_lang = libs::lang::normalize_interface_locale(&stored_lang).to_string();
    if stored_lang != user_lang {
        let normalized_value = serde_json::Value::String(user_lang.clone());
        if let Err(error) = crate::runtime_data::set_config(
            supervisor.as_ref(),
            CFG_INTERFACE_LANGUAGE.to_string(),
            normalized_value,
        )
        .await
        {
            log::error!("Failed to persist normalized interface language: {}", error);
        }
    }
    set_locale(&user_lang);
    log::info!("Set interface language to {}", user_lang);

    // Reconcile the OS-managed autostart registration with the stored preference.
    let auto_start = snapshot.get_bool(CFG_AUTO_START, false);
    let autolaunch = app.autolaunch();
    match autolaunch.is_enabled() {
        Ok(is_enabled) if auto_start != is_enabled => {
            let result = if auto_start {
                autolaunch.enable()
            } else {
                autolaunch.disable()
            };
            if let Err(e) = result {
                log::error!("Failed to synchronize autostart registration: {}", e);
            } else {
                log::info!("Autostart registration synchronized: {}", auto_start);
            }
        }
        Ok(_) => {}
        Err(e) => log::error!("Failed to read autostart registration: {}", e),
    }

    // Mirror the idle-sleep inhibitor preference.
    let workflow_prevent_idle_sleep = snapshot.get_bool(CFG_WORKFLOW_PREVENT_IDLE_SLEEP, false);
    crate::workflow::react::idle_sleep::WORKFLOW_IDLE_SLEEP_INHIBITOR
        .set_enabled(workflow_prevent_idle_sleep);
}

/// Restores the saved geometry of the windows created during setup.
///
/// Runs once the runtime configuration snapshot is available; a window that is
/// absent keeps the geometry it was created with. Window creation itself stays
/// synchronous, so the first paint is never blocked on the runtime.
fn restore_initial_windows(
    app: &tauri::AppHandle,
    snapshot: &crate::runtime_config::RuntimeConfigSnapshot,
) {
    for label in ["workflow", "assistant"] {
        match app.get_webview_window(label) {
            Some(window) => {
                restore_window_config(&window, snapshot.restore_config(label));
            }
            None => log::warn!(
                "Window '{}' was not created; skipping geometry restore",
                label
            ),
        }
    }
}

/// Remembers the size the user left a window at.
///
/// The size is read from the window itself instead of the resize event, because the
/// event carries the size the window had when the platform reported the resize. A
/// window is created with a default size and its saved size is restored right after,
/// while the setup hook runs, and the reports both steps produce only reach the event
/// loop afterwards: they can arrive once initialization has finished while still
/// describing the default size, which would overwrite the size the user left behind.
///
/// A window the user cannot see cannot be resized by the user, so nothing is returned
/// while it is hidden.
fn current_saved_window_size(
    window: &tauri::Window,
    snapshot: &crate::runtime_config::RuntimeConfigSnapshot,
) -> Option<WindowSize> {
    if !window.is_visible().unwrap_or(false) {
        return None;
    }

    let (Ok(size), Ok(scale_factor)) = (window.inner_size(), window.scale_factor()) else {
        warn!(
            "Failed to read the current size of window '{}'",
            window.label()
        );
        return None;
    };

    // Convert the physical size to the logical size the configuration stores.
    let logical_size = size.to_logical::<f64>(scale_factor);
    if logical_size.width <= 0.0 || logical_size.height <= 0.0 {
        return None;
    }

    // The window may be holding the docked ChatHub page, which widened it by the width that
    // page needs. That width belongs to the page rather than to the window, so it is handed
    // back here and kept out of the record: reopening the app must not restore a window that
    // is wider than the workflow UI ever was.
    let width = width_without_docked_page(window, logical_size.width);

    let saved_size = snapshot.window_size(window.label()).unwrap_or_default();
    if saved_size.width == width && saved_size.height == logical_size.height {
        return None;
    }

    Some(WindowSize {
        width,
        height: logical_size.height,
    })
}

/// Width of a window without the space the docked ChatHub page is holding.
///
/// The page is docked inside the workflow window and widens it by the width the page needs,
/// so a remembered size has to leave that width out. A window that does not host the page,
/// or one whose page is hidden, keeps the width it was measured with.
fn width_without_docked_page(window: &tauri::Window, measured_width: f64) -> f64 {
    if window.label() != chat_hub::CHAT_HUB_HOST_WINDOW_LABEL {
        return measured_width;
    }

    let docked_width = window
        .try_state::<chat_hub::ChatHubPageState>()
        .map(|state| state.inner().grown_width())
        .unwrap_or(0.0);

    remembered_width(measured_width, docked_width)
}

/// Width a window is remembered with, leaving the width of a docked page out.
///
/// The workflow UI always keeps [`chat_hub::CHAT_HUB_MIN_HOST_WIDTH`] next to a page, so a
/// remembered width below it cannot describe a window the user could have had the page open
/// in: it can only come from a window that was shrunk by hand while the page was docked.
pub(crate) fn remembered_width(measured_width: f64, docked_width: f64) -> f64 {
    if docked_width > 0.0 && docked_width < measured_width {
        (measured_width - docked_width).max(chat_hub::CHAT_HUB_MIN_HOST_WIDTH)
    } else {
        measured_width
    }
}

/// How long a window change waits before it is written to the configuration.
///
/// A resize or a move is reported to the event loop before the window has applied the
/// change it describes, and the windows are created with a default geometry that is
/// replaced by the saved one while the setup hook runs: the reports of those two steps
/// reach the event loop only afterwards, where they would be read as the geometry the
/// window had before it was restored. Waiting for the change to settle is what keeps
/// such a report from overwriting the geometry the user left the window at.
const WINDOW_GEOMETRY_SAVE_DELAY: Duration = Duration::from_millis(300);

/// Writes the size of a window back once its current resize has settled.
///
/// A new resize replaces the pending write, so dragging a window stores the size it
/// ended at instead of every step along the way.
fn schedule_window_size_save(window: &tauri::Window) {
    let label = window.label().to_string();
    let window = window.clone();

    let Ok(mut timers) = SIZE_TIMERS.lock() else {
        error!("SIZE_TIMERS mutex is poisoned");
        return;
    };

    if let Some(handle) = timers.remove(&label) {
        handle.abort();
    }

    let timer = spawn(async move {
        tokio::time::sleep(WINDOW_GEOMETRY_SAVE_DELAY).await;

        let Some(snapshot) = window
            .try_state::<Arc<crate::runtime_config::RuntimeConfigCache>>()
            .and_then(|cache| cache.current())
        else {
            return;
        };
        let Some(size) = current_saved_window_size(&window, snapshot.as_ref()) else {
            return;
        };
        let Some(supervisor) = window
            .try_state::<Arc<crate::runtime_client::RuntimeSupervisor>>()
            .map(|state| state.inner().clone())
        else {
            error!("Runtime supervisor is not registered; skipping window size save");
            return;
        };

        if let Err(error) =
            crate::runtime_config::save_window_size(supervisor.as_ref(), window.label(), size).await
        {
            error!("Failed to save window size: {}", error);
        }
    });

    timers.insert(label, timer);
}

/// Writes the position of a window back once its current move has settled.
///
/// A new move replaces the pending write, so dragging a window stores the position it
/// ended at instead of every step along the way.
fn schedule_window_position_save(window: &tauri::Window, key: &'static str) {
    let label = window.label().to_string();
    let window = window.clone();

    let Ok(mut timers) = POSITION_TIMERS.lock() else {
        error!("POSITION_TIMERS mutex is poisoned");
        return;
    };

    if let Some(handle) = timers.remove(&label) {
        handle.abort();
    }

    let timer = spawn(async move {
        tokio::time::sleep(WINDOW_GEOMETRY_SAVE_DELAY).await;

        let Some(snapshot) = window
            .try_state::<Arc<crate::runtime_config::RuntimeConfigCache>>()
            .and_then(|cache| cache.current())
        else {
            return;
        };
        let Some(position) = current_saved_window_position(&window, snapshot.as_ref(), key) else {
            return;
        };
        let Some(supervisor) = window
            .try_state::<Arc<crate::runtime_client::RuntimeSupervisor>>()
            .map(|state| state.inner().clone())
        else {
            error!("Runtime supervisor is not registered; skipping window position save");
            return;
        };

        if let Err(error) =
            crate::runtime_config::save_window_position(supervisor.as_ref(), key, position).await
        {
            error!("Failed to save window position: {}", error);
        }
    });

    timers.insert(label, timer);
}

/// The position a window should be remembered at, if it differs from the stored one.
///
/// The position is read from the window itself instead of the move event, because the
/// event carries the position the window had when the platform reported the move: a
/// window is created centered before its saved position is restored, while setup runs,
/// and that early report only reaches the event loop afterwards, where it would store
/// the default position over the one the user left behind.
fn current_saved_window_position(
    window: &tauri::Window,
    snapshot: &crate::runtime_config::RuntimeConfigSnapshot,
    key: &str,
) -> Option<MainWindowPosition> {
    let Ok(current_position) = window.outer_position() else {
        warn!(
            "Failed to read the current position of window '{}'",
            window.label()
        );
        return None;
    };

    let old_pos = if key == CFG_WORKFLOW_WINDOW_POSITION {
        snapshot.workflow_window_position()
    } else {
        snapshot.main_window_position()
    };
    let screen_name = get_screen_name(window);

    if old_pos.screen_name != screen_name
        || old_pos.x != current_position.x
        || old_pos.y != current_position.y
    {
        let current_window_size = match window.outer_size() {
            Ok(size) => size,
            Err(e) => {
                warn!(
                    "Failed to get outer size when saving window '{}' position: {}. Skipping save.",
                    window.label(),
                    e
                );
                return None;
            }
        };

        if !window::is_position_on_any_screen(
            window.app_handle(),
            current_position.x,
            current_position.y,
            PhysicalSize::new(current_window_size.width, current_window_size.height),
        ) {
            warn!(
                "Skipping save for window '{}' position ({}, {}) because it is invalid for current monitors.",
                window.label(),
                current_position.x,
                current_position.y
            );
            return None;
        }

        return Some(MainWindowPosition {
            screen_name,
            x: current_position.x,
            y: current_position.y,
        });
    }

    None
}
