use std::future::Future;
use std::pin::Pin;

use log::{error, warn};
use rust_i18n::t;
use serde::Deserialize;
use tauri::Listener;
use tauri::LogicalSize;
use tauri::Manager;
use tauri::PhysicalSize;
use tauri::WebviewWindowBuilder;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
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
    #[cfg(target_os = "macos")]
    {
        use tauri::LogicalSize;
        let mut size = _size
            .map(|s| LogicalSize::new(s.width as f64, s.height as f64))
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
/// # Parameters
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

        if let Err(e) = window.show() {
            warn!("Failed to show assistant window: {}", e);
        }
        if let Err(e) = window.set_focus() {
            warn!("Failed to set focus to assistant window: {}", e);
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
                if let Err(e) = window.set_focus() {
                    warn!("Failed to set focus to main window: {}", e);
                }
                if let Ok(is_minimized) = window.is_minimized() {
                    if is_minimized {
                        if let Err(e) = window.unminimize() {
                            warn!("Failed to unminimize main window: {}", e);
                        }
                    }
                }
                // if let Err(e) = window.show() {
                //     warn!("Failed to show main window: {}", e);
                // }
            }
        } else {
            warn!("Failed to determine visibility of assistant window");
        }
    }
}

/// Internal function to create or open note window
///
/// This function is used to open a new note window, or if the window already exists, it displays and focuses the window.
///
/// # Parameters
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
                .skip_taskbar(true)
                .inner_size(850.0, 600.0)
                .min_inner_size(600.0, 400.0);
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
                log::error!("{}", t!("main.failed_to_fix_note_window_visual", error = e));
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
/// # Parameters
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
        .skip_taskbar(true)
        .maximizable(false)
        .inner_size(650.0, 700.0)
        .min_inner_size(650.0, 600.0);

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
                log::error!("{}", t!("main.failed_to_fix_window_visual", error = e));
            }
        });
    }

    Ok(())
}

/// Internal function to create or focus URL window
///
/// Creates a new window to display the specified URL
///
/// # Parameters
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
/// # Parameters
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
