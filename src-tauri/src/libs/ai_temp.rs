use crate::libs::tsid::TsidGenerator;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

pub const AI_TEMP_ROOT: &str = "/tmp";
pub const LARGE_TOOL_OUTPUT_CHAR_LIMIT: usize = 20_000;
const TEMP_FILE_CREATE_ATTEMPTS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedToolOutput {
    pub path: String,
    pub file_size_bytes: u64,
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = path.components().peekable();
    let mut normalized = if let Some(component @ Component::Prefix(..)) = components.peek() {
        let component = component.clone();
        components.next();
        PathBuf::from(component.as_os_str())
    } else {
        PathBuf::new()
    };

    for component in components {
        match component {
            Component::Prefix(..) => unreachable!(),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }

    normalized
}

fn ai_temp_relative_path(path: &Path) -> Option<PathBuf> {
    normalize_path(path)
        .strip_prefix(Path::new(AI_TEMP_ROOT))
        .ok()
        .map(Path::to_path_buf)
}

/// Resolves the model-facing `/tmp` namespace into the platform's process temp directory.
pub fn resolve_ai_temp_path(path: &Path) -> PathBuf {
    ai_temp_relative_path(path)
        .map(|relative| std::env::temp_dir().join(relative))
        .unwrap_or_else(|| path.to_path_buf())
}

fn display_ai_temp_relative_path(relative: &Path) -> String {
    let relative = relative.to_string_lossy().replace('\\', "/");
    if relative.is_empty() {
        AI_TEMP_ROOT.to_string()
    } else {
        format!("{AI_TEMP_ROOT}/{relative}")
    }
}

/// Returns a short model-facing path when `path` is inside the process temp directory.
pub fn display_ai_temp_path(path: &Path) -> Option<String> {
    let temp_root = std::env::temp_dir();
    if let Ok(relative) = path.strip_prefix(&temp_root) {
        return Some(display_ai_temp_relative_path(relative));
    }

    let canonical_root = fs::canonicalize(&temp_root).ok()?;
    let canonical_path = fs::canonicalize(path).ok()?;
    canonical_path
        .strip_prefix(canonical_root)
        .ok()
        .map(display_ai_temp_relative_path)
}

pub fn persist_tool_output(content: &str) -> io::Result<PersistedToolOutput> {
    let generator = TsidGenerator::new(1).map_err(io::Error::other)?;

    for _ in 0..TEMP_FILE_CREATE_ATTEMPTS {
        let tsid = generator.generate().map_err(io::Error::other)?;
        let physical_path = std::env::temp_dir().join(tsid);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);

        let mut file = match options.open(&physical_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        if let Err(error) = file
            .write_all(content.as_bytes())
            .and_then(|_| file.flush())
        {
            drop(file);
            let _ = fs::remove_file(&physical_path);
            return Err(error);
        }

        let file_size_bytes = file.metadata()?.len();
        let path = display_ai_temp_path(&physical_path)
            .unwrap_or_else(|| physical_path.to_string_lossy().to_string());
        return Ok(PersistedToolOutput {
            path,
            file_size_bytes,
        });
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to allocate a unique temporary output file",
    ))
}

pub fn persist_large_tool_output(content: &str) -> io::Result<Option<PersistedToolOutput>> {
    if content.chars().count() <= LARGE_TOOL_OUTPUT_CHAR_LIMIT {
        return Ok(None);
    }

    persist_tool_output(content).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ai_temp_path_to_process_temp_directory() {
        assert_eq!(
            resolve_ai_temp_path(Path::new("/tmp/example/output.txt")),
            std::env::temp_dir().join("example").join("output.txt")
        );
    }

    #[test]
    fn displays_process_temp_path_with_ai_alias() {
        assert_eq!(
            display_ai_temp_path(&std::env::temp_dir().join("example.txt")).as_deref(),
            Some("/tmp/example.txt")
        );
    }

    #[test]
    fn persists_large_output_behind_ai_temp_alias() {
        let content = "x".repeat(LARGE_TOOL_OUTPUT_CHAR_LIMIT + 1);
        let persisted = persist_large_tool_output(&content).unwrap().unwrap();
        let physical_path = resolve_ai_temp_path(Path::new(&persisted.path));

        let file_name = Path::new(&persisted.path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap();
        assert!(persisted.path.starts_with("/tmp/"));
        assert_eq!(file_name.len(), 13);
        assert_eq!(fs::read_to_string(&physical_path).unwrap(), content);
        assert_eq!(persisted.file_size_bytes, content.len() as u64);

        fs::remove_file(physical_path).unwrap();
    }
}
