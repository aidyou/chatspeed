#[cfg(test)]
mod tests {
    use rust_i18n::set_locale;

    #[small_ctor::ctor]
    unsafe fn init() {
        // Deterministic, dev-data-free harness: pin a pure English locale so no
        // test depends on the machine locale, and touch no database or log file.
        // (The previous ctor opened the development store and the file logger.)
        set_locale("en");
    }

    #[tokio::test]
    async fn mock_runtime_app_handle_builds() {
        let _app_handle = super::get_app_handle();
    }

    /// The ChatHub page widens the window it is docked in, so a remembered window size has to
    /// leave the width of that page out.
    #[test]
    fn a_remembered_window_width_leaves_the_docked_page_width_out() {
        // A window without a docked page is remembered as it is measured.
        assert_eq!(crate::remembered_width(1024.0, 0.0), 1024.0);
        // A window holding a page hands the width of that page back.
        assert_eq!(crate::remembered_width(1624.0, 600.0), 1024.0);
        // The width the workflow UI keeps next to a page is the floor of what is remembered.
        assert_eq!(
            crate::remembered_width(800.0, 600.0),
            crate::chat_hub::CHAT_HUB_MIN_HOST_WIDTH
        );
        // A page wider than the window it is docked in never produces a nonsense width.
        assert_eq!(crate::remembered_width(500.0, 600.0), 500.0);
    }
}

// use lazy_static::*;

/// Builds a state-free Tauri mock app for tests.
///
/// No `MainStore` or `ChatState` is injected: the desktop tests must never open
/// the runtime database, and the chat state is runtime-owned.
pub fn get_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
    let app =
        tauri::test::mock_builder().build(tauri::test::mock_context(tauri::test::noop_assets()));
    app.expect("build the mock Tauri app").handle().clone()
}

// lazy_static! {
//     pub static ref MOCK_APP_HANDLE: Arc<tauri::AppHandle<tauri::test::MockRuntime>> =
//         Arc::new(get_app_handle());
// }
