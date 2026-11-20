//! Z-order controls for the existing native toplevel.

use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
