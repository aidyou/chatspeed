//! Standalone X11/XWayland smoke test for a Wry child hosted by GPUI Fast.

#[cfg(target_os = "linux")]
mod linux {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use gpui_kit::{canvas, prelude::*, Context, Render, Window, WindowOptions};
    use raw_window_handle::{
        HandleError, HasWindowHandle, RawWindowHandle, WindowHandle, XlibWindowHandle,
    };
    use wry::{
        dpi::{PhysicalPosition, PhysicalSize},
        NewWindowResponse, Rect, WebView, WebViewBuilder,
    };

    struct X11Parent<'a> {
        _window: WindowHandle<'a>,
        xlib: XlibWindowHandle,
    }

    impl HasWindowHandle for X11Parent<'_> {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            // SAFETY: The original borrowed handle keeps the same X11 window valid.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Xlib(self.xlib)) })
        }
    }

    fn attach(window: &Window) -> Result<WebView, Box<dyn std::error::Error>> {
        let handle = HasWindowHandle::window_handle(window)?;
        let xlib = match handle.as_raw() {
            RawWindowHandle::Xlib(handle) => handle,
            RawWindowHandle::Xcb(handle) => XlibWindowHandle::new(handle.window.get().into()),
            _ => return Err("The WebView probe requires X11 or XWayland".into()),
        };
        let parent = X11Parent {
            _window: handle,
            xlib,
        };
        Ok(WebViewBuilder::new()
            .with_incognito(true)
            .with_bounds(Rect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new(1, 1).into(),
            })
            .with_html("<!doctype html><meta charset=utf-8><main><input aria-label='WebView probe'><p>GPUI Fast + Wry</p></main>")
            .with_navigation_handler(|url| url == "about:blank")
            .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
            .build_as_child(&parent)?)
    }

    struct Probe(Rc<WebView>, Rc<Cell<Option<(u32, u32)>>>);

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let view = self.0.clone();
            let last_size = self.1.clone();
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let scale = window.scale_factor();
                    let width = (f32::from(bounds.size.width) * scale).round() as u32;
                    let height = (f32::from(bounds.size.height) * scale).round() as u32;
                    if width == 0 || height == 0 || last_size.get() == Some((width, height)) {
                        return;
                    }
                    match view.set_bounds(Rect {
                        position: PhysicalPosition::new(0, 0).into(),
                        size: PhysicalSize::new(width, height).into(),
                    }) {
                        Ok(()) => last_size.set(Some((width, height))),
                        Err(error) => eprintln!("WebView resize failed: {error}"),
                    }
                },
            )
            .size_full()
        }
    }

    pub fn run() {
        gtk::gdk::set_allowed_backends("x11");
        if let Err(error) = gtk::init() {
            eprintln!("GTK/X11 initialization failed: {error}");
            std::process::exit(1);
        }
        gpui_kit::platform::linux(gpui_kit::WindowingModes::X11).run(|cx| {
            let result = cx.open_window(WindowOptions::default(), |window, cx| {
                let view =
                    attach(window).expect("WebView probe could not attach to the GPUI window");
                cx.new(|_| Probe(Rc::new(view), Rc::new(Cell::new(None))))
            });
            if let Err(error) = result {
                eprintln!("GPUI window creation failed: {error}");
                cx.quit();
                return;
            }
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            cx.spawn(async move |cx| loop {
                for _ in 0..64 {
                    if !gtk::events_pending() {
                        break;
                    }
                    gtk::main_iteration_do(false);
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            })
            .detach();
            cx.activate(true);
        });
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    linux::run();
    #[cfg(not(target_os = "linux"))]
    eprintln!("The WebView probe currently supports Linux X11/XWayland only");
}
