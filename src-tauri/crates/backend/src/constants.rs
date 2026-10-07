use lazy_static::*;
use parking_lot::RwLock as PLRwLock;
#[cfg(any(debug_assertions, test))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// The main window info
pub const CFG_WINDOW_POSITION: &str = "window_position";
pub const CFG_WINDOW_SIZE: &str = "window_size";
pub const CFG_ASSISTANT_WINDOW_SIZE: &str = "assistant_window_size";
pub const CFG_WORKFLOW_WINDOW_SIZE: &str = "workflow_window_size";
pub const CFG_PROXY_SWITCHER_WINDOW_SIZE: &str = "proxy_switcher_window_size";
pub const CFG_WORKFLOW_WINDOW_POSITION: &str = "workflow_window_position";

pub const TRAY_ID: &str = "Chatspeed";

// Startup and update config
pub const CFG_AUTO_START: &str = "auto_start";
pub const CFG_AUTO_UPDATE: &str = "auto_update";
pub const CFG_WORKFLOW_PREVENT_IDLE_SLEEP: &str = "workflow_prevent_idle_sleep";

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
#[cfg(not(feature = "desktop"))]
pub const CFG_CHAT_COMPLETION_PROXY: &str = "chat_completion_proxy";
#[cfg(not(feature = "desktop"))]
pub const CFG_ACTIVE_PROXY_GROUP: &str = "active_proxy_group";
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_PORT: &str = "chat_completion_proxy_port";
pub const CFG_CCPROXY_PORT_DEFAULT: u16 = 11435;
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_LISTEN: &str = "chat_completion_proxy_listen";
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_LISTEN_DEFAULT: &str = "127.0.0.1";
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_LOG_TO_FILE: &str = "chat_completion_proxy_log_to_file";
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_LOG_PROXY_TO_FILE: &str = "chat_completion_proxy_log_proxy_to_file";
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_RETRY_ON_429: &str = "chat_completion_proxy_retry_on_429";
#[cfg(not(feature = "desktop"))]
pub const CFG_CCPROXY_RETRY_ON_429_DEFAULT: u32 = 0;
pub const CFG_SEARCH_ENGINE: &str = "search_engine";
pub const CFG_SCRAPER_DEBUG_MODE: &str = "scraper_debug_mode";
#[cfg(not(feature = "desktop"))]
pub const DEFAULT_WEB_SEARCH_TOOL: &str = "WebSearch";
#[cfg(not(feature = "desktop"))]
pub const DEFAULT_WEB_FETCH_TOOL: &str = "WebFetch";
// pub const CFG_SCRAPER_CONCURRENCY_COUNT: &str = "scraper_concurrency_count";

//======================================================
//  main window shortcuts
//======================================================
pub const CFG_MAIN_WINDOW_VISIBLE_SHORTCUT: &str = "main_window_visible_shortcut";
pub const DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT: &str = "F2";

pub const CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT: &str = "assistant_window_visible_shortcut";
pub const DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+Z";

pub const CFG_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT: &str =
    "assistant_window_visible_and_paste_shortcut";
pub const DEFAULT_ASSISTANT_WINDOW_VISIBLE_AND_PASTE_SHORTCUT: &str = "Alt+S";

pub const CFG_NOTE_WINDOW_VISIBLE_SHORTCUT: &str = "note_window_visible_shortcut";
pub const DEFAULT_NOTE_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+N";

pub const CFG_MOVE_WINDOW_LEFT_SHORTCUT: &str = "move_window_left_shortcut";
pub const DEFAULT_MOVE_WINDOW_LEFT_SHORTCUT: &str = "Alt+Shift+Left";

pub const CFG_MOVE_WINDOW_RIGHT_SHORTCUT: &str = "move_window_right_shortcut";
pub const DEFAULT_MOVE_WINDOW_RIGHT_SHORTCUT: &str = "Alt+Shift+Right";

pub const CFG_CENTER_WINDOW_SHORTCUT: &str = "center_window_shortcut";
pub const DEFAULT_CENTER_WINDOW_SHORTCUT: &str = "Alt+Shift+C";

//======================================================
// workflow window shortcuts
//======================================================
pub const CFG_WORKFLOW_WINDOW_VISIBLE_SHORTCUT: &str = "workflow_window_visible_shortcut";
pub const DEFAULT_WORKFLOW_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+W";

pub const CFG_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT: &str =
    "proxy_switcher_window_visible_shortcut";
pub const DEFAULT_PROXY_SWITCHER_WINDOW_VISIBLE_SHORTCUT: &str = "Alt+Shift+P";

//======================================================
// end main window shortcuts
//======================================================

pub const DEFAULT_THUMBNAIL_WIDTH: u32 = 200;
pub const DEFAULT_THUMBNAIL_HEIGHT: u32 = 200;

// assistant window always on top status
pub static ASSISTANT_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);
// main window always on top status
pub static MAIN_WINDOW_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);
// workflow window always on top status
pub static WORKFLOW_WINDOW_ALWAYS_ON_TOP: AtomicBool = AtomicBool::new(false);
// on mouse event status
pub static ON_MOUSE_EVENT: AtomicBool = AtomicBool::new(false);

/// The nearest ancestor directory named `src-tauri`, if any.
///
/// Development runs compile from two manifests: the desktop crate at
/// `src-tauri` and the nested runtime-backend crate at
/// `src-tauri/crates/backend`. Anchoring on the nearest `src-tauri` makes both
/// resolve identically without consulting the process working directory.
#[cfg(any(debug_assertions, test))]
fn nearest_src_tauri(manifest_dir: &Path) -> Option<&Path> {
    manifest_dir
        .ancestors()
        .find(|dir| dir.file_name().and_then(|name| name.to_str()) == Some("src-tauri"))
}

/// Repository root for development data, derived purely from a manifest path.
///
/// `src-tauri` is one level below the repository root, so its parent is the
/// repository root; a manifest that is not inside a `src-tauri` tree is used
/// as-is. The result never depends on the runtime environment, so a process
/// launched from any directory resolves the same `dev_data` location.
#[cfg(any(debug_assertions, test))]
fn dev_repository_root(manifest_dir: &Path) -> PathBuf {
    match nearest_src_tauri(manifest_dir) {
        Some(src_tauri) => src_tauri
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| src_tauri.to_path_buf()),
        None => manifest_dir.to_path_buf(),
    }
}

// The following static variables are used to store the paths of the http server and related directories
// They are initialized after the http server is initialized,
// more details see `src-tauri/src/http/server.rs` `start_http_server()`
lazy_static! {
    // HTTP server, default: http://127.0.0.1:21912. If port 21912 is unavailable, it will find an available port.
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
    pub static ref SCHEMA_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));
    // shared data dir: ${app_data}/shared
    pub static ref SHARED_DATA_DIR: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from("")));
    // Log directory: ${dev_data}/logs in development and the platform log directory in production.
    pub static ref LOG_DIR: Arc<PLRwLock<PathBuf>> = Arc::new(PLRwLock::new(PathBuf::new()));
    // chat completion proxy
    pub static ref CHAT_COMPLETION_PROXY: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(String::from(format!("http://localhost:{}", CFG_CCPROXY_PORT_DEFAULT))));
    // internal ccproxy api key
    pub static ref INTERNAL_CCPROXY_API_KEY: Arc<PLRwLock<String>> = Arc::new(PLRwLock::new(uuid::Uuid::new_v4().to_string()));

    // Just for Development environment data directory
    pub static ref STORE_DIR: Arc<PLRwLock<PathBuf>> = {
        #[cfg(debug_assertions)]
        {
            // The anchor comes from the compile-time manifest path so the desktop
            // crate and the nested runtime-backend crate both land on the
            // repository `dev_data` regardless of the process working directory.
            // An explicit `PROJECT_ROOT` override keeps the historical convention
            // of naming a path one level below the anchor (its parent is used).
            let root = match std::env::var("PROJECT_ROOT") {
                Ok(value) => PathBuf::from(value)
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from(".")),
                Err(_) => dev_repository_root(Path::new(env!("CARGO_MANIFEST_DIR"))),
            };
            let path = root.join("dev_data");
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

    // Resource path, bundled assets in production or source assets in development
    pub static ref RESOURCE_DIR: Arc<PLRwLock<PathBuf>> = {
        #[cfg(debug_assertions)]
        {
            // Assets live in `src-tauri/assets`. Resolving from the compile-time
            // manifest path keeps the desktop crate and the nested runtime-backend
            // crate in agreement, and no ambient `CARGO_MANIFEST_DIR` or process
            // working directory can redirect the bundled asset location.
            let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
            let path = match nearest_src_tauri(manifest_dir) {
                Some(src_tauri) => src_tauri.join("assets"),
                None => manifest_dir.join("src-tauri").join("assets"),
            };
            log::debug!("RESOURCE_DIR (dev) will be: {:?}", path);
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

/// Resolve bundled asset subdirectories across development and packaged layouts.
///
/// Development uses `src-tauri/assets/...`, so `RESOURCE_DIR` already points to `assets`.
/// Packaged builds use `Contents/Resources/assets/...`, while older layouts may place
/// files directly under `Contents/Resources/...`. Returning both candidates keeps the
/// lookup logic compatible without forcing callers to guess the bundle layout.
pub fn resolve_resource_subdirs(relative: &str) -> Vec<PathBuf> {
    let resource_dir = RESOURCE_DIR.read().clone();
    if resource_dir.as_os_str().is_empty() {
        return vec![];
    }

    let mut candidates = Vec::new();
    let resource_is_assets =
        resource_dir.file_name().and_then(|name| name.to_str()) == Some("assets");

    candidates.push(resource_dir.join(relative));
    if !resource_is_assets {
        candidates.push(resource_dir.join("assets").join(relative));
    }

    candidates.dedup();
    candidates
}

// When the search results include video or image websites, they are filtered out
pub static VIDEO_AND_IMAGE_DOMAINS: phf::Set<&'static str> = phf::phf_set! {
    // video websites
    "v.qq.com", "iqiyi.com", "youku.com", "imgo.tv", "bilibili.com", "xigua.com",
    "douyin.com", "kuaishou.com", "yspapp.cn", "youtube.com", "vimeo.com",
    "dailymotion.com", "netflix.com", "primevideo.com", "hulu.com", "disneyplus.com",
    "tiktok.com", "twitch.tv", "mgtv.com", "le.com", "acfun.cn", "bilibili.tv",
    // image websites
    "vcg.com", "dfic.cn", "tuchong.com", "zcool.com.cn",
    "ui.cn", "gettyimages.com", "shutterstock.com", "istockphoto.com", "stock.adobe.com",
    "alamy.com", "unsplash.com", "pexels.com", "pixabay.com", "instagram.com",
    "pinterest.com", "flickr.com", "dribbble.com", "behance.net", "freepik.com",
    "stockvault.net", "picjumbo.com", "gratisography.com", "lifeofpix.com",
    "pikwizard.com", "burst.shopify.com", "barnimages.com", "picalls.com",
    "smugmug.com", "deviantart.com", "artstation.com", "picfair.com", "eyeem.com",
};

pub static RESTRICTED_EXTENSIONS: phf::Set<&'static str> = phf::phf_set! {
    ".pdf", ".ppt", ".pptx", ".doc", ".docx", ".xls", ".xlsx", ".mp3", ".mp4", ".avi", ".mov",
    ".wmv", ".flv", ".zip", ".rar", ".7z", ".tar", ".gz", ".bz2", ".iso", ".exe", ".dmg", ".apk",
    ".jpg", ".jpeg", ".png", ".gif", ".bmp", ".tiff", ".webp",
};

// Internal parameter names for tool execution context
// These are injected by the workflow engine and should be removed before tool processing
#[cfg(not(feature = "desktop"))]
pub const INTERNAL_PARAM_TOOL_CALL_ID: &str = "__inner_tool_call_id";

#[cfg(test)]
mod debug_path_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn repository_root_matches_for_desktop_and_nested_backend_manifests() {
        // The desktop crate manifest is `src-tauri`; the runtime backend crate
        // manifest is nested under it. Both must resolve to the same repository
        // root so they share one `dev_data`.
        let desktop = dev_repository_root(Path::new("/repo/src-tauri"));
        let nested = dev_repository_root(Path::new("/repo/src-tauri/crates/backend"));
        assert_eq!(desktop, PathBuf::from("/repo"));
        assert_eq!(nested, PathBuf::from("/repo"));
    }

    #[test]
    fn repository_root_is_independent_of_process_working_directory() {
        // Only the supplied manifest path is consulted, so the same manifest
        // resolves identically no matter which directory the process runs from.
        assert_eq!(
            dev_repository_root(Path::new("/elsewhere/project/src-tauri/crates/backend")),
            PathBuf::from("/elsewhere/project"),
        );
    }

    #[test]
    fn nearest_src_tauri_stops_at_the_closest_ancestor() {
        assert_eq!(
            nearest_src_tauri(Path::new("/repo/src-tauri/crates/backend")),
            Some(Path::new("/repo/src-tauri")),
        );
        // Without a `src-tauri` ancestor the manifest is used as the anchor.
        assert_eq!(nearest_src_tauri(Path::new("/repo/crates/backend")), None);
        assert_eq!(
            dev_repository_root(Path::new("/repo/crates/backend")),
            PathBuf::from("/repo/crates/backend"),
        );
    }

    #[test]
    fn resource_anchor_keeps_assets_under_src_tauri() {
        for manifest in ["/repo/src-tauri", "/repo/src-tauri/crates/backend"] {
            let manifest = Path::new(manifest);
            let assets = match nearest_src_tauri(manifest) {
                Some(src_tauri) => src_tauri.join("assets"),
                None => manifest.join("src-tauri").join("assets"),
            };
            assert_eq!(assets, PathBuf::from("/repo/src-tauri/assets"));
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    fn store_dir_uses_the_compile_time_manifest_anchor() {
        // The live static must resolve from the compile-time manifest instead of
        // the process working directory, so a `cargo test` launched from any
        // directory lands on the same repository `dev_data`.
        let expected = dev_repository_root(Path::new(env!("CARGO_MANIFEST_DIR"))).join("dev_data");
        assert_eq!(get_static_var(&STORE_DIR), expected);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn resource_dir_uses_the_compile_time_manifest_anchor() {
        // Assets resolve to the same `src-tauri/assets` from either manifest, and
        // the static no longer depends on an ambient `CARGO_MANIFEST_DIR`.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let expected = match nearest_src_tauri(manifest) {
            Some(src_tauri) => src_tauri.join("assets"),
            None => manifest.join("src-tauri").join("assets"),
        };
        assert_eq!(get_static_var(&RESOURCE_DIR), expected);
    }
}
