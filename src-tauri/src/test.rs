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

        let db_path = {
            let dev_dir = &*crate::STORE_DIR.read();
            dev_dir.join("chatspeed.db")
        };

        // Setup language
        let main_store = MainStore::new(db_path)
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
