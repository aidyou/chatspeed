use anyhow::Result;
use log::info;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Wry};

#[cfg(not(debug_assertions))]
use tauri::Manager;

// Recursively copies a directory after removing any previous destination contents.
fn replace_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    let src_path = src.as_ref();
    let dst_path = dst.as_ref();

    if dst_path.exists() {
        if dst_path.is_dir() {
            fs::remove_dir_all(dst_path)?;
        } else {
            fs::remove_file(dst_path)?;
        }
    }
    copy_dir_all(src_path, dst_path)
}

fn schema_file_fingerprint(dir: &Path) -> io::Result<(usize, String)> {
    let mut file_names = Vec::<PathBuf>::new();
    collect_relative_file_names(dir, dir, &mut file_names)?;
    file_names.sort();

    let mut input = String::new();
    for path in &file_names {
        input.push_str(&path.to_string_lossy());
        input.push('\0');
    }

    Ok((file_names.len(), format!("{:x}", md5::compute(input))))
}

fn collect_relative_file_names(
    root: &Path,
    current: &Path,
    file_names: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_relative_file_names(root, &path, file_names)?;
        } else {
            let relative_path = path
                .strip_prefix(root)
                .map_err(|error| io::Error::other(error.to_string()))?;
            file_names.push(relative_path.to_path_buf());
        }
    }
    Ok(())
}

fn schema_file_sets_match(source: &Path, destination: &Path) -> io::Result<bool> {
    let source_fingerprint = schema_file_fingerprint(source)?;
    let destination_fingerprint = schema_file_fingerprint(destination)?;
    Ok(source_fingerprint == destination_fingerprint)
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let new_dst_path = dst.join(entry.file_name());

        if entry_type.is_dir() {
            copy_dir_all(&entry.path(), &new_dst_path)?;
        } else {
            fs::copy(entry.path(), new_dst_path)?;
        }
    }
    Ok(())
}

/// Ensures the bundled scraper schema replaces the application's prior schema directory at
/// startup, removing resource files that are no longer included with the current version.
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

    if !schema_src_path.exists() {
        return Err(anyhow::anyhow!(
            "Schema source directory does not exist: {:?}",
            schema_src_path
        ));
    }

    if !schema_dest_path.exists() {
        info!(
            "Copying scraper schema files from {:?} to {:?}",
            schema_src_path, schema_dest_path
        );
        copy_dir_all(&schema_src_path, &schema_dest_path)
            .map_err(|error| anyhow::anyhow!("Failed to copy schema files: {error}"))?;
        info!("Finished copying scraper schema files.");
        return Ok(());
    }

    if schema_dest_path.is_dir()
        && schema_file_sets_match(&schema_src_path, &schema_dest_path)
            .map_err(|error| anyhow::anyhow!("Failed to fingerprint schema files: {error}"))?
    {
        info!("Scraper schema file fingerprint matches bundled resources; skipping sync.");
        return Ok(());
    }

    info!(
        "Replacing scraper schema files from {:?} to {:?} because file fingerprints differ",
        schema_src_path, schema_dest_path
    );
    replace_dir_all(&schema_src_path, &schema_dest_path)
        .map_err(|error| anyhow::anyhow!("Failed to replace schema files: {error}"))?;
    info!("Finished replacing scraper schema files.");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{copy_dir_all, replace_dir_all, schema_file_fingerprint, schema_file_sets_match};
    use std::fs;

    #[test]
    fn replacing_schema_directory_removes_obsolete_resources() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("assets").join("scrape").join("schema");
        let destination_dir = temp_dir.path().join("app").join("schema");
        fs::create_dir_all(source_dir.join("nested")).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        fs::write(source_dir.join("current.json"), b"current schema").unwrap();
        fs::write(
            source_dir.join("nested").join("current.js"),
            b"current helper",
        )
        .unwrap();
        fs::write(destination_dir.join("obsolete.json"), b"old schema").unwrap();

        replace_dir_all(&source_dir, &destination_dir).unwrap();

        assert_eq!(
            fs::read(destination_dir.join("current.json")).unwrap(),
            b"current schema"
        );
        assert_eq!(
            fs::read(destination_dir.join("nested").join("current.js")).unwrap(),
            b"current helper"
        );
        assert!(!destination_dir.join("obsolete.json").exists());
    }

    #[test]
    fn matching_schema_file_sets_have_the_same_fingerprint() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let destination_dir = temp_dir.path().join("destination");
        fs::create_dir_all(source_dir.join("nested")).unwrap();
        fs::write(source_dir.join("a.json"), b"source content").unwrap();
        fs::write(source_dir.join("nested").join("b.js"), b"source helper").unwrap();
        copy_dir_all(&source_dir, &destination_dir).unwrap();
        fs::write(
            destination_dir.join("a.json"),
            b"changed content is ignored",
        )
        .unwrap();

        assert!(schema_file_sets_match(&source_dir, &destination_dir).unwrap());
        assert_eq!(
            schema_file_fingerprint(&source_dir).unwrap(),
            schema_file_fingerprint(&destination_dir).unwrap()
        );
    }

    #[test]
    fn differing_schema_file_sets_replace_obsolete_resources() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let destination_dir = temp_dir.path().join("destination");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        fs::write(source_dir.join("current.json"), b"current schema").unwrap();
        fs::write(destination_dir.join("obsolete.json"), b"old schema").unwrap();

        assert!(!schema_file_sets_match(&source_dir, &destination_dir).unwrap());
        replace_dir_all(&source_dir, &destination_dir).unwrap();

        assert!(schema_file_sets_match(&source_dir, &destination_dir).unwrap());
        assert_eq!(
            fs::read(destination_dir.join("current.json")).unwrap(),
            b"current schema"
        );
        assert!(!destination_dir.join("obsolete.json").exists());
    }
}
