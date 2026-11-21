//! Z-order controls for the existing native toplevel.

use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub fn geometry_values(
    _window: &Window,
    width: f32,
    height: f32,
) -> Vec<(&'static str, serde_json::Value)> {
    let size = (
        chatspeed_gpui::prefs::WORKFLOW_WINDOW_SIZE_KEY,
        serde_json::json!({"width": width, "height": height}),
    );
    #[cfg(target_os = "windows")]
    if let Some(position) = window_position(_window) {
        return vec![size, ("workflow_window_position", position)];
    }
    vec![size]
}

#[cfg(target_os = "windows")]
fn window_position(window: &Window) -> Option<serde_json::Value> {
    use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let mut frame = RECT::default();
    if unsafe { IsIconic(hwnd) != 0 || GetWindowRect(hwnd, &mut frame) == 0 } {
        return None;
    }
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
    let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if monitor.is_null() || unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) } == 0 {
        return None;
    }
    let work = info.monitorInfo.rcWork;
    if frame.left < work.left
        || frame.top < work.top
        || frame.right > work.right
        || frame.bottom > work.bottom
    {
        return None;
    }
    let end = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    Some(
        serde_json::json!({"x": frame.left, "y": frame.top, "screen_name": String::from_utf16_lossy(&info.szDevice[..end])}),
    )
}

/// Move only when the saved monitor name and the entire outer frame match a
/// current work area. Wayland does not expose absolute toplevel positioning.
#[cfg(target_os = "windows")]
pub fn restore_position(window: &Window, x: i32, y: i32, name: &str) {
    use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};
    struct Candidate<'a> {
        name: &'a str,
        x: i64,
        y: i64,
        width: i64,
        height: i64,
        fits: bool,
    }
    unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
        // EnumDisplayMonitors invokes this callback synchronously.
        let candidate = unsafe { &mut *(data as *mut Candidate<'_>) };
        let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(monitor, &mut info.monitorInfo) } != 0 {
            let end = info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len());
            let work = info.monitorInfo.rcWork;
            candidate.fits |= String::from_utf16_lossy(&info.szDevice[..end]) == candidate.name
                && candidate.x >= i64::from(work.left)
                && candidate.y >= i64::from(work.top)
                && candidate.x + candidate.width <= i64::from(work.right)
                && candidate.y + candidate.height <= i64::from(work.bottom);
        }
        1
    }
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as HWND;
    let mut frame = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut frame) } == 0 {
        return;
    }
    // Refuse crosses between different DPI scales: moving there could enlarge
    // the frame after validation. Monitor DPI is evaluated by Windows itself.
    let mut candidate = Candidate {
        name,
        x: x.into(),
        y: y.into(),
        width: i64::from(frame.right) - i64::from(frame.left),
        height: i64::from(frame.bottom) - i64::from(frame.top),
        fits: false,
    };
    if candidate.width <= 0 || candidate.height <= 0 {
        return;
    }
    let current_monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
    let saved_monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL) };
    if current_monitor != saved_monitor || saved_monitor.is_null() {
        return;
    }
    if unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            std::ptr::null(),
            Some(visit),
            &mut candidate as *mut _ as LPARAM,
        )
    } != 0
        && candidate.fits
    {
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }
}

pub fn supported(window: &Window) -> bool {
    HasWindowHandle::window_handle(window).is_ok_and(|handle| match handle.as_raw() {
        #[cfg(target_os = "linux")]
        RawWindowHandle::Xcb(_) | RawWindowHandle::Xlib(_) => true,
        #[cfg(target_os = "windows")]
        RawWindowHandle::Win32(_) => true,
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(_) => true,
        _ => false,
    })
}

pub fn set(window: &Window, pinned: bool) -> Result<(), Box<dyn std::error::Error>> {
    match HasWindowHandle::window_handle(window)?.as_raw() {
        #[cfg(target_os = "linux")]
        RawWindowHandle::Xcb(handle) => set_x11(handle.window.get(), pinned),
        #[cfg(target_os = "linux")]
        RawWindowHandle::Xlib(handle) => set_x11(handle.window.try_into()?, pinned),
        #[cfg(target_os = "windows")]
        RawWindowHandle::Win32(handle) => {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            // The HWND is borrowed from the live GPUI window on its UI thread.
            let ok = unsafe {
                SetWindowPos(
                    handle.hwnd.get() as _,
                    if pinned { HWND_TOPMOST } else { HWND_NOTOPMOST },
                    0,
                    0,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE,
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(())
        }
        #[cfg(target_os = "macos")]
        RawWindowHandle::AppKit(handle) => {
            // AppKit supplies an NSView; borrow it only on the GPUI main thread.
            let view = unsafe { &*handle.ns_view.cast::<objc2_app_kit::NSView>().as_ptr() };
            let window = view.window().ok_or("view has no native window")?;
            window.setLevel(if pinned { 3 } else { 0 });
            Ok(())
        }
        _ => Err("window stacking is not supported by this backend".into()),
    }
}

#[cfg(target_os = "linux")]
fn set_x11(window: u32, pinned: bool) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::{connection::Connection, protocol::xproto::*};
    let (conn, _) = x11rb::connect(None)?;
    let root = conn.get_geometry(window)?.reply()?.root;
    let state = conn.intern_atom(false, b"_NET_WM_STATE")?.reply()?.atom;
    let above = conn
        .intern_atom(false, b"_NET_WM_STATE_ABOVE")?
        .reply()?
        .atom;
    let event = ClientMessageEvent::new(32, window, state, [u32::from(pinned), above, 0, 1, 0]);
    conn.send_event(
        false,
        root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    )?
    .check()?;
    conn.flush()?;
    Ok(())
}
