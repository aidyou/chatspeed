use crate::db::error::StoreError;
use chrono::Local;
use log::{error, info};
use regex;
use rusqlite::backup::StepResult;
use rusqlite::{backup, Connection};
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

/// Configuration for database backup operations.
pub struct BackupConfig {
    /// Directory path where backups will be stored
    pub backup_dir: Option<String>,
    /// Whether to backup the chat database
    pub backup_workflow_db: Option<bool>,
}

/// Manages database backup and restore operations.
pub struct DbBackup {
    main_db_path: PathBuf,
    workflow_db_path: PathBuf,
    backup_dir: PathBuf,
    backup_workflow_db: bool,
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
            let app_local_data_dir = _app
                .path()
                .app_data_dir()
                .expect("Failed to retrieve the application data directory");
            std::fs::create_dir_all(&app_local_data_dir)
                .map_err(|e| StoreError::TauriError(e.to_string()))?;
            app_local_data_dir
        };

        // Ensure backup directory exists
        let backup_dir = match config.backup_dir.as_deref() {
            None | Some("") => app_dir.join("backup"),
            Some(dir) => PathBuf::from(dir),
        };
        // let backup_dir = backup_dir.join(Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());

        if !backup_dir.exists() {
            fs::create_dir_all(&backup_dir).map_err(|e| {
                error!("Failed to create backup directory: {}", e);
                StoreError::IoError(
                    t!(
                        "db.failed_to_create_backup_dir_at",
                        path = backup_dir.display(),
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;
        }

        Ok(Self {
            main_db_path: app_dir.join("chatspeed.db"),
            workflow_db_path: app_dir.join("workflow.db"),
            backup_dir,
            backup_workflow_db: config.backup_workflow_db.unwrap_or(true),
        })
    }

    /// Performs database backup operations.
    ///
    /// # Arguments
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

        // Backup config database
        let config_backup_path = self.backup_single_db(&self.main_db_path, "chatspeed")?;
        backup_paths.push(config_backup_path);

        // Backup chat database if requested
        if self.backup_workflow_db && self.workflow_db_path.exists() {
            let chat_backup_path = self.backup_single_db(&self.workflow_db_path, "workflow")?;
            backup_paths.push(chat_backup_path);
        }

        Ok(backup_paths)
    }

    /// Copies database from source to destination with progress tracking
    ///
    /// # Arguments
    ///
    /// * `source_path` - Path to the source database
    /// * `dest_path` - Path to the destination database
    /// * `db_type` - Type of database (for logging purposes)
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the operation fails
    fn copy_database(
        source_path: &Path,
        dest_path: &Path,
        db_type: &str,
    ) -> Result<(), StoreError> {
        // Open source database
        let source = Connection::open(source_path).map_err(|e| {
            error!("Failed to open source database: {}", e);
            StoreError::DatabaseError(
                t!(
                    "db.failed_to_open_source_db",
                    path = source_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Open destination database
        let mut dest = Connection::open(dest_path).map_err(|e| {
            error!("Failed to open destination database: {}", e);
            StoreError::DatabaseError(
                t!(
                    "db.failed_to_open_dest_db",
                    path = dest_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Perform copy operation
        let backup = backup::Backup::new(&source, &mut dest).map_err(|e| {
            error!("Failed to initialize backup: {}", e);
            StoreError::DatabaseError(
                t!("db.failed_to_init_backup_process", error = e.to_string()).to_string(),
            )
        })?;

        let mut retry_count = 0;
        loop {
            match backup.step(1000) {
                Ok(StepResult::Done) => {
                    info!(
                        "Database copy completed from {:?} to {:?}",
                        source_path, dest_path
                    );
                    break;
                }
                Ok(StepResult::Busy) | Ok(StepResult::Locked) => {
                    retry_count += 1;
                    if retry_count > 10 {
                        return Err(StoreError::DatabaseError(
                            t!("db.backup_busy_locked").to_string(),
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                Ok(StepResult::More) | Ok(_) => {
                    let progress = backup.progress();
                    let percent = ((progress.pagecount - progress.remaining) as f64
                        / progress.pagecount as f64
                        * 100.0) as u32;
                    info!("Progress for {}.db: {}%", db_type, percent);
                }
                Err(e) => {
                    error!("Failed step: {}", e);
                    return Err(StoreError::from(e));
                }
            }
        }

        let progress = backup.progress();
        if progress.remaining > 0 {
            error!(
                "Database copy incomplete: {} pages remaining out of {}",
                progress.remaining, progress.pagecount
            );
            return Err(StoreError::DatabaseError(
                t!("db.backup_copy_incomplete").to_string(),
            ));
        }

        Ok(())
    }

    /// Backs up a single database file
    fn backup_single_db(&self, db_path: &Path, db_type: &str) -> Result<PathBuf, StoreError> {
        let backup_path = self.backup_dir.join(format!("{}.db", db_type));
        Self::copy_database(db_path, &backup_path, db_type)?;
        Ok(backup_path)
    }

    /// Restores a single database from a backup file
    fn restore_single_db(&self, backup_path: &Path, target_path: &Path) -> Result<(), StoreError> {
        if !backup_path.exists() {
            return Err(StoreError::NotFound(
                t!("db.backup_file_not_found", path = backup_path.display()).to_string(),
            ));
        }

        let db_type = target_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");
        Self::copy_database(backup_path, target_path, db_type)
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
                t!("db.backup_dir_not_found", path = self.backup_dir.display()).to_string(),
            ));
        }

        let mut backups: Vec<PathBuf> = fs::read_dir(&self.backup_dir)
            .map_err(|e| {
                StoreError::IoError(
                    t!(
                        "db.failed_to_read_backup_dir",
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

    /// Backs up databases and user files to a specified directory
    ///
    /// # Returns
    ///  Returns a `StoreError` if any backup operation fails
    pub fn backup_to_directory(&mut self) -> Result<(), StoreError> {
        self.backup_dir = self
            .backup_dir
            .join(Local::now().format("%Y-%m-%d_%H-%M-%S").to_string());
        if !self.backup_dir.exists() {
            fs::create_dir_all(&self.backup_dir).map_err(|e| {
                error!("Failed to create backup directory: {}", e);
                StoreError::IoError(
                    t!(
                        "db.failed_to_create_backup_dir_at",
                        path = self.backup_dir.display(),
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;
        }
        // Backup databases
        let _ = self.backup()?;

        // Backup user files
        let theme_dir = &*crate::HTTP_SERVER_THEME_DIR.read();
        let upload_dir = &*crate::HTTP_SERVER_UPLOAD_DIR.read();
        self.backup_user_files(Path::new(&theme_dir), Path::new(&upload_dir))?;

        Ok(())
    }

    /// Restores databases and user files from a backup directory
    ///
    /// # Arguments
    ///
    /// * `backup_dir` - Path to the backup directory
    /// * `theme_dir` - Path to restore theme files
    /// * `upload_dir` - Path to restore uploaded files
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if any restore operation fails
    pub fn restore_from_directory(
        &self,
        backup_dir: &Path,
        theme_dir: &Path,
        upload_dir: &Path,
    ) -> Result<(), StoreError> {
        // Verify backup directory exists
        if !backup_dir.exists() || !backup_dir.is_dir() {
            return Err(StoreError::NotFound(
                t!(
                    "db.backup_dir_not_found_for_restore",
                    path = backup_dir.display()
                )
                .to_string(),
            ));
        }

        // Check for config.db
        let config_backup = backup_dir.join("chatspeed.db");
        if config_backup.exists() {
            self.restore_single_db(&config_backup, &self.main_db_path)?;
        }

        // Check for chat.db
        let chat_backup = backup_dir.join("workflow.db");
        if chat_backup.exists() {
            self.restore_single_db(&chat_backup, &self.workflow_db_path)?;
        }

        // Check for user_files.zip
        let user_files = backup_dir.join("user_files.zip");
        if user_files.exists() {
            self.restore_user_files(&user_files, theme_dir, upload_dir)?;
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
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if restore operation fails
    fn restore_user_files(
        &self,
        zip_path: &Path,
        theme_dir: &Path,
        upload_dir: &Path,
    ) -> Result<(), StoreError> {
        let file = File::open(zip_path).map_err(|e| {
            error!("Failed to open zip file: {}", e);
            StoreError::IoError(
                t!(
                    "db.failed_to_open_zip",
                    path = zip_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        let mut archive = ZipArchive::new(file).map_err(|e| {
            error!("Failed to read zip archive: {}", e);
            StoreError::IoError(
                t!("db.failed_to_read_zip_archive", error = e.to_string()).to_string(),
            )
        })?;

        // Create target directories if they don't exist
        fs::create_dir_all(theme_dir).map_err(|e| {
            error!("Failed to create theme directory: {}", e);
            StoreError::IoError(
                t!(
                    "db.failed_to_create_theme_dir",
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
                    "db.failed_to_create_upload_dir",
                    path = upload_dir.display(),
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
                        "db.failed_to_read_zip_entry",
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
                _ => continue, // Skip files not in themes/ or uploads/
            };

            if file.name().ends_with('/') {
                fs::create_dir_all(&outpath).map_err(|e| {
                    error!("Failed to create directory {}: {}", outpath.display(), e);
                    StoreError::IoError(
                        t!(
                            "db.failed_to_create_dir_from_zip",
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
                                "db.failed_to_create_parent_dir_from_zip",
                                path = p.display(),
                                error = e.to_string()
                            )
                            .to_string(),
                        )
                    })?;
                }
                let mut outfile = File::create(&outpath).map_err(|e| {
                    error!("Failed to create file {}: {}", outpath.display(), e);
                    StoreError::IoError(
                        t!(
                            "db.failed_to_create_file_from_zip",
                            path = outpath.display(),
                            error = e.to_string()
                        )
                        .to_string(),
                    )
                })?;
                std::io::copy(&mut file, &mut outfile).map_err(|e| {
                    error!("Failed to write file {}: {}", outpath.display(), e);
                    StoreError::IoError(
                        t!(
                            "db.failed_to_write_file_from_zip",
                            path = outpath.display(),
                            error = e.to_string()
                        )
                        .to_string(),
                    )
                })?;
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
                                    "db.failed_to_set_permissions_from_zip",
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

        info!("User files restored successfully");
        Ok(())
    }

    /// Creates a zip archive containing all files from theme and upload directories
    ///
    /// # Arguments
    ///
    /// * `theme_dir` - Path to the theme directory
    /// * `upload_dir` - Path to the upload directory
    ///
    /// # Returns
    ///
    /// Result containing the path to the created zip file
    pub fn backup_user_files(
        &self,
        theme_dir: &Path,
        upload_dir: &Path,
    ) -> Result<PathBuf, StoreError> {
        let output_path = self.backup_dir.join("user_files.zip");

        let file = fs::File::create(output_path.clone()).map_err(|e| {
            error!("Failed to create zip file: {}", e);
            StoreError::IoError(
                t!(
                    "db.failed_to_create_zip_for_backup",
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
                                    "db.failed_to_strip_prefix_for_zip",
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
                                    "db.failed_to_add_file_to_zip",
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
                                    "db.failed_to_open_file_for_zip",
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
                                t!("db.failed_to_read_file_for_zip", error = e.to_string())
                                    .to_string(),
                            )
                        })?;

                        zip.write_all(&buffer).map_err(|e| {
                            error!("Failed to write to zip: {}", e);
                            StoreError::IoError(
                                t!("db.failed_to_write_to_zip", error = e.to_string()).to_string(),
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

        zip.finish().map_err(|e| {
            error!("Failed to finish zip file: {}", e);
            StoreError::IoError(t!("db.failed_to_finish_zip", error = e.to_string()).to_string())
        })?;

        info!("Created user files backup: {:?}", output_path.clone());
        Ok(output_path.to_path_buf())
    }
}
