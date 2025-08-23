use anyhow::Result;
use log::error;
use log::info;
use log::warn;
use std::fs;
use std::io;
use std::path::Path;
use tauri::{AppHandle, Wry};

#[allow(unused_imports)]
use tauri::Manager;

// Helper function to recursively copy a directory, only copying files that don't exist in the destination.
fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    let src_path = src.as_ref();
    let dst_path = dst.as_ref();

    if !dst_path.exists() {
        fs::create_dir_all(&dst_path)?;
    }

    for entry in fs::read_dir(src_path)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let new_dst_path = dst_path.join(entry.file_name());

        if entry_type.is_dir() {
            copy_dir_all(entry.path(), &new_dst_path)?;
        } else if !new_dst_path.exists() {
            // Copy file only if it does not exist in the destination
            fs::copy(entry.path(), &new_dst_path)?;
        }
    }
    Ok(())
}

/// Ensures that the default configuration files (for scrapers and schemas)
/// are present in the application's data directory. If they don't exist,
/// they are copied from the bundled application assets.
///
/// This function is intended to be called at application startup.
pub fn ensure_default_configs_exist(_app: &AppHandle<Wry>) -> Result<()> {
    #[cfg(debug_assertions)]
    let app_data_dir = { &*crate::STORE_DIR.read() };

    #[cfg(not(debug_assertions))]
    let app_data_dir = _app
        .path()
        .app_data_dir()
        .map_err(|e| anyhow::anyhow!("Failed to get application data directory, error: {:?}", e))?;

    // In debug mode, read directly from the source assets to avoid bundling issues.
    // In release mode, read from the bundled resources.
    #[cfg(debug_assertions)]
    let schema_src_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("scrape")
        .join("schema");

    #[cfg(not(debug_assertions))]
    let schema_src_path = {
        let resource_dir = _app
            .path()
            .resource_dir()
            .map_err(|e| anyhow::anyhow!("Failed to get resource directory: {:?}", e))?;
        resource_dir.join("assets").join("scrape").join("schema")
    };

    let schema_dest_path = app_data_dir.join("schema");

    // Spawn a background task to handle file copying asynchronously.
    tauri::async_runtime::spawn(async move {
        if schema_src_path.exists() {
            info!(
                "Checking and copying missing schema files from {:?} to {:?}",
                schema_src_path, schema_dest_path
            );
            if let Err(e) = copy_dir_all(&schema_src_path, &schema_dest_path) {
                error!("Failed to copy schema files: {}", e);
            } else {
                info!("Finished checking and copying schema files.");
            }
        } else {
            warn!(
                "Schema source directory does not exist, skipping copy: {:?}",
                schema_src_path
            );
        }
    });

    Ok(())
}
