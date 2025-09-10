#[cfg(test)]
mod tests {
    use rust_i18n::set_locale;

    use crate::{
        constants::CFG_INTERFACE_LANGUAGE,
        db::{self, MainStore},
        libs,
        logger::setup_test_logger,
    };

    #[small_ctor::ctor]
    unsafe fn init() {
        let _ = setup_test_logger();

        // Setup language
        let main_store = MainStore::new(super::get_db_path())
            .map_err(|e| db::StoreError::IoError(e.to_string()))
            .expect("Failed to create main store");
        let user_lang =
            main_store.get_config(CFG_INTERFACE_LANGUAGE, libs::lang::get_system_locale());
        if !user_lang.is_empty() {
            set_locale(&user_lang);
            log::info!("Set interace language to {}", user_lang);
        }
    }
}

// use lazy_static::*;
use std::sync::Arc;

fn get_db_path() -> std::path::PathBuf {
    let db_path = {
        let dev_dir = &*crate::STORE_DIR.read();
        dev_dir.join("chatspeed.db")
    };
    db_path
}

pub fn get_app_handle() -> tauri::AppHandle<tauri::test::MockRuntime> {
    let main_store = crate::db::MainStore::new(get_db_path()).expect("Failed to create main store");
    let main_store = Arc::new(std::sync::RwLock::new(main_store));
    let window_channels = Arc::new(crate::libs::window_channels::WindowChannels::new());
    let chat_state = crate::ai::interaction::chat_completion::ChatState::new(
        window_channels,
        None,
        main_store.clone(),
    );

    let app = tauri::test::mock_builder()
        .manage(main_store)
        .manage(chat_state)
        .build(tauri::test::mock_context(tauri::test::noop_assets()));

    app.unwrap().handle().clone()
}

// lazy_static! {
//     pub static ref MOCK_APP_HANDLE: Arc<tauri::AppHandle<tauri::test::MockRuntime>> =
//         Arc::new(get_app_handle());
// }
