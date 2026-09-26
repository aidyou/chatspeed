//! ZIP archive safety for Agent Skill sources.
//!
//! An archive is the one input where an attacker fully controls both the entry
//! names and the entry metadata, so every entry is validated before anything is
//! written: path shape, entry type, per-entry and total size, and compression
//! ratio. Anything that cannot be proven benign refuses the whole archive —
//! there is no partial extraction (AC-5/INV-4).
//!
//! The first deliverable supports ZIP only; tar/tar.gz would need a new
//! dependency and a new attack surface, so it stays out of scope.

use std::collections::HashSet;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};

use crate::capability::error::CapabilityError;

/// Limits applied to one archive.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    /// Maximum `uncompressed / compressed` ratio for a single entry.
    pub max_compression_ratio: u64,
}

/// Conservative defaults for a text-and-markdown Skill bundle.
pub const DEFAULT_LIMITS: ArchiveLimits = ArchiveLimits {
    max_entries: 2_048,
    max_entry_bytes: 16 * 1024 * 1024,
    max_total_bytes: 64 * 1024 * 1024,
    max_compression_ratio: 200,
};

/// The result of a successful extraction.
#[derive(Debug, Clone)]
pub struct ArchiveReport {
    pub entries: usize,
    pub total_bytes: u64,
}

/// Extracts a ZIP archive into `destination`, validating every entry first.
pub fn extract_zip(archive_path: &Path, destination: &Path) -> Result<ArchiveReport, CapabilityError> {
    extract_zip_with_limits(archive_path, destination, DEFAULT_LIMITS)
}

/// Extracts a ZIP archive with explicit limits.
pub fn extract_zip_with_limits(
    archive_path: &Path,
    destination: &Path,
    limits: ArchiveLimits,
) -> Result<ArchiveReport, CapabilityError> {
    let file = std::fs::File::open(archive_path).map_err(|error| {
        CapabilityError::invalid_request(format!("failed to open the skill archive: {error}"))
    })?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file)).map_err(|error| {
        CapabilityError::invalid_request(format!("failed to read the skill archive: {error}"))
    })?;

    if archive.len() > limits.max_entries {
        return Err(CapabilityError::refused(format!(
            "the skill archive has {} entries, above the {}-entry limit",
            archive.len(),
            limits.max_entries
        )));
    }

    std::fs::create_dir_all(destination).map_err(|error| {
        CapabilityError::internal(format!("failed to create the extraction directory: {error}"))
    })?;

    // Two entries that normalize to the same path (or differ only by case)
    // would let a later entry silently overwrite an earlier one.
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_bytes: u64 = 0;
    let mut entries: usize = 0;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            CapabilityError::invalid_request(format!("failed to read archive entry {index}: {error}"))
        })?;

        let raw_name = entry.name().to_string();
        if raw_name.trim().is_empty() {
            return Err(CapabilityError::refused(
                "the skill archive contains an entry with an empty name",
            ));
        }

        let relative = safe_relative_path(&raw_name)?;
        let normalized = relative.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(normalized) {
            return Err(CapabilityError::refused(format!(
                "the skill archive contains colliding entries at '{raw_name}'"
            )));
        }

        let is_directory = entry.is_dir() || raw_name.ends_with('/');
        if let Some(mode) = entry.unix_mode() {
            match classify_entry_kind(mode) {
                EntryKind::Symlink => {
                    return Err(CapabilityError::refused(format!(
                        "the skill archive contains a symbolic link at '{raw_name}'"
                    )));
                }
                EntryKind::Special => {
                    return Err(CapabilityError::refused(format!(
                        "the skill archive contains a special file at '{raw_name}'"
                    )));
                }
                EntryKind::Directory | EntryKind::Regular | EntryKind::Unspecified => {}
            }
        }

        let target = destination.join(&relative);
        if is_directory {
            std::fs::create_dir_all(&target).map_err(|error| {
                CapabilityError::internal(format!("failed to create an archive directory: {error}"))
            })?;
            continue;
        }

        let declared_size = entry.size();
        if declared_size > limits.max_entry_bytes {
            return Err(CapabilityError::refused(format!(
                "archive entry '{raw_name}' is {declared_size} bytes, above the per-entry limit"
            )));
        }
        let compressed = entry.compressed_size().max(1);
        if declared_size / compressed > limits.max_compression_ratio {
            return Err(CapabilityError::refused(format!(
                "archive entry '{raw_name}' exceeds the compression-ratio limit"
            )));
        }
        total_bytes = total_bytes.saturating_add(declared_size);
        if total_bytes > limits.max_total_bytes {
            return Err(CapabilityError::refused(
                "the skill archive exceeds the expanded size limit",
            ));
        }

        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                CapabilityError::internal(format!("failed to create an archive directory: {error}"))
            })?;
        }
        let mut output = std::fs::File::create(&target).map_err(|error| {
            CapabilityError::internal(format!("failed to write an extracted file: {error}"))
        })?;
        // The reader is bounded even when the header lied about the size.
        let mut bounded = entry.by_ref().take(limits.max_entry_bytes + 1);
        let written = std::io::copy(&mut bounded, &mut output).map_err(|error| {
            CapabilityError::internal(format!("failed to extract an archive entry: {error}"))
        })?;
        output.flush().map_err(|error| {
            CapabilityError::internal(format!("failed to flush an extracted file: {error}"))
        })?;
        if written > limits.max_entry_bytes {
            return Err(CapabilityError::refused(format!(
                "archive entry '{raw_name}' expanded beyond the per-entry limit"
            )));
        }
        entries += 1;
    }

    Ok(ArchiveReport {
        entries,
        total_bytes,
    })
}

/// Validates one archive entry name and returns its relative path.
///
/// Absolute paths, drive letters, UNC prefixes, `..` traversal, backslashes,
/// control characters and NUL are all refused. Windows-reserved names and
/// trailing dots/spaces are refused too, because they alias on some
/// filesystems.
pub fn safe_relative_path(raw_name: &str) -> Result<PathBuf, CapabilityError> {
    if raw_name.is_empty() {
        return Err(CapabilityError::refused("an archive entry has an empty name"));
    }
    if raw_name.contains('\0') {
        return Err(CapabilityError::refused(
            "an archive entry name contains a NUL byte",
        ));
    }
    if raw_name.chars().any(|character| character.is_control()) {
        return Err(CapabilityError::refused(
            "an archive entry name contains control characters",
        ));
    }
    if raw_name.starts_with('/') || raw_name.starts_with('\\') {
        return Err(CapabilityError::refused(format!(
            "archive entry '{raw_name}' is an absolute path"
        )));
    }
    if raw_name.contains('\\') {
        return Err(CapabilityError::refused(format!(
            "archive entry '{raw_name}' uses a backslash separator"
        )));
    }
    let bytes = raw_name.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return Err(CapabilityError::refused(format!(
            "archive entry '{raw_name}' contains a drive letter"
        )));
    }

    let mut path = PathBuf::new();
    for segment in raw_name.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err(CapabilityError::refused(format!(
                "archive entry '{raw_name}' escapes the extraction directory"
            )));
        }
        if segment.ends_with(' ') || segment.ends_with('.') {
            return Err(CapabilityError::refused(format!(
                "archive entry '{raw_name}' ends a segment with a space or dot"
            )));
        }
        if is_windows_reserved(segment) {
            return Err(CapabilityError::refused(format!(
                "archive entry '{raw_name}' uses a reserved device name"
            )));
        }
        path.push(segment);
    }

    if path.as_os_str().is_empty() {
        return Err(CapabilityError::refused(format!(
            "archive entry '{raw_name}' has no usable path"
        )));
    }
    Ok(path)
}

/// The entry type a ZIP unix mode describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    /// No type bits were recorded.
    Unspecified,
    /// A regular file.
    Regular,
    /// A directory.
    Directory,
    /// A symbolic link.
    Symlink,
    /// A device, socket, FIFO or anything else ChatSpeed will not extract.
    Special,
}

/// Classifies a ZIP entry's unix mode.
///
/// Internet archives rarely carry these bits, but when they do they are the
/// only way to tell a stored symlink from a regular file, so the bits decide
/// whether the archive is refused.
pub fn classify_entry_kind(mode: u32) -> EntryKind {
    match mode & 0o170_000 {
        0 => EntryKind::Unspecified,
        0o100_000 => EntryKind::Regular,
        0o040_000 => EntryKind::Directory,
        0o120_000 => EntryKind::Symlink,
        _ => EntryKind::Special,
    }
}

fn is_windows_reserved(segment: &str) -> bool {    let stem = segment
        .split('.')
        .next()
        .unwrap_or(segment)
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem[3..].chars().all(|character| character.is_ascii_digit()))
}

/// Returns the single top-level directory of an extraction, when the archive
/// wrapped its content in one (as a GitHub source archive does).
pub fn single_root_child(destination: &Path) -> Option<PathBuf> {
    let mut directories = Vec::new();
    let entries = std::fs::read_dir(destination).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            directories.push(path);
        } else {
            return None;
        }
    }
    if directories.len() == 1 {
        directories.pop()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use zip::write::SimpleFileOptions;

    fn write_archive(path: &Path, entries: &[(&str, &[u8])]) {
        let file = std::fs::File::create(path).expect("create archive");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for (name, content) in entries {
            writer
                .start_file(*name, options)
                .expect("start archive entry");
            writer.write_all(content).expect("write archive entry");
        }
        writer.finish().expect("finish archive");
    }

    #[test]
    fn a_benign_archive_extracts() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("demo.zip");
        write_archive(
            &archive,
            &[
                ("demo/SKILL.md", b"---\nname: demo\n---\nbody"),
                ("demo/references/guide.md", b"guide"),
            ],
        );

        let destination = temp.path().join("out");
        let report = extract_zip(&archive, &destination).expect("extract");
        assert_eq!(report.entries, 2);
        assert!(destination.join("demo/SKILL.md").is_file());
        assert!(destination.join("demo/references/guide.md").is_file());
        assert_eq!(
            single_root_child(&destination).expect("single root").file_name().unwrap(),
            "demo"
        );
    }

    #[test]
    fn traversal_absolute_and_drive_paths_are_refused() {
        for name in [
            "../escape.md",
            "demo/../../escape.md",
            "/etc/passwd",
            "C:/Windows/system32/evil.dll",
            "demo\\evil.md",
            "demo/CON.md",
            "demo/trailing. ",
        ] {
            let error = safe_relative_path(name)
                .err()
                .unwrap_or_else(|| panic!("{name} must be refused"));
            assert_eq!(error.code(), crate::capability::error::code::REFUSED);
        }
        assert!(safe_relative_path("demo/SKILL.md").is_ok());
    }

    #[test]
    fn a_traversal_entry_refuses_the_whole_archive() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("evil.zip");
        write_archive(&archive, &[("demo/SKILL.md", b"body"), ("../escape.md", b"x")]);

        let destination = temp.path().join("out");
        let error = extract_zip(&archive, &destination).expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }

    #[test]
    fn the_entry_kind_classifier_recognizes_links_and_devices() {
        assert_eq!(classify_entry_kind(0o120_777), EntryKind::Symlink);
        assert_eq!(classify_entry_kind(0o100_644), EntryKind::Regular);
        assert_eq!(classify_entry_kind(0o040_755), EntryKind::Directory);
        assert_eq!(classify_entry_kind(0o020_666), EntryKind::Special);
        assert_eq!(classify_entry_kind(0o010_644), EntryKind::Special);
        assert_eq!(classify_entry_kind(0o0_644), EntryKind::Unspecified);
    }

    /// Rewrites the central-directory external attributes of the first entry,
    /// which is where a stored unix mode lives. The `zip` writer masks the
    /// entry type, so a symlink entry can only be produced this way.
    fn declare_entry_mode(archive_path: &Path, mode: u32) {
        let mut bytes = std::fs::read(archive_path).expect("read archive");
        let signature = [0x50u8, 0x4b, 0x01, 0x02];
        let position = bytes
            .windows(4)
            .position(|window| window == signature)
            .expect("central directory header");
        // Host system 3 (Unix) makes the mode meaningful to any reader.
        bytes[position + 5] = 3;
        bytes[position + 38..position + 42].copy_from_slice(&(mode << 16).to_le_bytes());
        std::fs::write(archive_path, bytes).expect("write archive");
    }

    #[test]
    fn a_symlink_entry_refuses_the_whole_archive() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("link.zip");
        write_archive(&archive, &[("demo/escape", b"/etc/passwd")]);
        declare_entry_mode(&archive, 0o120_777);

        let destination = temp.path().join("out");
        let error = extract_zip(&archive, &destination).expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
        assert!(!destination.join("demo/escape").exists());
    }

    #[test]
    fn a_device_entry_refuses_the_whole_archive() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("device.zip");
        write_archive(&archive, &[("demo/node", b"")]);
        declare_entry_mode(&archive, 0o020_666);

        let destination = temp.path().join("out");
        let error = extract_zip(&archive, &destination).expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }

    #[test]
    fn colliding_entry_names_refuse_the_archive() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("collision.zip");
        write_archive(
            &archive,
            &[("demo/SKILL.md", b"a"), ("demo/skill.md", b"b")],
        );

        let destination = temp.path().join("out");
        let error = extract_zip(&archive, &destination).expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }

    #[test]
    fn per_entry_and_compression_limits_refuse_the_archive() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("big.zip");
        let payload = vec![b'a'; 4_096];
        write_archive(&archive, &[("demo/SKILL.md", &payload)]);

        let destination = temp.path().join("out");
        let limits = ArchiveLimits {
            max_entries: 8,
            max_entry_bytes: 1_024,
            max_total_bytes: 8_192,
            max_compression_ratio: 200,
        };
        let error = extract_zip_with_limits(&archive, &destination, limits)
            .expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);

        let ratio_limits = ArchiveLimits {
            max_entries: 8,
            max_entry_bytes: 64 * 1024,
            max_total_bytes: 64 * 1024,
            max_compression_ratio: 1,
        };
        let error = extract_zip_with_limits(&archive, &destination, ratio_limits)
            .expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }

    #[test]
    fn an_entry_count_above_the_limit_refuses_the_archive() {
        let temp = TempDir::new().expect("temp dir");
        let archive = temp.path().join("many.zip");
        let owned: Vec<(String, Vec<u8>)> = (0..4)
            .map(|index| (format!("demo/file-{index}.md"), b"x".to_vec()))
            .collect();
        let entries: Vec<(&str, &[u8])> = owned
            .iter()
            .map(|(name, content)| (name.as_str(), content.as_slice()))
            .collect();
        write_archive(&archive, &entries);

        let destination = temp.path().join("out");
        let limits = ArchiveLimits {
            max_entries: 2,
            ..DEFAULT_LIMITS
        };
        let error = extract_zip_with_limits(&archive, &destination, limits)
            .expect_err("must refuse");
        assert_eq!(error.code(), crate::capability::error::code::REFUSED);
    }
}
