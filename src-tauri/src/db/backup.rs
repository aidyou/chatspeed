use crate::db::backup_crypto::{decrypt_database_streaming, encrypt_database_streaming};
use crate::db::error::StoreError;
use chrono::Local;
use log::{error, info};
use regex;
use rust_i18n::t;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use walkdir::WalkDir;
use zip::ZipArchive;
use zip::{write::FileOptions, ZipWriter};

#[cfg(not(debug_assertions))]
use tauri::Manager;

fn copy_directory(source: &Path, destination: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            fs::create_dir(&destination_path)?;
            copy_directory(&source_path, &destination_path)?;
        } else {
            let mut source_file = File::open(source_path)?;
            let mut destination_file = File::options()
                .write(true)
                .create_new(true)
                .open(destination_path)?;
            std::io::copy(&mut source_file, &mut destination_file)?;
        }
    }

    Ok(())
}

/// Configuration for database backup operations.
pub struct BackupConfig {
    /// Directory path where backups will be stored
    pub backup_dir: Option<String>,
    /// If true, skip creating the backup directory (for restore/read operations)
    pub read_only: bool,
}

/// Manages database backup and restore operations.
pub struct DbBackup {
    main_db_path: PathBuf,
    backup_dir: PathBuf,
    staging_backup_dir: PathBuf,
}

impl DbBackup {
    /// Creates a new `DbBackup` instance.
    ///
    /// # Arguments
    ///
    /// * `app` - A reference to the Tauri `AppHandle`
    /// * `config` - Backup configuration specifying backup directory and options
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if initialization fails
    pub fn new(_app: &AppHandle, config: BackupConfig) -> Result<Self, StoreError> {
        #[cfg(debug_assertions)]
        let app_dir = { &*crate::STORE_DIR.read() };

        #[cfg(not(debug_assertions))]
        let app_dir = {
            let app_local_data_dir = _app.path().app_data_dir().map_err(|e| {
                StoreError::TauriError(format!(
                    "Failed to retrieve the application data directory: {}",
                    e
                ))
            })?;
            std::fs::create_dir_all(&app_local_data_dir)
                .map_err(|e| StoreError::TauriError(e.to_string()))?;
            app_local_data_dir
        };

        // Ensure backup directory exists
        let backup_dir = match config.backup_dir.as_deref() {
            None | Some("") => app_dir.join("backup"),
            Some(dir) => PathBuf::from(dir),
        };

        let staging_backup_dir = app_dir.join("backup").join(".staging");

        // Only create directories for write operations (backup), not for read operations (restore).
        // Backups are assembled under a non-listable application-local staging root before being
        // moved to their normal destination. This avoids cloud filesystem limitations during
        // archive creation and prevents incomplete backups from being restored.
        if !config.read_only {
            for directory in [&backup_dir, &staging_backup_dir] {
                if !directory.exists() {
                    fs::create_dir_all(directory).map_err(|e| {
                        error!("Failed to create backup directory: {}", e);
                        StoreError::IoError(
                            t!(
                                "db.backup.failed_to_create_backup_dir_at",
                                path = directory.display(),
                                error = e.to_string()
                            )
                            .to_string(),
                        )
                    })?;
                }
            }
        }

        Ok(Self {
            main_db_path: app_dir.join("chatspeed.db"),
            backup_dir,
            staging_backup_dir,
        })
    }

    /// Performs database backup operations.
    ///
    /// # Returns
    ///
    /// A vector of paths to the created backup files
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if any backup operation fails
    fn backup(&self) -> Result<Vec<PathBuf>, StoreError> {
        let mut backup_paths = Vec::new();

        // Backup main database
        let main_backup_path = self.backup_single_db(&self.main_db_path, "chatspeed")?;
        backup_paths.push(main_backup_path);

        Ok(backup_paths)
    }

    /// Backs up a single database file with ZIP compression before encryption.
    fn backup_single_db(&self, db_path: &Path, db_type: &str) -> Result<PathBuf, StoreError> {
        let backup_path = self.backup_dir.join(format!("{}.db.zip", db_type));
        let backup_dir_name = self
            .backup_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown");
        let compressed_file = tempfile::NamedTempFile::new_in(&self.backup_dir)?;
        let encrypted_file = tempfile::NamedTempFile::new_in(&self.backup_dir)?;

        Self::compress_file(
            db_path,
            compressed_file.as_file().try_clone()?,
            &format!("{}.db", db_type),
        )?;
        encrypt_database_streaming(
            compressed_file.path(),
            encrypted_file.path(),
            backup_dir_name,
        )?;

        encrypted_file
            .persist_noclobber(&backup_path)
            .map_err(|error| {
                StoreError::IoError(
                    t!(
                        "db.backup.failed_to_create_encrypted_file",
                        error = error.error.to_string()
                    )
                    .to_string(),
                )
            })?;

        Ok(backup_path)
    }

    /// Decrypts a backup database to a securely staged temporary file.
    pub fn decrypt_to_temp(
        &self,
        backup_path: &Path,
        target_path: &Path,
    ) -> Result<tempfile::TempPath, StoreError> {
        if !backup_path.exists() {
            return Err(StoreError::NotFound(
                t!("db.backup.file_not_found", path = backup_path.display()).to_string(),
            ));
        }

        let backup_dir_name = backup_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let target_dir = target_path.parent().unwrap_or_else(|| Path::new("."));
        let temp_db_file = tempfile::NamedTempFile::new_in(target_dir)?;

        if backup_path
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("zip")
        {
            decrypt_database_streaming(backup_path, temp_db_file.path(), backup_dir_name)?;
            return Ok(temp_db_file.into_temp_path());
        }

        let compressed_temp_file = tempfile::NamedTempFile::new_in(target_dir)?;
        decrypt_database_streaming(backup_path, compressed_temp_file.path(), backup_dir_name)?;
        Self::extract_file(
            compressed_temp_file.path(),
            "chatspeed.db",
            temp_db_file.as_file().try_clone()?,
        )?;

        Ok(temp_db_file.into_temp_path())
    }

    fn compress_file(
        source_path: &Path,
        zip_file: File,
        entry_name: &str,
    ) -> Result<(), StoreError> {
        let source_file = File::open(source_path).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_open_file_for_zip",
                    path = source_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        let mut zip = ZipWriter::new(zip_file);
        let options: zip::write::SimpleFileOptions = FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o600);
        zip.start_file(entry_name, options).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_add_file_to_zip",
                    path = entry_name,
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        let mut source_file = source_file;
        std::io::copy(&mut source_file, &mut zip).map_err(|e| {
            StoreError::IoError(
                t!("db.backup.failed_to_write_to_zip", error = e.to_string()).to_string(),
            )
        })?;
        zip.finish().map_err(|e| {
            StoreError::IoError(
                t!("db.backup.failed_to_finish_zip", error = e.to_string()).to_string(),
            )
        })?;

        Ok(())
    }

    fn extract_file(
        zip_path: &Path,
        entry_name: &str,
        mut target_file: File,
    ) -> Result<(), StoreError> {
        let zip_file = File::open(zip_path).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_open_zip",
                    path = zip_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        let mut archive = ZipArchive::new(zip_file).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_read_zip_archive",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        let mut entry = archive.by_name(entry_name).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_read_zip_entry",
                    index = 0,
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        std::io::copy(&mut entry, &mut target_file).map_err(|e| {
            StoreError::IoError(
                t!("db.backup.failed_to_write_to_zip", error = e.to_string()).to_string(),
            )
        })?;

        Ok(())
    }

    /// Cleans up SQLite temporary files (-wal, -shm) for a given database path.
    pub fn cleanup_sqlite_temporaries(db_path: &Path) {
        let wal_path = db_path.with_extension("db-wal");
        let shm_path = db_path.with_extension("db-shm");
        let _ = fs::remove_file(wal_path);
        let _ = fs::remove_file(shm_path);
    }

    /// Lists all database backup directories in the backup directory, sorted by modification time.
    ///
    /// # Returns
    ///
    /// A vector of paths to backup directories, sorted with newest first
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if reading the backup directory fails
    pub fn list_backups(&self) -> Result<Vec<PathBuf>, StoreError> {
        if !self.backup_dir.exists() {
            return Err(StoreError::IoError(
                t!("db.backup.dir_not_found", path = self.backup_dir.display()).to_string(),
            ));
        }

        let mut backups: Vec<PathBuf> = fs::read_dir(&self.backup_dir)
            .map_err(|e| {
                StoreError::IoError(
                    t!(
                        "db.backup.failed_to_read_backup_dir",
                        path = self.backup_dir.display(),
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                // Only include directories that match our timestamp format
                if path.is_dir() && Self::is_backup_directory(&path) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        // Sort by modification time, newest first
        backups.sort_by(|a, b| {
            b.metadata()
                .and_then(|m| m.modified())
                .unwrap_or_else(|_| std::time::SystemTime::UNIX_EPOCH)
                .cmp(
                    &a.metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or_else(|_| std::time::SystemTime::UNIX_EPOCH),
                )
        });

        Ok(backups)
    }

    /// Checks if a directory name matches our backup timestamp format
    fn is_backup_directory(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .map(|name| {
                // Check if name matches format "YYYY-MM-DD_HH-MM-SS"
                let re = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}-\d{2}$").unwrap();
                re.is_match(name)
            })
            .unwrap_or(false)
    }

    fn move_completed_backup(source_dir: &Path, destination_dir: &Path) -> Result<(), StoreError> {
        // Custom destinations can be on a different volume (for example iCloud Drive), where
        // rename fails with EXDEV. Reserve the destination with an exclusive directory create so
        // another backup cannot race this copy and overwrite a completed archive.
        fs::create_dir(destination_dir).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_backup_dir_at",
                    path = destination_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        if let Err(e) = copy_directory(source_dir, destination_dir) {
            error!("Failed to copy completed backup directory: {}", e);
            if let Err(cleanup_error) = fs::remove_dir_all(destination_dir) {
                error!(
                    "Failed to remove incomplete copied backup directory {}: {}",
                    destination_dir.display(),
                    cleanup_error
                );
            }
            return Err(StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_backup_dir_at",
                    path = destination_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            ));
        }

        fs::remove_dir_all(source_dir).map_err(|e| {
            error!("Failed to remove copied staged backup directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_backup_dir_at",
                    path = source_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })
    }

    fn assemble_staged_backup(&self) -> Result<(), StoreError> {
        let result = (|| {
            // Assemble all archives locally before exposing a completed backup in a custom destination.
            let _ = self.backup()?;

            let theme_dir = &*crate::HTTP_SERVER_THEME_DIR.read();
            let upload_dir = &*crate::HTTP_SERVER_UPLOAD_DIR.read();
            let mcp_sessions_dir = &*crate::STORE_DIR.read().join("mcp_sessions");
            let schema_dir = &*crate::SCHEMA_DIR.read();
            let shared_dir = &*crate::SHARED_DATA_DIR.read();
            let static_dir = &*crate::HTTP_SERVER_DIR.read();
            self.backup_user_files(
                Path::new(&theme_dir),
                Path::new(&upload_dir),
                Path::new(&mcp_sessions_dir),
                Path::new(&schema_dir),
                Path::new(&shared_dir),
                Path::new(&static_dir),
            )
            .map(|_| ())
        })();

        if result.is_err() {
            if let Err(error) = fs::remove_dir_all(&self.backup_dir) {
                error!("Failed to remove incomplete staged backup: {}", error);
            }
        }

        result
    }

    /// Backs up databases and user files to a specified directory
    ///
    /// # Returns
    ///  Returns a `StoreError` if any backup operation fails
    pub fn backup_to_directory(&mut self) -> Result<(), StoreError> {
        let backup_name = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let destination_dir = self.backup_dir.join(&backup_name);
        let staging_dir = self.staging_backup_dir.join(&backup_name);

        fs::create_dir(&staging_dir).map_err(|e| {
            error!("Failed to create backup directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_backup_dir_at",
                    path = staging_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        let staged_backup = Self {
            main_db_path: self.main_db_path.clone(),
            backup_dir: staging_dir.clone(),
            staging_backup_dir: self.staging_backup_dir.clone(),
        };

        staged_backup.assemble_staged_backup()?;

        if destination_dir != staging_dir {
            Self::move_completed_backup(&staging_dir, &destination_dir)?;
        }

        Ok(())
    }

    /// Restores user files from a backup zip file
    ///
    /// # Arguments
    ///
    /// * `zip_path` - Path to the backup zip file
    /// * `theme_dir` - Path to restore theme files
    /// * `upload_dir` - Path to restore uploaded files
    /// * `mcp_sessions_dir` - Path to restore MCP sessions
    /// * `schema_dir` - Path to restore schema files
    /// * `shared_dir` - Path to restore shared files
    /// * `static_dir` - Path to restore static files
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if restore operation fails
    /// Restores user files from a ZIP archive.
    ///
    /// Returns Ok(true) if some files were skipped due to locks, Ok(false) if all files were restored.
    pub fn restore_user_files(
        &self,
        zip_path: &Path,
        theme_dir: &Path,
        upload_dir: &Path,
        mcp_sessions_dir: &Path,
        schema_dir: &Path,
        shared_dir: &Path,
        static_dir: &Path,
    ) -> Result<bool, StoreError> {
        let mut files_skipped = false; // Track if any files were skipped due to locks

        let file = File::open(zip_path).map_err(|e| {
            error!("Failed to open zip file: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_open_zip",
                    path = zip_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        let mut archive = ZipArchive::new(file).map_err(|e| {
            error!("Failed to read zip archive: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_read_zip_archive",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Create target directories if they don't exist
        fs::create_dir_all(theme_dir).map_err(|e| {
            error!("Failed to create theme directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_theme_dir",
                    path = theme_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        fs::create_dir_all(upload_dir).map_err(|e| {
            error!("Failed to create upload directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_upload_dir",
                    path = upload_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        fs::create_dir_all(mcp_sessions_dir).map_err(|e| {
            error!("Failed to create MCP sessions directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_mcp_sessions_dir",
                    path = mcp_sessions_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        fs::create_dir_all(schema_dir).map_err(|e| {
            error!("Failed to create schema directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_schema_dir",
                    path = schema_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        fs::create_dir_all(shared_dir).map_err(|e| {
            error!("Failed to create shared directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_shared_dir",
                    path = shared_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        fs::create_dir_all(static_dir).map_err(|e| {
            error!("Failed to create static directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_static_dir",
                    path = static_dir.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Extract files
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| {
                error!("Failed to read zip entry: {}", e);
                StoreError::IoError(
                    t!(
                        "db.backup.failed_to_read_zip_entry",
                        index = i,
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

            let outpath = match file.name() {
                name if name.starts_with("themes/") => {
                    let rel_path = Path::new(name.trim_start_matches("themes/"));
                    theme_dir.join(rel_path)
                }
                name if name.starts_with("uploads/") => {
                    let rel_path = Path::new(name.trim_start_matches("uploads/"));
                    upload_dir.join(rel_path)
                }
                name if name.starts_with("mcp_sessions/") => {
                    let rel_path = Path::new(name.trim_start_matches("mcp_sessions/"));
                    mcp_sessions_dir.join(rel_path)
                }
                name if name.starts_with("schema/") => {
                    let rel_path = Path::new(name.trim_start_matches("schema/"));
                    schema_dir.join(rel_path)
                }
                name if name.starts_with("shared/") => {
                    let rel_path = Path::new(name.trim_start_matches("shared/"));
                    shared_dir.join(rel_path)
                }
                name if name.starts_with("static/") => {
                    let rel_path = Path::new(name.trim_start_matches("static/"));
                    static_dir.join(rel_path)
                }
                _ => continue, // Skip files not in themes/, uploads/, mcp_sessions/, schema/, shared/ or static/
            };

            if file.name().ends_with('/') {
                fs::create_dir_all(&outpath).map_err(|e| {
                    error!("Failed to create directory {}: {}", outpath.display(), e);
                    StoreError::IoError(
                        t!(
                            "db.backup.failed_to_create_dir_from_zip",
                            path = outpath.display(),
                            error = e.to_string()
                        )
                        .to_string(),
                    )
                })?;
            } else {
                if let Some(p) = outpath.parent() {
                    fs::create_dir_all(p).map_err(|e| {
                        error!("Failed to create parent directory {}: {}", p.display(), e);
                        StoreError::IoError(
                            t!(
                                "db.backup.failed_to_create_parent_dir_from_zip",
                                path = p.display(),
                                error = e.to_string()
                            )
                            .to_string(),
                        )
                    })?;
                }

                // Try to create and write file, but handle file locking gracefully.
                // This matters for session artifacts that may still be in use during restore.
                match File::create(&outpath) {
                    Ok(mut outfile) => {
                        if let Err(e) = std::io::copy(&mut file, &mut outfile) {
                            // Check if this is a file locking error (error code 33 on Windows/Linux)
                            if let Some(os_error) = e.raw_os_error() {
                                if os_error == 33 {
                                    // File is locked - skip it with a warning
                                    log::warn!(
                                        "Skipping locked file {}: {}. Please restart the application to complete restoration.",
                                        outpath.display(),
                                        e
                                    );
                                    files_skipped = true;
                                    continue;
                                }
                            }
                            // For other errors, fail
                            error!("Failed to write file {}: {}", outpath.display(), e);
                            return Err(StoreError::IoError(
                                t!(
                                    "db.backup.failed_to_write_file_from_zip",
                                    path = outpath.display(),
                                    error = e.to_string()
                                )
                                .to_string(),
                            ));
                        }
                    }
                    Err(e) => {
                        // Check if this is a file locking error
                        if let Some(os_error) = e.raw_os_error() {
                            if os_error == 33 {
                                // File is locked - skip it with a warning
                                log::warn!(
                                    "Skipping locked file {}: {}. Please restart the application to complete restoration.",
                                    outpath.display(),
                                    e
                                );
                                files_skipped = true;
                                continue;
                            }
                        }
                        // For other errors, fail
                        error!("Failed to create file {}: {}", outpath.display(), e);
                        return Err(StoreError::IoError(
                            t!(
                                "db.backup.failed_to_create_file_from_zip",
                                path = outpath.display(),
                                error = e.to_string()
                            )
                            .to_string(),
                        ));
                    }
                }
            }

            // Get and set permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    fs::set_permissions(&outpath, fs::Permissions::from_mode(mode)).map_err(
                        |e| {
                            error!("Failed to set permissions for {}: {}", outpath.display(), e);
                            StoreError::IoError(
                                t!(
                                    "db.backup.failed_to_set_permissions_from_zip",
                                    path = outpath.display(),
                                    error = e.to_string()
                                )
                                .to_string(),
                            )
                        },
                    )?;
                }
            }
        }

        if files_skipped {
            log::warn!("User files restored with some files skipped due to file locks. Please restart the application to complete restoration.");
        } else {
            info!("User files restored successfully");
        }
        Ok(files_skipped)
    }

    /// Creates a zip archive containing all files from theme, upload, mcp_sessions, schema, shared and static directories
    ///
    /// # Arguments
    ///
    /// * `theme_dir` - Path to the theme directory
    /// * `upload_dir` - Path to the upload directory
    /// * `mcp_sessions_dir` - Path to the MCP sessions directory
    /// * `schema_dir` - Path to the schema directory
    /// * `shared_dir` - Path to the shared directory
    /// * `static_dir` - Path to the static directory
    ///
    /// # Returns
    ///
    /// Result containing the path to the created zip file
    pub fn backup_user_files(
        &self,
        theme_dir: &Path,
        upload_dir: &Path,
        mcp_sessions_dir: &Path,
        schema_dir: &Path,
        shared_dir: &Path,
        static_dir: &Path,
    ) -> Result<PathBuf, StoreError> {
        let output_path = self.backup_dir.join("user_files.zip");

        let file = fs::File::create(output_path.clone()).map_err(|e| {
            error!("Failed to create zip file: {}", e);
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_create_zip_for_backup",
                    path = output_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        let mut zip = ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions = FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o644);

        // Helper closure to add directory contents to zip
        let add_dir_to_zip =
            |zip: &mut ZipWriter<_>, base_path: &Path, prefix: &str| -> Result<(), StoreError> {
                for entry in WalkDir::new(base_path).into_iter().filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_file() {
                        let relative_path = path.strip_prefix(base_path).map_err(|e| {
                            error!("Failed to strip prefix: {}", e);
                            StoreError::IoError(
                                t!(
                                    "db.backup.failed_to_strip_prefix_for_zip",
                                    base = base_path.display(),
                                    full = path.display(),
                                    error = e.to_string()
                                )
                                .to_string(),
                            )
                        })?;

                        let zip_path = format!("{}/{}", prefix, relative_path.to_string_lossy());

                        zip.start_file(&zip_path, options).map_err(|e| {
                            error!("Failed to add file to zip: {}", e);
                            StoreError::IoError(
                                t!(
                                    "db.backup.failed_to_add_file_to_zip",
                                    path = zip_path,
                                    error = e.to_string()
                                )
                                .to_string(),
                            )
                        })?;

                        let mut file = fs::File::open(path).map_err(|e| {
                            error!("Failed to open file for zip: {}", e);
                            StoreError::IoError(
                                t!(
                                    "db.backup.failed_to_open_file_for_zip",
                                    path = path.display(),
                                    error = e.to_string()
                                )
                                .to_string(),
                            )
                        })?;

                        let mut buffer = Vec::new();
                        file.read_to_end(&mut buffer).map_err(|e| {
                            error!("Failed to read file: {}", e);
                            StoreError::IoError(
                                t!(
                                    "db.backup.failed_to_read_file_for_zip",
                                    error = e.to_string()
                                )
                                .to_string(),
                            )
                        })?;

                        zip.write_all(&buffer).map_err(|e| {
                            error!("Failed to write to zip: {}", e);
                            StoreError::IoError(
                                t!("db.backup.failed_to_write_to_zip", error = e.to_string())
                                    .to_string(),
                            )
                        })?;
                    }
                }
                Ok(())
            };

        // Add theme directory contents
        if theme_dir.exists() {
            add_dir_to_zip(&mut zip, theme_dir, "themes")?;
        }

        // Add upload directory contents
        if upload_dir.exists() {
            add_dir_to_zip(&mut zip, upload_dir, "uploads")?;
        }

        // Add MCP sessions directory contents
        if mcp_sessions_dir.exists() {
            add_dir_to_zip(&mut zip, mcp_sessions_dir, "mcp_sessions")?;
        }

        // Add schema directory contents
        if schema_dir.exists() {
            add_dir_to_zip(&mut zip, schema_dir, "schema")?;
        }

        // Add shared directory contents
        if shared_dir.exists() {
            add_dir_to_zip(&mut zip, shared_dir, "shared")?;
        }

        // Add static directory contents
        if static_dir.exists() {
            add_dir_to_zip(&mut zip, static_dir, "static")?;
        }

        zip.finish().map_err(|e| {
            error!("Failed to finish zip file: {}", e);
            StoreError::IoError(
                t!("db.backup.failed_to_finish_zip", error = e.to_string()).to_string(),
            )
        })?;

        info!("Created user files backup: {:?}", output_path.clone());
        Ok(output_path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::{copy_directory, DbBackup};
    use crate::db::backup_crypto::encrypt_database_streaming;
    use std::fs::{self, File};

    fn backup_with_dir(
        backup_dir: std::path::PathBuf,
        source_path: std::path::PathBuf,
    ) -> DbBackup {
        fs::create_dir_all(&backup_dir).unwrap();
        DbBackup {
            main_db_path: source_path,
            staging_backup_dir: backup_dir.clone(),
            backup_dir,
        }
    }

    #[test]
    fn backup_does_not_overwrite_existing_archive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_path = temp_dir.path().join("chatspeed.db");
        let backup_dir = temp_dir.path().join("2026-07-21_12-00-02");
        let existing_backup_path = backup_dir.join("chatspeed.db.zip");
        let existing_contents = b"existing encrypted backup";
        fs::write(&source_path, b"new database backup").unwrap();

        let backup = backup_with_dir(backup_dir, source_path.clone());
        fs::write(&existing_backup_path, existing_contents).unwrap();

        assert!(backup.backup_single_db(&source_path, "chatspeed").is_err());
        assert_eq!(fs::read(existing_backup_path).unwrap(), existing_contents);
    }

    #[test]
    fn failed_legacy_decryption_cleans_up_temporary_database() {
        let temp_dir = tempfile::tempdir().unwrap();
        let backup_dir = temp_dir.path().join("2026-07-21_12-00-03");
        let source_path = temp_dir.path().join("chatspeed.db");
        let corrupt_backup_path = backup_dir.join("chatspeed.db");
        let target_path = temp_dir.path().join("restored.db");
        let backup = backup_with_dir(backup_dir, source_path);
        fs::write(&corrupt_backup_path, b"corrupt encrypted backup").unwrap();

        assert!(backup
            .decrypt_to_temp(&corrupt_backup_path, &target_path)
            .is_err());
        assert!(fs::read_dir(temp_dir.path())
            .unwrap()
            .all(|entry| entry.unwrap().path() != target_path));
    }

    #[test]
    fn compressed_encrypted_backup_round_trips_database_contents() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_path = temp_dir.path().join("chatspeed.db");
        let backup_dir = temp_dir.path().join("2026-07-21_12-00-00");
        let restored_path = temp_dir.path().join("restored.db");
        let contents = vec![b'a'; 64 * 1024];
        fs::write(&source_path, &contents).unwrap();

        let backup = backup_with_dir(backup_dir.clone(), source_path.clone());
        let backup_path = backup.backup_single_db(&source_path, "chatspeed").unwrap();
        let temp_db_file = backup
            .decrypt_to_temp(&backup_path, &restored_path)
            .unwrap();

        assert_eq!(backup_path.file_name().unwrap(), "chatspeed.db.zip");
        assert_eq!(fs::read(&temp_db_file).unwrap(), contents);
        assert!(fs::read_dir(&backup_dir)
            .unwrap()
            .all(|entry| entry.unwrap().path() == backup_path));
    }

    #[test]
    fn legacy_encrypted_database_backup_still_restores() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_path = temp_dir.path().join("chatspeed.db");
        let backup_dir = temp_dir.path().join("2026-07-21_12-00-01");
        let legacy_backup_path = backup_dir.join("chatspeed.db");
        let restored_path = temp_dir.path().join("restored.db");
        let contents = b"legacy encrypted database backup";
        fs::write(&source_path, contents).unwrap();

        let backup = backup_with_dir(backup_dir.clone(), source_path);
        encrypt_database_streaming(
            &backup.main_db_path,
            &legacy_backup_path,
            "2026-07-21_12-00-01",
        )
        .unwrap();
        let temp_db_file = backup
            .decrypt_to_temp(&legacy_backup_path, &restored_path)
            .unwrap();

        assert_eq!(fs::read(&temp_db_file).unwrap(), contents);
    }

    #[test]
    fn compress_file_round_trips_database_contents() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_path = temp_dir.path().join("chatspeed.db");
        let zip_path = temp_dir.path().join("chatspeed.db.archive.tmp");
        let restored_path = temp_dir.path().join("restored.db");
        let contents = vec![b'a'; 64 * 1024];
        fs::write(&source_path, &contents).unwrap();

        DbBackup::compress_file(
            &source_path,
            File::create(&zip_path).unwrap(),
            "chatspeed.db",
        )
        .unwrap();
        DbBackup::extract_file(
            &zip_path,
            "chatspeed.db",
            File::create(&restored_path).unwrap(),
        )
        .unwrap();

        assert_eq!(fs::read(&restored_path).unwrap(), contents);
        assert!(fs::metadata(&zip_path).unwrap().len() < contents.len() as u64);
    }

    #[test]
    fn copy_directory_does_not_overwrite_existing_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("source");
        let destination_dir = temp_dir.path().join("destination");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        fs::write(source_dir.join("chatspeed.db.zip"), b"new archive").unwrap();
        fs::write(
            destination_dir.join("chatspeed.db.zip"),
            b"existing archive",
        )
        .unwrap();

        assert!(copy_directory(&source_dir, &destination_dir).is_err());
        assert_eq!(
            fs::read(destination_dir.join("chatspeed.db.zip")).unwrap(),
            b"existing archive"
        );
    }

    #[test]
    fn completed_backup_directory_moves_all_archives() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("staging").join("2026-07-21_12-00-04");
        let destination_dir = temp_dir
            .path()
            .join("destination")
            .join("2026-07-21_12-00-04");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(destination_dir.parent().unwrap()).unwrap();
        fs::write(source_dir.join("chatspeed.db.zip"), b"database archive").unwrap();
        fs::write(source_dir.join("user_files.zip"), b"user files archive").unwrap();

        DbBackup::move_completed_backup(&source_dir, &destination_dir).unwrap();

        assert!(!source_dir.exists());
        assert_eq!(
            fs::read(destination_dir.join("chatspeed.db.zip")).unwrap(),
            b"database archive"
        );
        assert_eq!(
            fs::read(destination_dir.join("user_files.zip")).unwrap(),
            b"user files archive"
        );
    }

    #[test]
    fn completed_backup_directory_does_not_overwrite_existing_destination() {
        let temp_dir = tempfile::tempdir().unwrap();
        let source_dir = temp_dir.path().join("staging").join("2026-07-21_12-00-05");
        let destination_dir = temp_dir
            .path()
            .join("destination")
            .join("2026-07-21_12-00-05");
        fs::create_dir_all(&source_dir).unwrap();
        fs::create_dir_all(&destination_dir).unwrap();
        fs::write(source_dir.join("chatspeed.db.zip"), b"new database archive").unwrap();
        fs::write(
            destination_dir.join("chatspeed.db.zip"),
            b"existing database archive",
        )
        .unwrap();

        assert!(DbBackup::move_completed_backup(&source_dir, &destination_dir).is_err());
        assert!(source_dir.exists());
        assert_eq!(
            fs::read(destination_dir.join("chatspeed.db.zip")).unwrap(),
            b"existing database archive"
        );
    }

    #[test]
    fn incomplete_staged_backup_is_removed_after_assembly_failure() {
        let temp_dir = tempfile::tempdir().unwrap();
        let backup_dir = temp_dir.path().join("2026-07-21_12-00-06");
        let missing_database_path = temp_dir.path().join("missing.db");
        let backup = backup_with_dir(backup_dir.clone(), missing_database_path);

        assert!(backup.assemble_staged_backup().is_err());
        assert!(!backup_dir.exists());
    }

    #[test]
    fn incomplete_staging_directories_are_excluded_from_backup_list() {
        let temp_dir = tempfile::tempdir().unwrap();
        let backup_root = temp_dir.path().join("backup");
        let staging_dir = backup_root.join(".staging").join("2026-07-21_12-00-08");
        let completed_dir = backup_root.join("2026-07-21_12-00-09");
        fs::create_dir_all(&staging_dir).unwrap();
        fs::create_dir_all(&completed_dir).unwrap();
        let backup = DbBackup {
            main_db_path: temp_dir.path().join("chatspeed.db"),
            backup_dir: backup_root.clone(),
            staging_backup_dir: backup_root.join(".staging"),
        };

        assert_eq!(backup.list_backups().unwrap(), vec![completed_dir]);
    }
}
