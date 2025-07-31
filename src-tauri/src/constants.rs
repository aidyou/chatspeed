use lazy_static::*;
use parking_lot::RwLock as PLRwLock;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// The main window info
pub const CFG_WINDOW_POSITION: &str = "window_position";
pub const CFG_WINDOW_SIZE: &str = "window_size";

pub const TRAY_ID: &str = "Chatspeed";

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
// chatspeed crawler api name
pub const CFG_CHP_SERVER: &str = "chatspeed_crawler";
pub const CFG_SEARCH_ENGINE: &str = "search_engine";
pub const CFG_CCPROXY_PORT: &str = "ccproxy_port";
pub const CFG_CCPROXY_PORT_DEFAULT: u16 = 11434;

// main window shortcuts
pub const CFG_MAIN_WINDOW_VISIBLE_SHORTCUT: &str = "main_window_visible_shortcut";
pub const DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT: &str = "F2";
pub const CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT: &str = "assistant_window_visible_shortcut";
pub const DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+Z";
pub const CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT: &str =
    "assistant_window_visible_and_paste_shortcut";
pub const DEFAULT_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT: &str = "Alt+S";
pub const CFG_NOTE_WINDOW_VISIBLE_SHORTCUT: &str = "note_window_visible_shortcut";
pub const DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+N";

pub const DEFAULT_THUMBNAIL_WIDTH: u32 = 200;
pub const DEFAULT_THUMBNAIL_HEIGHT: u32 = 200;

// assistant window always on top status
pub static ASSISTANT_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);
// main window always on top status
pub static MAIN_WINDOW_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);
// on mouse event status
pub static ON_MOUSE_EVENT: AtomicBool = AtomicBool::new(false);

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
    // log dir: ${app_data}/log, it will be initialized at startup in @src-tauri/src/logger.rs L108 `setup_logger`
    pub static ref LOG_DIR: Arc<PLRwLock<PathBuf>> = Arc::new(PLRwLock::new(PathBuf::new()));

    // Just for Development environment data directory
    pub static ref STORE_DIR: Arc<PLRwLock<PathBuf>> = {
        #[cfg(debug_assertions)]
        {
            use std::env;
            let project_root = if cfg!(test) {
                // In test environment, get project root directory from environment variable
                env::var("PROJECT_ROOT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| env::current_dir().expect("Failed to get current directory")).parent().unwrap().into()
            } else {
                // In development and production environments, get project root from CARGO_MANIFEST_DIR
                // CARGO_MANIFEST_DIR points to src-tauri/, so we need to go up one level to get project root
                let manifest_dir = env::var("CARGO_MANIFEST_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| env::current_dir().expect("Failed to get current directory"));

                log::debug!("CARGO_MANIFEST_DIR: {:?}", manifest_dir);

                // Check if we're in a Tauri project structure (src-tauri directory)
                let project_root = if manifest_dir.file_name().and_then(|n| n.to_str()) == Some("src-tauri") {
                    // Go up one level to get the project root
                    let root = manifest_dir.parent().unwrap_or(&manifest_dir).to_path_buf();
                    log::debug!("Detected Tauri project structure, using project root: {:?}", root);
                    root
                } else {
                    // If not in src-tauri, use the manifest directory as-is
                    log::debug!("Not in Tauri project structure, using manifest dir as root: {:?}", manifest_dir);
                    manifest_dir
                };
                project_root
            };
            let path = PathBuf::from(project_root).join("dev_data");
            log::debug!("STORE_DIR will be: {:?}", path);

            // Create directory if it doesn't exist
            if !path.exists() {
                if let Err(e) = std::fs::create_dir_all(&path) {
                    log::error!("Failed to create dev-data directory: {}", e);
                } else {
                    log::debug!("Created dev_data directory at: {:?}", path);
                }
            } else {
                log::debug!("dev_data directory already exists at: {:?}", path);
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
