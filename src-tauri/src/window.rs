use log::{error, warn};
use tauri::LogicalSize;
use tauri::Manager;
use tauri::PhysicalSize;
use tauri::WebviewWindowBuilder;

#[derive(Clone, Copy)]
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
    window: &tauri::WebviewWindow,
    size: Option<WindowSize>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    {
        let mut size = size
            .map(|s| LogicalSize::new(s.width as f64, s.height as f64))
            .unwrap_or_else(|| {
                window
                    .inner_size()
                    .unwrap_or(PhysicalSize::new(0, 0))
                    .to_logical(window.scale_factor().unwrap_or(1.0))
            });

        if size.width == 0.0 || size.height == 0.0 {
            return Ok(());
        }

        size.height += 1.0;
        window.set_size(tauri::Size::Logical(size))?;
        log::info!("Window size set to: {}x{}", size.width, size.height);

        // wait for window to be resized
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

        size.height -= 1.0;
        log::info!("Window size restored to: {}x{}", size.width, size.height);
        window.set_size(tauri::Size::Logical(size))?;
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
                // 添加视觉修复
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
