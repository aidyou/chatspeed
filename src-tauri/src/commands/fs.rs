use std::path::{Path, PathBuf};

use crate::libs::fs::{get_file_name, save_thumbnail_image};
use crate::HTTP_SERVER;
use crate::HTTP_SERVER_TMP_DIR;
use crate::HTTP_SERVER_UPLOAD_DIR;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rust_i18n::t;
use std::borrow::Cow;
use std::fs;

use crate::error::{AppError, Result};

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Command;
use std::time::UNIX_EPOCH;

use crate::workflow::react::security::CHATSPEED_IGNORE_FILE;

const EDITOR_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct EditorFileInfo {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified_at_ms: u64,
    pub is_file: bool,
}

#[derive(Debug, Serialize)]
pub struct EditorFileContent {
    pub path: String,
    pub name: String,
    pub content: String,
    pub size: u64,
    pub modified_at_ms: u64,
}

fn metadata_modified_at_ms(metadata: &fs::Metadata) -> Result<u64> {
    let modified = metadata.modified().map_err(|e| AppError::General {
        message: t!("command.fs.file_info_failed", error = e.to_string()).to_string(),
    })?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AppError::General {
            message: t!("command.fs.file_info_failed", error = e.to_string()).to_string(),
        })?;

    u64::try_from(duration.as_millis()).map_err(|e| AppError::General {
        message: t!("command.fs.file_info_failed", error = e.to_string()).to_string(),
    })
}

fn editor_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

fn editor_file_info_from_metadata(
    file_path: &str,
    metadata: &fs::Metadata,
) -> Result<EditorFileInfo> {
    let path = Path::new(file_path);
    Ok(EditorFileInfo {
        path: file_path.to_string(),
        name: editor_file_name(path),
        size: metadata.len(),
        modified_at_ms: metadata_modified_at_ms(metadata)?,
        is_file: metadata.is_file(),
    })
}

fn ensure_editor_file(file_path: &str) -> Result<fs::Metadata> {
    let metadata = fs::metadata(file_path).map_err(|e| AppError::General {
        message: t!("command.fs.file_info_failed", error = e.to_string()).to_string(),
    })?;

    if !metadata.is_file() {
        return Err(AppError::General {
            message: t!("command.fs.not_a_file").to_string(),
        });
    }

    Ok(metadata)
}

fn resolve_editor_max_bytes(max_bytes: Option<u64>) -> u64 {
    max_bytes
        .unwrap_or(EDITOR_MAX_FILE_BYTES)
        .min(EDITOR_MAX_FILE_BYTES)
}

fn ensure_editor_file_size(metadata: &fs::Metadata, max_bytes: u64) -> Result<()> {
    if metadata.len() > max_bytes {
        return Err(AppError::General {
            message: t!(
                "command.fs.file_too_large",
                max = EDITOR_MAX_FILE_BYTES.to_string()
            )
            .to_string(),
        });
    }

    Ok(())
}

fn git_status_priority(status: &str) -> u8 {
    let code = status.trim();
    if code.contains('D') {
        return 3;
    }
    if code == "??" || code.contains('A') {
        return 2;
    }
    if code.contains('M') || code.contains('R') || code.contains('C') || code.contains('U') {
        return 1;
    }
    0
}

fn pick_directory_git_status(path: &str, git_statuses: &HashMap<String, String>) -> Option<String> {
    let prefix = format!("{}/", path.trim_end_matches(['/', '\\']));
    let mut best_status: Option<&str> = None;
    let mut best_priority = 0;

    for (candidate_path, candidate_status) in git_statuses {
        if !candidate_path.starts_with(&prefix) {
            continue;
        }

        let priority = git_status_priority(candidate_status);
        if priority > best_priority {
            best_priority = priority;
            best_status = Some(candidate_status.as_str());
        }
    }

    best_status.map(ToOwned::to_owned)
}

fn should_skip_list_dir_entry(name: &str) -> bool {
    let name_lower = name.to_lowercase();
    name == "node_modules"
        || name == ".git"
        || name == "__pycache__"
        || name_lower.ends_with(".pyc")
        || name_lower == "thumbs.db"
        || name_lower == ".ds_store"
}

fn build_ignore_scope(path: &Path, file_name: &str) -> Option<(PathBuf, Gitignore)> {
    for base_dir in path.ancestors() {
        let ignore_file = base_dir.join(file_name);
        if !ignore_file.is_file() {
            continue;
        }
        let mut builder = GitignoreBuilder::new(base_dir);
        if builder.add(&ignore_file).is_some() {
            continue;
        }
        if let Ok(matcher) = builder.build() {
            return Some((base_dir.to_path_buf(), matcher));
        }
    }
    None
}

fn is_ignored_by_scope(matcher: Option<&(PathBuf, Gitignore)>, path: &Path, is_dir: bool) -> bool {
    let Some((base_dir, matcher)) = matcher else {
        return false;
    };
    path.strip_prefix(base_dir)
        .ok()
        .map(|relative| matcher.matched(relative, is_dir).is_ignore())
        .unwrap_or(false)
}

fn is_whitelisted_by_scope(
    matcher: Option<&(PathBuf, Gitignore)>,
    path: &Path,
    is_dir: bool,
) -> bool {
    let Some((base_dir, matcher)) = matcher else {
        return false;
    };
    path.strip_prefix(base_dir)
        .ok()
        .map(|relative| matcher.matched(relative, is_dir).is_whitelist())
        .unwrap_or(false)
}

fn git_working_dir(path: &str) -> &Path {
    let target = Path::new(path);
    if target.is_dir() {
        target
    } else {
        target.parent().unwrap_or(target)
    }
}

fn get_repo_root(path: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(git_working_dir(path))
        .output()
        .map_err(|e| AppError::General {
            message: format!("Failed to execute git: {}", e),
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
    ))
}

#[tauri::command]
pub async fn get_git_status(path: &str) -> Result<HashMap<String, String>> {
    let Some(repo_root) = get_repo_root(path)? else {
        return Ok(HashMap::new());
    };

    let output = Command::new("git")
        .args(["-c", "status.relativePaths=false", "status", "--porcelain"])
        .current_dir(git_working_dir(path))
        .output()
        .map_err(|e| AppError::General {
            message: format!("Failed to execute git: {}", e),
        })?;

    if !output.status.success() {
        return Ok(HashMap::new()); // Not a git repo or other error, return empty
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut status_map = HashMap::new();
    let base_path = Path::new(&repo_root);

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = line[..2].trim().to_string();
        let relative_path = line[3..].to_string();

        // Convert to absolute path for easier matching in frontend
        let absolute_path = base_path.join(relative_path).to_string_lossy().to_string();
        status_map.insert(absolute_path, status);
    }

    Ok(status_map)
}

#[tauri::command]
pub async fn read_git_base_text_file(file_path: &str) -> Result<Option<String>> {
    let file = Path::new(file_path);
    if !file.exists() || file.is_dir() {
        return Ok(None);
    }

    let Some(repo_root) = get_repo_root(file_path)? else {
        return Ok(None);
    };

    let repo_root_path = Path::new(&repo_root);
    let relative_path = match file.strip_prefix(repo_root_path) {
        Ok(relative) => relative,
        Err(_) => return Ok(None),
    };

    let relative_path = relative_path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");

    if relative_path.is_empty() {
        return Ok(None);
    }

    let spec = format!("HEAD:{}", relative_path);
    let output = Command::new("git")
        .args(["show", &spec])
        .current_dir(repo_root_path)
        .output()
        .map_err(|e| AppError::General {
            message: format!("Failed to execute git: {}", e),
        })?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}
#[tauri::command]
pub async fn list_dir(path: &str) -> Result<Vec<Value>> {
    let mut list = Vec::new();
    let root_path = Path::new(path);
    let chatspeed_ignore = build_ignore_scope(root_path, CHATSPEED_IGNORE_FILE);
    let gitignore = build_ignore_scope(root_path, ".gitignore");
    let has_chatspeed_ignore = chatspeed_ignore.is_some();

    // Get git status for the directory if it's a git repo
    let git_statuses = get_git_status(path).await.unwrap_or_default();

    // Use ignore crate with git filters disabled when .csignore exists
    // so explicitly allowed git-ignored data can appear in workflow file lists.
    let mut walker = ignore::WalkBuilder::new(path);
    walker
        .max_depth(Some(1)) // Only current directory
        .standard_filters(!has_chatspeed_ignore) // Respect .gitignore unless ChatSpeed rules are present
        .hidden(false); // We want to see hidden files unless they are ignored by git

    for result in walker.build() {
        let entry = match result {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Skip the base path itself
        if entry.depth() == 0 {
            continue;
        }

        let path_buf = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Additional manual filters for common unwanted items
        if should_skip_list_dir_entry(&name) {
            continue;
        }

        let is_dir = path_buf.is_dir();
        let allowed_by_chatspeed =
            is_whitelisted_by_scope(chatspeed_ignore.as_ref(), path_buf, is_dir);
        if is_ignored_by_scope(chatspeed_ignore.as_ref(), path_buf, is_dir)
            || (!allowed_by_chatspeed && is_ignored_by_scope(gitignore.as_ref(), path_buf, is_dir))
        {
            continue;
        }
        let path_str = path_buf.to_string_lossy().to_string();

        // Find git status for this file (keys in git_statuses are absolute paths)
        let status = git_statuses.get(&path_str).cloned().or_else(|| {
            if is_dir {
                pick_directory_git_status(&path_str, &git_statuses)
            } else {
                None
            }
        });

        list.push(serde_json::json!({
            "name": name,
            "path": path_str,
            "is_dir": is_dir,
            "git_status": status,
        }));
    }

    // Sort: directories first, then alphabetical
    list.sort_by(|a, b| {
        let a_is_dir = a["is_dir"].as_bool().unwrap_or(false);
        let b_is_dir = b["is_dir"].as_bool().unwrap_or(false);
        if a_is_dir != b_is_dir {
            return b_is_dir.cmp(&a_is_dir);
        }
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });

    Ok(list)
}

/// Read text file content
///
/// Reads the content of a text file.
///
/// # Arguments
/// * `file_path` - Path to the text file
///
/// # Returns
/// * `Result<String>` - File content or error message
#[tauri::command]
pub async fn read_text_file(file_path: &str) -> Result<String> {
    log::debug!("Reading text file from path: {}", file_path);

    let content = fs::read_to_string(file_path).map_err(|e| {
        log::error!("Failed to read text file '{}': {}", file_path, e);
        AppError::General {
            message: t!("command.fs.read_file_failed", error = e.to_string()).to_string(),
        }
    })?;

    log::debug!(
        "Successfully read text file, content length: {}",
        content.len()
    );
    Ok(content)
}

#[tauri::command]
pub async fn get_text_file_info(file_path: &str) -> Result<EditorFileInfo> {
    let metadata = ensure_editor_file(file_path)?;
    editor_file_info_from_metadata(file_path, &metadata)
}

#[tauri::command]
pub async fn read_text_file_for_editor(
    file_path: &str,
    max_bytes: Option<u64>,
) -> Result<EditorFileContent> {
    log::debug!("Reading editor text file from path: {}", file_path);

    let resolved_max_bytes = resolve_editor_max_bytes(max_bytes);
    let metadata = ensure_editor_file(file_path)?;
    ensure_editor_file_size(&metadata, resolved_max_bytes)?;

    let content = fs::read_to_string(file_path).map_err(|e| {
        log::error!("Failed to read editor text file '{}': {}", file_path, e);
        AppError::General {
            message: t!("command.fs.read_file_failed", error = e.to_string()).to_string(),
        }
    })?;

    let info = editor_file_info_from_metadata(file_path, &metadata)?;
    Ok(EditorFileContent {
        path: info.path,
        name: info.name,
        content,
        size: info.size,
        modified_at_ms: info.modified_at_ms,
    })
}

#[tauri::command]
pub async fn write_text_file_for_editor(
    file_path: &str,
    content: &str,
    expected_modified_at_ms: u64,
    expected_size: u64,
) -> Result<EditorFileInfo> {
    let metadata = ensure_editor_file(file_path)?;
    let current_modified_at_ms = metadata_modified_at_ms(&metadata)?;

    if metadata.len() != expected_size || current_modified_at_ms != expected_modified_at_ms {
        return Err(AppError::General {
            message: t!("command.fs.file_changed_on_disk").to_string(),
        });
    }

    fs::write(file_path, content).map_err(|e| AppError::General {
        message: t!("command.fs.write_file_failed", error = e.to_string()).to_string(),
    })?;

    let updated_metadata = ensure_editor_file(file_path)?;
    editor_file_info_from_metadata(file_path, &updated_metadata)
}

#[tauri::command]
pub async fn open_path_in_file_manager(path: &str) -> Result<()> {
    let target = Path::new(path);

    if !target.exists() {
        return Err(AppError::General {
            message: format!("Path does not exist: {}", path),
        });
    }

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };

    let status = command.status().map_err(|e| AppError::General {
        message: format!("Failed to open path '{}': {}", path, e),
    })?;

    if !status.success() {
        return Err(AppError::General {
            message: format!(
                "Failed to open path '{}': command exited with {}",
                path, status
            ),
        });
    }

    Ok(())
}

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
) -> Result<String> {
    let tmp_dir = HTTP_SERVER_TMP_DIR.read().clone();
    let save_path = save_thumbnail_image(
        image_path,
        Path::new(&tmp_dir),
        preview_width,
        preview_height,
    )
    .map_err(|e| AppError::General {
        message: t!("command.fs.image_preview_failed", error = e.to_string()).to_string(),
    })?;

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

#[tauri::command]
pub async fn image_source_url(image_path: &std::path::Path) -> Result<String> {
    let upload_dir = HTTP_SERVER_UPLOAD_DIR.read().clone();
    let original_dir = Path::new(&upload_dir).join("workflow-source");
    std::fs::create_dir_all(&original_dir).map_err(|e| AppError::General {
        message: format!(
            "Failed to create workflow source image directory '{}': {}",
            original_dir.display(),
            e
        ),
    })?;

    let file_name = get_file_name(image_path);
    let save_path = original_dir.join(file_name.clone());
    std::fs::copy(image_path, &save_path).map_err(|e| AppError::General {
        message: format!(
            "Failed to copy source image '{}' to '{}': {}",
            image_path.display(),
            save_path.display(),
            e
        ),
    })?;

    let mut http_server = HTTP_SERVER.read().clone();
    if http_server.is_empty() {
        http_server = "http://127.0.0.1:21914".to_string()
    };

    Ok(format!(
        "{}/upload/workflow-source/{}",
        http_server, file_name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn editor_read_rejects_files_over_resolved_limit() {
        let dir = tempdir().expect("temporary directory");
        let path = dir.path().join("large.rs");
        fs::write(&path, "abcdef").expect("write test file");

        let result = read_text_file_for_editor(path.to_str().expect("utf8 path"), Some(5)).await;

        assert!(result.is_err(), "file over provided limit must be rejected");
        let error = result.err().expect("expected error").to_string();
        assert!(
            error.contains("too") || error.contains("大") || error.contains("Maximum"),
            "expected size-limit error, got: {error}"
        );
    }

    #[tokio::test]
    async fn editor_write_rejects_changed_file_before_overwrite() {
        let dir = tempdir().expect("temporary directory");
        let path = dir.path().join("conflict.ts");
        fs::write(&path, "let value = 1;\n").expect("write test file");

        let opened = read_text_file_for_editor(path.to_str().expect("utf8 path"), None)
            .await
            .expect("read editor file");
        fs::write(&path, "let value = 123;\n").expect("simulate external change");

        let result = write_text_file_for_editor(
            path.to_str().expect("utf8 path"),
            "let value = 2;\n",
            opened.modified_at_ms,
            opened.size,
        )
        .await;

        assert!(result.is_err(), "changed file must not be overwritten");
        let disk_content = fs::read_to_string(&path).expect("read disk content");
        assert_eq!(disk_content, "let value = 123;\n");
    }

    #[tokio::test]
    async fn editor_write_updates_metadata_after_successful_save() {
        let dir = tempdir().expect("temporary directory");
        let path = dir.path().join("save.py");
        fs::write(&path, "print('old')\n").expect("write test file");

        let opened = read_text_file_for_editor(path.to_str().expect("utf8 path"), None)
            .await
            .expect("read editor file");
        let updated = write_text_file_for_editor(
            path.to_str().expect("utf8 path"),
            "print('new')\n",
            opened.modified_at_ms,
            opened.size,
        )
        .await
        .expect("save editor file");

        assert_eq!(updated.size, "print('new')\n".len() as u64);
        let disk_content = fs::read_to_string(&path).expect("read disk content");
        assert_eq!(disk_content, "print('new')\n");
    }
}
