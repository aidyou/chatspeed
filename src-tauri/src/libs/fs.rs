use image::{imageops::FilterType::Lanczos3, GenericImageView, ImageFormat, ImageReader};
use rust_i18n::t;
use std::path::MAIN_SEPARATOR;
use xxhash_rust::xxh32;

use crate::{DEFAULT_THUMBNAIL_HEIGHT, DEFAULT_THUMBNAIL_WIDTH};

/// Save a thumbnail image to the given directory
///
/// # Arguments
/// * `image_path` - The path to the image to save
/// * `save_dir` - The directory to save the image to
/// * `opt_width` - The width of the thumbnail (optional)
/// * `opt_height` - The height of the thumbnail (optional)
///
/// # Returns
/// * `std::path::PathBuf` - The path to the saved image
/// * `String` - An error message if the operation failed
pub fn save_thumbnail_image(
    image_path: &std::path::Path,
    save_dir: &std::path::Path,
    opt_width: Option<u32>,
    opt_height: Option<u32>,
) -> Result<std::path::PathBuf, String> {
    let img = ImageReader::open(&image_path)
        .map_err(|e| format!("Failed to open image: {}", e))?
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess image format: {}", e))?
        .decode()
        .map_err(|e| format!("Failed to decode image: {}", e))?;

    let format = ImageFormat::from_path(&image_path)
        .map_err(|e| format!("Failed to determine image format: {}", e))?;

    let img_width = opt_width.unwrap_or(DEFAULT_THUMBNAIL_WIDTH);
    let img_height = opt_height.unwrap_or(DEFAULT_THUMBNAIL_HEIGHT);

    let (width, height) = img.dimensions();
    let (new_width, new_height) = if width < height {
        (
            img_width,
            (img_width as f32 * height as f32 / width as f32) as u32,
        )
    } else {
        (
            (img_width as f32 * width as f32 / height as f32) as u32,
            img_height,
        )
    };

    let mut resized = img.resize(new_width, new_height, Lanczos3);
    let cropped = resized.crop(0, 0, img_width, img_height);

    // if file_id is not None, save the image to the upload directory
    let file_name = get_file_name(image_path);
    let file_path = save_dir.join(file_name.clone());

    let mut file = std::fs::File::create(&file_path)
        .map_err(|e| t!("chat.failed_to_create_file", error = e))?;

    cropped
        .write_to(&mut file, format)
        .map_err(|e| t!("chat.failed_to_write_file", error = e))?;

    // 在返回之前标准化路径分隔符
    let path_str = file_path.to_string_lossy().replace(MAIN_SEPARATOR, "/");

    Ok(std::path::PathBuf::from(path_str))
}

/// Get the file name of an image
///
/// # Arguments
/// * `image_path` - The path to the image
///
/// # Returns
/// * `String` - The file name of the image
pub fn get_file_name(image_path: &std::path::Path) -> String {
    if let Some(fname) = image_path.file_name() {
        let name_str = fname.to_string_lossy().to_string();
        format!(
            "{}.{}",
            hash_string(&name_str),
            image_path
                .extension()
                .and_then(|f| f.to_str())
                .unwrap_or("png")
        )
    } else {
        format!(
            "{}.png",
            hash_string(&image_path.to_string_lossy().to_string())
        )
    }
}

/// Generate a 32-bit hash string for the given string
///
/// # Arguments
/// * `s` - The string to hash
///
/// # Returns
/// A 32-bit hash string
///
/// # Examples
///
/// ```no_run
/// use crate::libs::hash::hash_string;
/// assert_eq!(hash_string("hello"), "fb0077f9");
/// ```
pub fn hash_string(s: &str) -> String {
    format!("{:x}", xxh32::xxh32(s.as_bytes(), 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_string() {
        assert_eq!(hash_string("hello"), "fb0077f9");
        assert_eq!(hash_string("中国"), "885195f7");
        assert_eq!(hash_string("हिन्दी"), "7ec9ad3e");
    }
}
