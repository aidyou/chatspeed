use std::path::Path;

use crate::libs::fs::save_thumbnail_image;
use crate::HTTP_SERVER;
use crate::HTTP_SERVER_TMP_DIR;
use rust_i18n::t;
use std::borrow::Cow;

/// Read and process an image file
///
/// Reads an image file from the given path, resizes it to 200x200px while maintaining aspect ratio,
/// and returns the processed image data.
///
/// # Arguments
/// * `path` - Path to the image file
///
/// # Returns
/// * `Result<Vec<u8>, String>` - Processed image data as bytes or error message
#[tauri::command]
pub async fn image_preview(
    image_path: &std::path::Path,
    preview_width: Option<u32>,
    preview_height: Option<u32>,
) -> Result<String, String> {
    let tmp_dir = HTTP_SERVER_TMP_DIR.read().clone();
    let save_path = save_thumbnail_image(
        image_path,
        Path::new(&tmp_dir),
        preview_width,
        preview_height,
    )
    .map_err(|e| t!("command.fs.image_preview_failed", error = e.to_string()).to_string())?;

    // Get the file name from the saved path
    let file_name = save_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(Cow::Borrowed)
        .unwrap_or_else(|| t!("command.fs.unknown_filename")) // t! returns Cow<'_, str>
        .to_string();

    let mut http_server = HTTP_SERVER.read().clone();
    if http_server.is_empty() {
        http_server = "http://127.0.0.1:21914".to_string()
    };

    Ok(format!("{}/tmp/{}", http_server, file_name))
}
