use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

use log::{debug, error, warn};
use rust_i18n::t;
use serde::Deserialize;
use serde::Serialize;
use tauri::Listener;
use tauri::LogicalSize;
use tauri::Manager;
use tauri::PhysicalPosition;
use tauri::PhysicalSize;
use tauri::WebviewWindow;
use tauri::WebviewWindowBuilder;
use tauri::Window;

use crate::constants::CFG_WINDOW_POSITION;
use crate::constants::CFG_WINDOW_SIZE;
use crate::db::MainStore;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f64,
    pub height: f64,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MainWindowPosition {
    pub screen_name: Option<String>,
    pub x: i32,
    pub y: i32,
}

impl Default for MainWindowPosition {
    fn default() -> Self {
        Self {
            screen_name: None,
            x: 0,
            y: 0,
        }
    }
}

/// Represents a rectangle for intersection checks.
#[derive(Clone, Copy, Debug)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    /// Checks if this rectangle intersects with another rectangle.
    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }
}

/// Checks if a given window position and size would be on any available screen.
///
/// # Arguments
/// * `app_handle` - The Tauri application handle to get monitor information.
/// * `position_x` - The X coordinate of the window's top-left corner.
/// * `position_y` - The Y coordinate of the window's top-left corner.
/// * `window_size` - The physical size of the window.
///
/// # Returns
/// `true` if the window would be at least partially on a screen, `false` otherwise.
fn is_position_on_any_screen<R: tauri::Runtime>(
    app_handle: &tauri::AppHandle<R>,
    position_x: i32,
    position_y: i32,
    window_size: PhysicalSize<u32>,
) -> bool {
    match app_handle.available_monitors() {
        Ok(monitors) => {
            if monitors.is_empty() {
                warn!("No monitors available to check window position against.");
                return false; // Or true, depending on desired behavior if no monitors
            }

            #[cfg(debug_assertions)]
            {
                debug!(
                    "is_position_on_any_screen: Checking position ({}, {}) with window_size {}x{}",
                    position_x, position_y, window_size.width, window_size.height
                );
                debug!("Available monitors for position check (is_position_on_any_screen):");

                for (i, monitor) in monitors.iter().enumerate() {
                    debug!(
                        "  Monitor {}: Name: {:?}, Position: {:?}, Size: {:?}, ScaleFactor: {}",
                        i,
                        monitor.name(),
                        monitor.position(),
                        monitor.size(),
                        monitor.scale_factor()
                    );
                }
            }

            let window_rect = Rect {
                x: position_x,
                y: position_y,
                width: window_size.width as i32,
                height: window_size.height as i32,
            };

            #[cfg(debug_assertions)]
            debug!(
                "Window rect for check (is_position_on_any_screen): {:?}",
                window_rect
            );

            for monitor in monitors {
                let monitor_pos = monitor.position();
                let monitor_size = monitor.size();
                let monitor_rect = Rect {
                    x: monitor_pos.x,
                    y: monitor_pos.y,
                    width: monitor_size.width as i32,
                    height: monitor_size.height as i32,
                };

                #[cfg(debug_assertions)]
                debug!(
                    "  Checking against monitor_rect (is_position_on_any_screen): {:?}",
                    monitor_rect
                );
                if window_rect.intersects(&monitor_rect) {
                    #[cfg(debug_assertions)]
                    debug!(
                        "  Intersection FOUND with monitor {:?} ({:?}) (is_position_on_any_screen)",
                        monitor.name(),
                        monitor_pos
                    );
                    return true;
                }
            }
            warn!(
                "Window position ({}, {}) with size {}x{} is off-screen.",
                position_x, position_y, window_size.width, window_size.height
            );
            false
        }
        Err(e) => {
            error!("Failed to get available monitors: {}", e);
            false // Conservatively assume off-screen if monitor info is unavailable
        }
    }
}

/// Fixes a visual artifact bug in Tauri v2 where transparent windows initially lack shadows and borders
/// This is a temporary workaround and can be removed once the issue is fixed in Tauri
///
/// 修复 Tauri v2 中透明窗口初始化时缺少阴影和边框的视觉 bug
/// 这是一个临时的解决方案，当 Tauri 官方修复此问题后可以移除此函数
///
/// # How it works
/// The function triggers a minimal resize by temporarily increasing the window height by 1 pixel
/// and then restoring it, which forces the window manager to properly render the window decorations
///
/// # 工作原理
/// 通过临时将窗口高度增加 1 像素然后还原的方式触发一个最小的调整，
/// 这样可以强制窗口管理器正确渲染窗口装饰效果
///
/// # Arguments 参数
/// * `window` - Reference to the window that needs fixing
///            需要修复的窗口引用
/// * `size` - Optional window size. If not provided, current window size will be used
///          可选的窗口大小。如果未提供，将使用当前窗口大小
///
/// # Returns 返回值
/// * `Result<(), Box<dyn std::error::Error>>` - Success or error if the operation fails
///                                            操作成功返回 Ok(()), 失败返回错误
///
/// # Note 注意
/// This function can be removed once Tauri fixes the transparent window initialization issue
/// 当 Tauri 修复透明窗口初始化问题后，可以移除此函数
pub async fn fix_window_visual(
    _window: &tauri::WebviewWindow,
    _size: Option<WindowSize>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(target_os = "windows"))]
    {
        use tauri::LogicalSize;
        let mut size = _size
            .map(|s| LogicalSize::new(s.width, s.height))
            .unwrap_or_else(|| {
                _window
                    .inner_size()
                    .unwrap_or(PhysicalSize::new(0, 0))
                    .to_logical(_window.scale_factor().unwrap_or(1.0))
            });

        if size.width == 0.0 || size.height == 0.0 {
            return Ok(());
        }

        size.height += 1.0;
        _window.set_size(tauri::Size::Logical(size))?;
        log::info!("Window size set to: {}x{}", size.width, size.height);

        // wait for window to be resized
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        size.height -= 1.0;
        log::info!("Window size restored to: {}x{}", size.width, size.height);
        _window.set_size(tauri::Size::Logical(size))?;
    }
    Ok(())
}

/// Toggles the visibility of the assistant window.
///
/// If the assistant window exists, it will be shown or hidden based on its current state.
/// If it does not exist, a new assistant window will be created with specified configurations.
///
/// # Arguments
/// - `app`: A reference to the Tauri application handle.
///
/// # Example
/// ```no_run
/// use tauri::App;
/// toggle_assistant_window(&app);
/// ```
pub fn toggle_assistant_window(app: &tauri::AppHandle) {
    let window_label = "assistant";
    if let Some(window) = app.get_webview_window(window_label) {
        if let Ok(scale_factor) = window.scale_factor() {
            if let Err(e) = window.set_min_size(Some(tauri::Size::Physical(PhysicalSize {
                width: (400.0 * scale_factor) as u32,
                height: (400.0 * scale_factor) as u32,
            }))) {
                warn!("Failed to set minimum size for assistant window: {}", e);
            }
        }

        // 先检查窗口是否有效
        if window.is_visible().unwrap_or(true) {
            // 如果已经可见，只需要设置焦点
            if let Err(e) = window.set_focus() {
                warn!("Failed to set focus to assistant window: {}", e);
            }
        } else {
            // 如果不可见，先显示窗口
            if let Err(e) = window.show() {
                warn!("Failed to show assistant window: {}", e);
            } else {
                // 显示成功后再设置焦点
                if let Err(e) = window.set_focus() {
                    warn!("Failed to set focus to assistant window: {}", e);
                }
            }
        }
    } else {
        match WebviewWindowBuilder::new(
            app,
            window_label,
            tauri::WebviewUrl::App(format!("/{}", window_label).into()),
        )
        .decorations(false)
        .transparent(true)
        .skip_taskbar(true)
        .min_inner_size(400.0, 500.0)
        .center()
        .build()
        {
            Ok(window) => {
                let window_clone = window.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = fix_window_visual(&window_clone, None).await {
                        error!("Failed to fix window visual: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Create assistant window error: {}", e);
            }
        }
    }
}

/// Toggles the visibility of the main window.
///
/// If the main window exists, it will be shown or hidden based on its current state.
/// If it does not exist, a new main window will be created with specified configurations.
pub fn toggle_main_window(app: &tauri::AppHandle) {
    let window_label = "main";
    if let Some(window) = app.get_webview_window(window_label) {
        if let Ok(is_visible) = window.is_visible() {
            if is_visible {
                // if let Err(e) = window.hide() {
                //     warn!("Failed to hide main window: {}", e);
                // }
                if let Err(e) = window.set_focus() {
                    warn!("Failed to set focus to main window: {}", e);
                }
            } else {
                // 先检查窗口是否最小化
                if let Ok(is_minimized) = window.is_minimized() {
                    if is_minimized {
                        // 如果窗口最小化，先恢复窗口
                        if let Err(e) = window.unminimize() {
                            warn!("Failed to unminimize main window: {}", e);
                        } else {
                            // 等待瞬间确保窗口已恢复
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                }

                // 最后设置焦点
                if window.is_visible().unwrap_or(false) {
                    if let Err(e) = window.set_focus() {
                        warn!("Failed to set focus to main window: {}", e);
                    }
                }
                // if let Err(e) = window.show() {
                //     warn!("Failed to show main window: {}", e);
                // }
            }
        } else {
            warn!("Failed to determine visibility of main window");
            // 如果无法确定可见性，尝试显示窗口
            if let Err(e) = window.show() {
                warn!("Failed to show main window: {}", e);
            } else if let Err(e) = window.set_focus() {
                warn!("Failed to set focus to main window: {}", e);
            }
        }
    }
}

/// Internal function to create or open note window
///
/// This function is used to open a new note window, or if the window already exists, it displays and focuses the window.
///
/// # Arguments
/// - `app_handle` - Tauri application handle
///
/// # Returns
/// - `Result<(), String>` - Ok if successful, Err with error message if failed
pub async fn create_or_focus_note_window(app_handle: tauri::AppHandle) -> Result<(), String> {
    let label = "note";

    if let Some(window) = app_handle.get_webview_window(label) {
        if !window
            .is_visible()
            .map_err(|e| t!("main.failed_to_check_window_visibility", error = e))?
        {
            window
                .show()
                .map_err(|e| t!("main.failed_to_show_window", error = e))?;
        }
        window
            .set_focus()
            .map_err(|e| t!("main.failed_to_set_window_focus", error = e))?;
    } else {
        let mut webview_window_builder =
            WebviewWindowBuilder::new(&app_handle, label, tauri::WebviewUrl::App("/note".into()))
                .title("Notes")
                .decorations(false)
                .inner_size(850.0, 600.0)
                .min_inner_size(600.0, 400.0)
                .center();
        #[cfg(target_os = "windows")]
        {
            webview_window_builder = webview_window_builder.transparent(false);
        }
        #[cfg(not(target_os = "windows"))]
        {
            webview_window_builder = webview_window_builder.transparent(true);
        }
        let webview_window = webview_window_builder
            .build()
            .map_err(|e| t!("main.failed_to_create_note_window", error = e))?;

        let _ = webview_window.show();
        let _ = webview_window.set_focus();

        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::window::fix_window_visual(&webview_window, None).await {
                log::error!("Failed to fix note window visual: {}", e);
            }
        });
    }
    Ok(())
}

#[derive(Deserialize)]
struct SettingWindowPayload {
    setting_type: String,
}

#[derive(Deserialize)]
struct UrlWindowPayload {
    url: String,
}

/// Internal function to create or focus setting window
///
/// Creates a new setting window or focuses an existing one
///
/// # Arguments
/// - `app_handle` - The Tauri application handle
/// - `setting_type` - The type of setting to show
///
/// # Returns
/// - `Result<(), String>` - Ok if successful, Err with error message if failed
pub async fn create_or_focus_setting_window(
    app_handle: tauri::AppHandle,
    setting_type: Option<&str>,
) -> Result<(), String> {
    let label = "settings";
    if let Some(window) = app_handle.get_webview_window(label) {
        if let Some(setting_type) = setting_type {
            window
                .eval(&format!(
                    "window.location.href = '/settings/{}';console.log('/settings/{}')",
                    setting_type, setting_type
                ))
                .map_err(|e| t!("main.failed_to_navigate_to_settings", error = e))?;
        }
        if !window
            .is_visible()
            .map_err(|e| t!("main.failed_to_check_window_visibility", error = e))?
        {
            window
                .show()
                .map_err(|e| t!("main.failed_to_show_window", error = e))?;
        }
        window
            .set_focus()
            .map_err(|e| t!("main.failed_to_set_window_focus", error = e))?;
    } else {
        let mut webview_window_builder = WebviewWindowBuilder::new(
            &app_handle,
            label,
            tauri::WebviewUrl::App(format!("/settings/{}", setting_type.unwrap_or("")).into()),
        )
        .title("")
        .decorations(false)
        .maximizable(false)
        .inner_size(650.0, 750.0)
        .min_inner_size(650.0, 600.0)
        .center();

        #[cfg(target_os = "windows")]
        {
            webview_window_builder = webview_window_builder.transparent(false);
        }
        #[cfg(not(target_os = "windows"))]
        {
            webview_window_builder = webview_window_builder.transparent(true);
        }
        let webview_window = webview_window_builder
            .build()
            .map_err(|e| t!("main.failed_to_create_settings_window", error = e))?;

        if let Ok(Some(monitor)) = webview_window.current_monitor() {
            webview_window
                .set_max_size(Some(tauri::Size::Logical(LogicalSize {
                    width: 650.0,
                    height: monitor.size().height as f64,
                })))
                .map_err(|e| t!("main.failed_to_set_max_window_size", error = e))?;
        }

        let _ = webview_window.show();
        let _ = webview_window.set_focus();

        tauri::async_runtime::spawn(async move {
            if let Err(e) = crate::window::fix_window_visual(&webview_window, None).await {
                log::error!("Failed to fix settings window visual: {}", e);
            }
        });
    }

    Ok(())
}

/// Internal function to create or focus URL window
///
/// Creates a new window to display the specified URL
///
/// # Arguments
/// - `app_handle` - The Tauri application handle
/// - `url` - The URL to display
///
/// # Returns
/// - `Result<(), String>` - Ok if successful, Err with error message if failed
pub async fn create_or_focus_url_window(
    app_handle: tauri::AppHandle,
    url: &str,
) -> Result<(), String> {
    let window_label = "webview";

    if let Some(window) = app_handle.get_webview_window(window_label) {
        // Update the URL if the window already exists
        if let Err(e) = window.eval(&format!("window.location.href = '{}';", url)) {
            return Err(t!("main.failed_to_navigate_to_url", url = url, error = e).to_string());
        }

        // Ensure the window is visible and has focus
        if !window.is_visible().unwrap_or(false) {
            let _ = window.show();
        }
        let _ = window.set_focus();
    } else {
        // Create a new webview window if it doesn't exist
        let webview_window = WebviewWindowBuilder::new(
            &app_handle,
            window_label,
            tauri::WebviewUrl::App(url.into()),
        )
        .title("Web View")
        .inner_size(1200.0, 800.0)
        .min_inner_size(800.0, 600.0)
        .center()
        .build()
        .map_err(|e| t!("main.failed_to_create_webview_window", error = e).to_string())?;

        // Show the window and set focus
        let _ = webview_window.show();
        let _ = webview_window.set_focus();

        // cleanup if window is closed
        // let window_clone = webview_window.clone();
        // webview_window.on_window_event(move |event| match event {
        //     tauri::WindowEvent::Destroyed => {
        //         // Clear all browsing data when window is destroyed
        //         if let Err(e) = window_clone.clear_all_browsing_data() {
        //             log::error!("Failed to clear browsing data: {}", e);
        //         }
        //     }
        //     _ => {}
        // });
    }
    Ok(())
}

/// Register window creation event listeners
///
/// Sets up event listeners for custom window creation events:
/// - create-note-window: Creates or focuses the note window
/// - create-setting-window: Creates or focuses the setting window with specified type
///
/// # Arguments
/// - `app_handle` - The Tauri application handle
pub fn setup_window_creation_handlers(app_handle: tauri::AppHandle) {
    // Get main window once
    let main_window = app_handle
        .get_webview_window("main")
        .expect("Main window not found");

    // Helper function to spawn window creation task
    let spawn_window_task = |task: Pin<Box<dyn Future<Output = Result<(), String>> + Send>>| {
        tauri::async_runtime::spawn(async move {
            if let Err(e) = task.await {
                log::error!("Failed to create window: {}", e);
            }
        });
    };

    // Register note window creation event
    let app_handle_clone = app_handle.clone();
    main_window.listen("create-note-window", move |_| {
        let app = app_handle_clone.clone();
        spawn_window_task(Box::pin(
            async move { create_or_focus_note_window(app).await },
        ));
    });

    // Register setting window creation event
    let app_handle_clone = app_handle.clone();
    main_window.listen("create-setting-window", move |event| {
        let app = app_handle_clone.clone();
        let setting_type = serde_json::from_str::<SettingWindowPayload>(event.payload())
            .unwrap_or_else(|_| SettingWindowPayload {
                setting_type: "general".to_string(),
            })
            .setting_type;

        spawn_window_task(Box::pin(async move {
            create_or_focus_setting_window(app, Some(&setting_type)).await
        }));
    });

    // Register URL window creation event
    main_window.listen("create-url-window", move |event| {
        let app = app_handle.clone();
        let url = serde_json::from_str::<UrlWindowPayload>(event.payload())
            .unwrap_or_else(|_| UrlWindowPayload {
                url: "https://www.aidyou.ai".to_string(),
            })
            .url;

        spawn_window_task(Box::pin(async move {
            create_or_focus_url_window(app, &url).await
        }));
    });
}

/// Restores window size and position configuration to a window
///
/// # Arguments
/// * `window` - The window to apply configuration to
/// * `main_store` - The main store
pub fn restore_window_config(window: &WebviewWindow, main_store: &Arc<Mutex<MainStore>>) {
    let mut current_window_size = window.outer_size().unwrap_or_else(|e| {
        warn!(
            "Failed to get initial window outer size for '{}': {}. Using default 800x600.",
            window.label(),
            e
        );
        PhysicalSize::new(800, 600) // Default size if current size cannot be obtained
    });

    if let Ok(c) = main_store.lock() {
        // restore window size
        // Since a Some(default) is provided, get_config should always return Some.
        // .unwrap() is safe here assuming get_config honors the default on missing/error.
        let saved_size = c
            .get_config(CFG_WINDOW_SIZE, Some(WindowSize::default()))
            .unwrap();
        if saved_size.width > 0.0 && saved_size.height > 0.0 {
            let new_logical_size = LogicalSize::new(saved_size.width, saved_size.height);
            if let Err(e) = window.set_size(tauri::Size::Logical(new_logical_size)) {
                warn!("Failed to set window size: {}", e);
            }

            #[cfg(debug_assertions)]
            {
                log::debug!(
                    "Window size set to: {}x{} (logical)",
                    saved_size.width,
                    saved_size.height
                );
            }

            // Update current_window_size to physical for position check
            if let Ok(scale_factor) = window.scale_factor() {
                current_window_size = new_logical_size.to_physical(scale_factor);
            } else {
                warn!("Failed to get scale factor, position check might be less accurate.");
            }

            let window_clone = window.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::window::fix_window_visual(
                    &window_clone,
                    Some(saved_size), // Pass the saved logical size
                )
                .await
                {
                    log::error!(
                        "Failed to fix window visual for '{}': {}",
                        window_clone.label(),
                        e
                    );
                }
            });
        }

        // restore window position
        let window_position_config =
            c.get_config(CFG_WINDOW_POSITION, MainWindowPosition::default()); // default 直接是 MainWindowPosition
        let saved_pos = window_position_config;

        #[cfg(debug_assertions)]
        debug!(
            "Attempting to restore window '{}' to position: ({}, {}) on screen '{}'",
            window.label(),
            saved_pos.x,
            saved_pos.y,
            saved_pos.screen_name.as_deref().unwrap_or("N/A")
        );

        // --- Window Position Restoration Logic ---
        // The following logic attempts to restore the window to its last saved position.
        // However, a critical consideration is the stability of the underlying windowing library (`tao`),
        // especially on macOS when dealing with virtual screens created by third-party software.
        // `tao` can panic if `-[NSWindow screen]` returns NULL, which can happen if a window
        // is positioned on certain areas of a virtual screen, even if that area is logically
        // reported as part of the screen by `available_monitors()`.
        //
        // To mitigate this, we employ a multi-step validation:
        // 1. Basic Check: Ensure the saved position is not (0,0) (usually a default/uninitialized state).
        // 2. On-Screen Check: Use `is_position_on_any_screen` to verify if the saved position
        //    intersects with any reported monitor (physical or virtual).
        // 3. Suspicious Virtual Screen Check (Heuristic): If the window intersects with a monitor,
        //    further check if this monitor exhibits characteristics of a problematic virtual screen
        //    (e.g., non-primary and excessively large dimensions). This is a heuristic to identify
        //    screens that might lead to the `tao` panic.
        //
        // If the position is deemed "unsafe" by these checks, the window is centered on the
        // primary physical monitor as a fallback to prevent the application from crashing.

        // Only attempt to restore if saved_pos is not the default (0,0)
        // and it's on a screen. Otherwise, center it.
        if saved_pos.x != 0 || saved_pos.y != 0 {
            let mut position_is_considered_safe = false;

            // Validate the saved position before applying it
            if is_position_on_any_screen(
                window.app_handle(),
                saved_pos.x,
                saved_pos.y,
                current_window_size, // Use the (potentially updated) physical size
            ) {
                // Position is on *some* screen according to `is_position_on_any_screen`.
                // Now, apply heuristics to check if it's a potentially problematic (e.g., large virtual) screen
                // that might cause issues with lower-level OS calls (`-[NSWindow screen]`).
                position_is_considered_safe = true; // Assume safe initially

                // Heuristic: Define a threshold for what might be an excessively large virtual monitor width
                const SUSPICIOUSLY_LARGE_WIDTH: u32 = 6500; // e.g., wider than a Pro Display XDR

                if let Ok(monitors) = window.app_handle().available_monitors() {
                    if let Ok(primary_monitor_opt) = window.app_handle().primary_monitor() {
                        let primary_monitor_name =
                            primary_monitor_opt.as_ref().and_then(|m| m.name());

                        #[cfg(debug_assertions)]
                        debug!(
                            "Performing suspicious screen check. Primary monitor: {:?}",
                            primary_monitor_name
                        );

                        for monitor in monitors {
                            let monitor_rect = Rect {
                                x: monitor.position().x,
                                y: monitor.position().y,
                                width: monitor.size().width as i32,
                                height: monitor.size().height as i32,
                            };
                            let window_rect_to_check = Rect {
                                x: saved_pos.x,
                                y: saved_pos.y,
                                width: current_window_size.width as i32,
                                height: current_window_size.height as i32,
                            };

                            if window_rect_to_check.intersects(&monitor_rect) {
                                let is_primary = primary_monitor_name == monitor.name();
                                if monitor.size().width > SUSPICIOUSLY_LARGE_WIDTH && !is_primary {
                                    // This log should remain a `warn` as it indicates a significant deviation from normal restoration.
                                    warn!(
                                        "Window at ({},{}) intersects with a non-primary, suspiciously large screen: {:?} (Size: {}x{}). Considering position unsafe.",
                                        saved_pos.x, saved_pos.y, monitor.name(), monitor.size().width, monitor.size().height
                                    );
                                    position_is_considered_safe = false;
                                    break; // Found a suspicious screen, no need to check others
                                }
                            }
                        }
                    } else {
                        warn!("Could not get primary monitor information for safety check.");
                        // If we can't get primary monitor info, it's harder to apply the "non-primary" part of the heuristic.
                        // For now, if is_position_on_any_screen was true, we might still consider it safe,
                        // or one could choose to be more conservative here and set position_is_considered_safe = false.
                        // Current logic: relies on the initial `is_position_on_any_screen` if this fails.
                    }
                } else {
                    warn!("Could not get available monitors for safety check.");
                    // If we can't get monitor list, we can't perform the suspicious screen check.
                    // Rely on the initial is_position_on_any_screen result.
                } // End of suspicious screen check
            }

            if position_is_considered_safe {
                if let Err(e) = window.set_position(tauri::Position::Physical(
                    PhysicalPosition::new(saved_pos.x, saved_pos.y),
                )) {
                    warn!(
                        "Failed to set window position for '{}' to ({}, {}): {}. Centering.",
                        window.label(),
                        saved_pos.x,
                        saved_pos.y,
                        e
                    );
                    if let Err(center_err) = window.center() {
                        error!(
                            "Failed to center window '{}' after set_position failed: {}",
                            window.label(),
                            center_err
                        );
                    }
                } else {
                    #[cfg(debug_assertions)]
                    debug!(
                        "Window '{}' position restored to: ({}, {})",
                        window.label(),
                        saved_pos.x,
                        saved_pos.y
                    );
                }
            } else {
                warn!(
                        "Saved window position ({}, {}) for '{}' is off-screen or on a suspicious virtual screen. Centering window instead.",
                        saved_pos.x,
                        saved_pos.y,
                        window.label(),
                    );
                if let Err(e) = window.center() {
                    error!("Failed to center window '{}': {}", window.label(), e);
                    // Consider what to do if even centering fails, though it's rare.
                }
            }
        } else {
            // Saved position is (0,0), which we treat as "center" or "unspecified"
            #[cfg(debug_assertions)] // Changed to debug as this is a common case for first launch or reset
            debug!(
                "Saved position for window '{}' is (0,0) or default. Centering window.",
                window.label(),
            );
            if let Err(e) = window.center() {
                error!("Failed to center window '{}': {}", window.label(), e);
            }
        }
    }
}

/// Get the current window position
///
/// # Arguments
/// - `window` - The window to get the position of.
///
/// # Returns
/// - `Option<String>` - The current screen name, or None if the window is not found.
pub fn get_screen_name(window: &Window) -> Option<String> {
    // IMPORTANT: This function cannot prevent the panic if window.current_monitor()
    // itself panics internally due to the window being off-screen on macOS.
    // The primary fix is to ensure windows are restored to valid screen positions.
    match window.current_monitor() {
        Ok(Some(monitor)) => {
            let name = monitor.name().map(|s| s.to_string());

            #[cfg(debug_assertions)]
            {
                debug!(
                    "Window '{}' is on monitor: {:?} (Position: {:?}, Size: {:?})",
                    window.label(),
                    name,
                    monitor.position(),
                    monitor.size()
                );
            }

            name
        }
        Ok(None) => {
            warn!(
                "Window '{}' is not on any screen (current_monitor returned Ok(None)).",
                window.label()
            );
            None
        }
        Err(e) => {
            error!(
                "Error getting current monitor for window '{}': {}",
                window.label(),
                e
            );
            None
        }
    }
}
