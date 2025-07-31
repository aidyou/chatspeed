use rust_i18n::t;
use std::{
    net::{AddrParseError, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Once},
};
use tauri::AppHandle;
// Required for AppHandle::path() method even when using fully qualified syntax (<AppHandle as Manager>::path)
// DO NOT REMOVE: This trait import is necessary for the Manager trait to be in scope
#[allow(unused_imports)]
use tauri::Manager;
use tokio::{
    signal,
    sync::broadcast,
    task,
    time::{self, Duration},
};
use warp::{http::StatusCode, reply::Response as WarpResponse, Filter, Rejection};

use crate::{
    ccproxy::{self, handle_proxy_rejection},
    constants::{CFG_CCPROXY_PORT, CFG_CCPROXY_PORT_DEFAULT},
    db::MainStore,
};
use crate::{
    HTTP_SERVER, HTTP_SERVER_DIR, HTTP_SERVER_THEME_DIR, HTTP_SERVER_TMP_DIR,
    HTTP_SERVER_UPLOAD_DIR, PLUGINS_DIR, SHARED_DATA_DIR, STORE_DIR,
};

static INIT: Once = Once::new();

/// Starts an HTTP server with multiple static directories.
///
/// # Arguments
/// * `app` - Tauri application handle.
/// * `main_store` - Shared main store for configuration and data.
///
/// # Returns
/// * `Result<(), String>` - Returns `Ok(())` on success, or an error message on failure.
pub async fn start_http_server(
    app: &AppHandle,
    main_store: Arc<Mutex<MainStore>>,
) -> Result<(), String> {
    // plugins dir
    let app_data_dir = get_app_data_dir(app)?;
    let plugins_dir = app_data_dir.join("plugins");
    // shared data dir
    let shared_data_dir = app_data_dir.join("shared");
    std::fs::create_dir_all(&plugins_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&shared_data_dir).map_err(|e| e.to_string())?;

    // Get the server directory
    let server_dir = get_server_dir(app)?;
    // Define the path for the theme directory
    let theme_dir = Path::new(&server_dir).join("theme");
    // Define the path for the upload directory
    let upload_dir = Path::new(&server_dir).join("upload");
    // Define the path for the temporary directory
    let tmp_dir = Path::new(&server_dir).join("tmp");

    // Create necessary directories
    std::fs::create_dir_all(&theme_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&upload_dir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;

    // Set up static theme service
    let static_theme = warp::path("theme").and(warp::fs::dir(theme_dir.clone()));

    // Set up static upload service
    let static_upload = warp::path("upload").and(warp::fs::dir(upload_dir.clone()));

    // Set up static temporary service
    let static_tmp = warp::path("tmp").and(warp::fs::dir(tmp_dir.clone()));

    // add save png service
    let save_png = warp::path!("save" / "png")
        .and(warp::post())
        .and(warp::body::bytes())
        .and_then(handle_save_png);

    // Define a filter that rejects with warp::reject::notFound()
    // This allows the .recover() mechanism to handle 404 errors.
    let handle_not_found_errors = warp::any()
        .and_then(|| async { Err::<WarpResponse, Rejection>(warp::reject::not_found()) });

    // define cors config
    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["Content-Type", "Authorization"])
        .build();

    // Combine all routes
    // All branches of .or() should ideally have the same Error type (Rejection)
    // before .recover() is applied.
    let serve = static_theme
        .or(static_upload)
        .or(static_tmp)
        .or(save_png) // Ensure save_png is distinct or correctly ordered if it might conflict
        .or(handle_not_found_errors) // Use the rejecting not_found handler
        .with(cors.clone())
        .recover(handle_proxy_rejection); // This single recover handles rejections from routes and CORS.

    let addr = try_available_port("127.0.0.1", 21912).await.map_err(|e| {
        log::error!("Failed to find available port: {}", e);
        e
    })?;

    log::info!("Serving static files at http://{}", addr);

    INIT.call_once(|| {
        // Store the server directory
        *STORE_DIR.write() = app_data_dir.clone();
        *PLUGINS_DIR.write() = plugins_dir.to_string_lossy().to_string();
        *SHARED_DATA_DIR.write() = shared_data_dir.to_string_lossy().to_string();
        *HTTP_SERVER_DIR.write() = server_dir.to_string_lossy().to_string();
        *HTTP_SERVER_THEME_DIR.write() = theme_dir.to_string_lossy().to_string();
        *HTTP_SERVER_UPLOAD_DIR.write() = upload_dir.to_string_lossy().to_string();
        *HTTP_SERVER_TMP_DIR.write() = tmp_dir.to_string_lossy().to_string();
        *HTTP_SERVER.write() = format!("http://{}", addr);
    });

    // Create a broadcast channel for signal transmission
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // Start the HTTP server
    let serve_handle = task::spawn(async move {
        log::info!("Starting HTTP server on {}", addr);

        // Add a small delay to ensure port is fully released
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Start the server - warp 0.3.7 doesn't have try_bind, so we use run
        // The panic issue should be mitigated by our improved port checking above
        warp::serve(serve).run(addr).await;
    });

    // Create chat completion proxy routes
    // ccproxy routes are served independently on a separate port
    let ccproxy_serve = ccproxy::routes(main_store.clone())
        .with(cors)
        .recover(handle_proxy_rejection);
    let server_port = if let Ok(store) = main_store.lock() {
        store.get_config(CFG_CCPROXY_PORT, CFG_CCPROXY_PORT_DEFAULT)
    } else {
        CFG_CCPROXY_PORT_DEFAULT
    };

    // Start chat completion proxy server with retry mechanism
    let ccproxy_handle = task::spawn(async move {
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 5;

        loop {
            attempts += 1;

            match try_available_port("0.0.0.0", server_port).await {
                Ok(ccproxy_addr) => {
                    log::info!("Serving chat completion proxy on http://{}", ccproxy_addr);
                    // Try to start the server immediately
                    warp::serve(ccproxy_serve.clone()).run(ccproxy_addr).await;
                    break; // If we get here, server started successfully
                }
                Err(e) => {
                    log::error!(
                        "Failed to start ccproxy server (attempt {}): {}",
                        attempts,
                        e
                    );
                    if attempts >= MAX_ATTEMPTS {
                        log::error!(
                            "Failed to start ccproxy server after {} attempts",
                            MAX_ATTEMPTS
                        );
                        break;
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    });

    // Start a temporary file cleanup task
    let tx_clone = shutdown_tx.clone();
    let cleanup_handle = task::spawn(async move {
        cleanup_tmp_dir(tmp_dir, tx_clone.subscribe()).await;
    });

    // Listen for shutdown signals
    let shutdown_handle = task::spawn(async move {
        signal::ctrl_c().await.expect("Failed to listen for ctrl_c");
        // Send shutdown signal
        let _ = shutdown_tx.send(());
    });

    // Wait for tasks to complete
    tokio::select! {
        _ = serve_handle => {},
        _ = ccproxy_handle=>{},
        _ = cleanup_handle => {},
        _ = shutdown_handle => {},
    }

    Ok(())
}

async fn try_available_port(ip: &str, start_port: u16) -> Result<SocketAddr, String> {
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 3;

    let mut start_port = start_port;
    loop {
        attempts += 1;
        let port = find_available_port(ip, start_port, 65535)?;
        let addr: SocketAddr = format!("{}:{}", ip, port)
            .parse()
            .map_err(|e: AddrParseError| format!("Failed to parse address: {}", e))?;

        log::info!("Found available port: {} (attempt {})", port, attempts);

        // Try to bind to verify the port is available
        match std::net::TcpListener::bind(addr) {
            Ok(listener) => {
                drop(listener); // Release immediately to avoid holding the port
                log::debug!("Port {} is confirmed available", port);
                return Ok(addr);
            }
            Err(e) => {
                log::warn!(
                    "Port {} became unavailable: {} (attempt {})",
                    port,
                    e,
                    attempts
                );
                if attempts >= MAX_ATTEMPTS {
                    start_port += 1;
                }
                // Small delay before retry
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                continue;
            }
        }
    }
}
/// Attempts to find an available port starting from `start_port` up to `max_port`.
///
/// # Arguments
/// * `start_port` - The starting port number to try.
/// * `max_port` - The maximum port number to try.
///
/// # Returns
/// * `Result<u16, String>` - An available port number or an error message.
fn find_available_port(ip: &str, start_port: u16, max_port: u16) -> Result<u16, String> {
    use std::net::{SocketAddr, TcpListener};

    for port in start_port..=max_port {
        let addr: SocketAddr = format!("{}:{}", ip, port)
            .parse()
            .map_err(|e| format!("Invalid address format: {}", e))?;

        match TcpListener::bind(addr) {
            Ok(listener) => {
                // Get the actual bound port (in case we used port 0)
                let bound_port = listener
                    .local_addr()
                    .map_err(|e| format!("Failed to get local address: {}", e))?
                    .port();
                drop(listener); // Close the listener immediately
                return Ok(bound_port);
            }
            Err(_) => continue,
        }
    }
    Err(t!(
        "http.server_no_available_ports",
        start_port = start_port,
        max_port = max_port
    )
    .to_string())
}

/// Retrieves the application data directory based on the development environment.
///
/// # Arguments
/// * `app` - Application handle.
///
/// # Returns
/// * `Result<PathBuf, String>` - The application data directory or an error message.
fn get_app_data_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(debug_assertions)]
    {
        let dev_dir = &*crate::STORE_DIR.read();
        std::fs::create_dir_all(&dev_dir).map_err(|e| e.to_string())?;
        Ok(dev_dir.clone())
    }

    #[cfg(not(debug_assertions))]
    {
        let app_local_data_dir = _app.path().app_data_dir().or_else(|_| {
            Err(t!(
                "http.server_failed_to_get_app_data_dir",
                error = "Option was None"
            )
            .to_string())
        })?;
        std::fs::create_dir_all(&app_local_data_dir).map_err(|e| {
            t!(
                "http.server_failed_to_get_app_data_dir",
                error = e.to_string()
            )
            .to_string()
        })?;
        Ok(app_local_data_dir)
    }
}

/// Retrieves the server directory.
///
/// # Arguments
/// * `app` - Application handle.
///
/// # Returns
/// * `Result<PathBuf, String>` - The server directory or an error message.
fn get_server_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = get_app_data_dir(_app)?.join("static");
    std::fs::create_dir_all(&app_data_dir).map_err(|e| e.to_string())?;
    Ok(app_data_dir)
}

/// Periodically cleans up the temporary directory.
///
/// # Arguments
/// * `tmp_dir` - The path of the temporary directory.
/// * `shutdown` - The receiver for shutdown signals.
async fn cleanup_tmp_dir(tmp_dir: PathBuf, mut shutdown: broadcast::Receiver<()>) {
    let duration = Duration::from_secs(3600);
    let mut interval = time::interval(duration); // Runs once every hour

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Err(e) = perform_cleanup(&tmp_dir).await {
                    log::error!("Cleanup failed: {}", e);
                }
            }
            _ = shutdown.recv() => {
                log::info!("Shutdown signal received. Stopping cleanup task.");
                break;
            }
        }
    }
}

/// Performs the actual cleanup operation for the temporary directory.
///
/// # Arguments
/// * `tmp_dir` - The path of the temporary directory.
///
/// # Returns
/// * `Result<(), String>` - Returns `Ok(())` on success, or an error message on failure.
async fn perform_cleanup(tmp_dir: &PathBuf) -> Result<(), String> {
    let files = std::fs::read_dir(tmp_dir).map_err(|e| format!("ReadDir failed: {}", e))?;
    let now = std::time::SystemTime::now();
    let cutoff = Duration::from_secs(3600);

    for file in files {
        let file = file.map_err(|e| format!("File error: {}", e))?;
        let file_path = file.path();
        let metadata = file
            .metadata()
            .map_err(|e| format!("Metadata error: {}", e))?;
        let created = metadata
            .created()
            .map_err(|e| format!("Created time error: {}", e))?;
        if now.duration_since(created).map_err(|e| e.to_string())? > cutoff {
            std::fs::remove_file(&file_path).map_err(|e| format!("Remove file error: {}", e))?;
            log::info!("Removed expired file: {}", file_path.display());
        }
    }

    Ok(())
}

/// Handles saving PNG data to a file
async fn handle_save_png(body: bytes::Bytes) -> Result<impl warp::Reply, warp::Rejection> {
    // Generate a unique filename using current timestamp
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("image_{}.png", timestamp);

    // Get the downloads directory
    let download_dir = match dirs::download_dir() {
        Some(path) => path,
        None => {
            return Ok(warp::reply::with_status(
                t!("http.server_downloads_dir_not_found").to_string(),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    // Create full file path
    let file_path = download_dir.join(filename);

    // Write the bytes to file
    if let Err(e) = tokio::fs::write(&file_path, body).await {
        let error_message =
            t!("http.server_failed_to_save_file", error = e.to_string()).to_string();
        return Ok(warp::reply::with_status(
            error_message,
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }

    Ok(warp::reply::with_status(
        t!("http.server_file_saved_successfully").to_string(),
        StatusCode::OK,
    ))
}
