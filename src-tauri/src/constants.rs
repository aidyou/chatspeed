use lazy_static::*;
use parking_lot::RwLock as PLRwLock;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub const CFG_WINDOW_WIDTH: &str = "window_width";
pub const CFG_WINDOW_HEIGHT: &str = "window_height";
pub const TRAY_ID: &str = "chatspeed";

// Auto update config
pub const CFG_AUTO_UPDATE: &str = "auto_update";

// =================================================
// Core plugin identifiers
// =================================================
// uncomment when the workflow is ready
// pub const CORE_PLUGIN_HTTP_CLIENT: &str = "http_client";
// pub const CORE_PLUGIN_STORE: &str = "store";
// pub const CORE_PLUGIN_SELECTOR: &str = "selector";
// pub const PYTHON_RUNTIME: &str = "python_runtime";
// pub const DENO_RUNTIME: &str = "deno_runtime";

// interface language
pub const CFG_INTERFACE_LANGUAGE: &str = "interface_language";
// pub const CFG_WORD_SELECTION_TOOLBAR: &str = "word_selection_toolbar";

// main window shortcuts
pub const CFG_MAIN_WINDOW_VISIBLE_SHORTCUT: &str = "main_window_visible_shortcut";
pub const DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT: &str = "F2";
pub const CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT: &str = "assistant_window_visible_shortcut";
pub const DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+Z";
pub const CFG_NOTE_WINDOW_VISIBLE_SHORTCUT: &str = "note_window_visible_shortcut";
pub const DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+N";

pub const DEFAULT_THUMBNAIL_WIDTH: u32 = 200;
pub const DEFAULT_THUMBNAIL_HEIGHT: u32 = 200;

// assistant window always on top status
pub static ASSISTANT_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);
// main window always on top status
pub static MAIN_WINDOW_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);

// The following static variables are used to store the paths of the http server and related directories
// They are initialized after the http server is initialized,
// more details see `src-tauri/src/http/server.rs` `start_http_server()`
lazy_static! {
    // http server, like http://127.0.0.1:21914
    pub static ref HTTP_SERVER: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::new()));
    // http server dir: ${app_data}/static
    pub static ref HTTP_SERVER_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::new()));
    // http server tmp dir: ${app_data}/static/tmp
    pub static ref HTTP_SERVER_TMP_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));
    // http server theme dir: ${app_data}/static/theme
    pub static ref HTTP_SERVER_THEME_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));
    // http server upload dir: ${app_data}/static/upload
    pub static ref HTTP_SERVER_UPLOAD_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));
    // plugins dir: ${app_data}/plugins
    pub static ref PLUGINS_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));
    // shared data dir: ${app_data}/shared
    pub static ref SHARED_DATA_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));

    // Development environment data directory
    pub static ref STORE_DIR: Arc<PLRwLock<PathBuf>> = {
        #[cfg(debug_assertions)]
        {
            let path = PathBuf::from("dev_data");
            // Create directory if it doesn't exist
            if !path.exists() {
                if let Err(e) = std::fs::create_dir_all(&path) {
                    log::error!("Failed to create dev-data directory: {}", e);
                }
            }
            Arc::new(PLRwLock::new(path))
        }
        #[cfg(not(debug_assertions))]
        {
            Arc::new(PLRwLock::new(PathBuf::new()))
        }
    };
}

/// read the value from the RwLock or return the default value if the lock cannot be acquired
pub fn get_static_var<T: Clone>(var: &Arc<PLRwLock<T>>) -> T {
    var.read().clone()
}
