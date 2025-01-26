use crate::db::error::StoreError;

use log::error;
use rusqlite::{params, Connection, Result};

use rust_i18n::t;
use serde_json::Value;
use std::collections::HashMap;
use tauri::AppHandle;
// Required for AppHandle::path() method even when using fully qualified syntax (<AppHandle as Manager>::path)
// DO NOT REMOVE: This trait import is necessary for the Manager trait to be in scope
#[allow(unused_imports)]
use tauri::Manager;

use super::{sql::migrations::manager, types::Config, AiModel, AiSkill};

impl Config {
    /// Retrieves the value associated with the specified key from the settings.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the setting to retrieve.
    ///
    /// # Returns
    ///
    /// Returns an `Option<&Value>` containing the value if found, or `None` if the key does not exist.
    pub fn get_setting(&self, key: &str) -> Option<&Value> {
        self.settings.get(key)
    }

    /// Updates the value associated with the specified key in the settings.
    ///
    /// If the key already exists, its value will be replaced. If it does not exist, a new key-value pair will be added.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the setting to update.
    /// * `value` - The new value to associate with the key.
    pub fn update_setting(&mut self, key: &str, value: Value) {
        if let Some(old_value) = self.settings.get_mut(key) {
            *old_value = value;
        } else {
            self.settings.insert(key.to_string(), value);
        }
    }

    /// Retrieves an AI model by its ID.
    ///
    /// # Arguments
    /// * `id` - The ID of the AI model to retrieve.
    ///
    /// # Returns
    /// Returns an `Option<AiModel>` containing the AI model if found, or `None` if not found.
    pub fn get_ai_model_by_id(&self, id: i64) -> Result<AiModel, StoreError> {
        self.ai_models
            .iter()
            .find(|m| m.id == Some(id))
            .cloned()
            .ok_or_else(|| StoreError::StringError(format!("Failed to find the AI model: {}", id)))
    }

    /// Retrieves a thread-safe clone of the AI models.
    ///
    /// # Returns
    ///
    /// Returns an `Arc<Vec<AiModel>>` containing the AI models.
    pub fn get_ai_models(&self) -> Vec<AiModel> {
        self.ai_models.clone()
    }

    /// Retrieves an AI skill by its ID.
    ///
    /// # Arguments
    /// * `id` - The ID of the AI skill to retrieve.
    ///
    /// # Returns
    /// Returns an `Option<AiSkill>` containing the AI skill if found, or `None` if not found.
    pub fn get_ai_skill_by_id(&self, id: i64) -> Result<AiSkill, StoreError> {
        self.ai_skills
            .iter()
            .find(|s| s.id == Some(id))
            .cloned()
            .ok_or_else(|| StoreError::StringError(format!("Failed to find the AI skill: {}", id)))
    }

    /// Retrieves a thread-safe clone of the AI skills.
    ///
    /// # Returns
    ///
    /// Returns an `Arc<Vec<AiSkill>>` containing the AI skills.
    pub fn get_ai_skills(&self) -> Vec<AiSkill> {
        self.ai_skills.clone()
    }

    /// Sets the AI models in the configuration.
    ///
    /// This method replaces the existing AI models with the provided vector.
    ///
    /// # Arguments
    ///
    /// * `ai_models` - A vector of `AiModel` instances to set.
    pub fn set_ai_models(&mut self, ai_models: Vec<AiModel>) {
        self.ai_models = ai_models;
    }

    /// Sets the AI skills in the configuration.
    ///
    /// This method replaces the existing AI skills with the provided vector.
    ///
    /// # Arguments
    ///
    /// * `ai_skills` - A vector of `AiSkill` instances to set.
    pub fn set_ai_skills(&mut self, ai_skills: Vec<AiSkill>) {
        self.ai_skills = ai_skills;
    }
}

/// Manages unified storage for the application, including chat history and configuration.
pub struct MainStore {
    pub(crate) conn: Connection,
    pub(crate) config: Config,
}

impl MainStore {
    /// Creates a new `Store` instance.
    ///
    /// This function initializes the database connection, creates the necessary
    /// tables if they do not exist, and sets up the storage path.
    ///
    /// # Arguments
    ///
    /// * `_app` - A reference to the Tauri `AppHandle`.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database connection or initialization fails.
    pub fn new(_app: &AppHandle) -> Result<Self, StoreError> {
        #[cfg(debug_assertions)]
        let db_path = {
            let dev_dir = &*crate::STORE_DIR.read();
            dev_dir.join("chatspeed.db")
        };

        #[cfg(not(debug_assertions))]
        let db_path = {
            let app_local_data_dir = _app
                .path()
                .app_data_dir()
                .expect(t!("db.failed_to_get_app_data_dir").into());
            std::fs::create_dir_all(&app_local_data_dir)
                .map_err(|e| StoreError::StringError(e.to_string()))?;
            app_local_data_dir.join("chatspeed.db")
        };

        let mut conn = Connection::open(&db_path).map_err(|e| {
            let err = t!("db.failed_to_open_db_connection", error = e.to_string()).to_string();
            log::error!("{}", err);
            StoreError::DatabaseError(err)
        })?;

        Self::init_db(&mut conn).map_err(|e| {
            let err = t!("db.failed_to_initialize_database", error = e.to_string()).to_string();
            log::error!("{}", err);
            StoreError::DatabaseError(err)
        })?;

        Self::migrate_data(_app).map_err(|e| {
            let err = t!("db.failed_to_migrate_database", error = e.to_string()).to_string();
            log::error!("{}", err);
            StoreError::DatabaseError(err)
        })?;

        // let conn = Arc::new(Mutex::new(conn));
        let config = Self::load_config(&conn)?;

        Ok(Self { conn, config })
    }

    /// Loads the configuration from the database.
    ///
    /// This method retrieves all settings, AI models, and AI skills from the database
    /// and constructs a `Config` struct.
    ///
    /// # Arguments
    ///
    /// * `conn` - A reference to the database connection.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the `Config` struct if successful, or a `StoreError` if an error occurs.
    fn load_config(conn: &Connection) -> Result<Config, StoreError> {
        let settings = Self::get_all_config(conn)?;
        let ai_models = Self::get_all_ai_models(conn)?;
        let ai_skills = Self::get_all_ai_skills(conn)?;

        Ok(Config {
            settings,
            ai_models,
            ai_skills,
        })
    }

    /// Reloads the configuration from the database.
    ///
    /// # Returns
    /// Returns a `Result` containing `()` if successful, or a `StoreError` if an error occurs.
    pub fn reload_config(&mut self) -> Result<(), StoreError> {
        match Self::load_config(&self.conn) {
            Ok(config) => {
                self.config = config;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Retrieves all configuration items from the database.
    ///
    /// Fetches all key-value pairs and returns them as a HashMap.
    ///
    /// # Returns
    ///
    /// A HashMap containing all configuration items.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub(crate) fn get_all_config(conn: &Connection) -> Result<HashMap<String, Value>, StoreError> {
        let mut stmt = conn.prepare("SELECT key, value FROM config").map_err(|e| {
            error!("Failed to prepare statement for getting all config: {}", e);
            StoreError::from(e)
        })?;
        let rows = stmt
            .query_map([], |row| {
                let key: String = row.get("key")?;
                let value_str: String = row.get("value")?;
                let value: Value = serde_json::from_str(&value_str).unwrap_or(Value::Null);
                Ok((key, value))
            })
            .map_err(|e| {
                error!("Failed to query rows for all config: {}", e);
                StoreError::from(e)
            })?;

        let mut config_map = HashMap::new();
        for row in rows {
            let (key, value) = row?;
            config_map.insert(key, value);
        }

        Ok(config_map)
    }

    /// Retrieves all AI models from the database.
    ///
    /// Fetches all records from the `ai_model` table ordered by `sort_index`.
    ///
    /// # Returns
    ///
    /// A vector of `AiModel` instances.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub(crate) fn get_all_ai_models(conn: &Connection) -> Result<Vec<AiModel>, StoreError> {
        let mut stmt = conn
            .prepare("SELECT * FROM ai_model ORDER BY sort_index ASC, id ASC")
            .map_err(|e| {
                error!(
                    "Failed to prepare statement for getting all AI models: {}",
                    e
                );
                StoreError::from(e)
            })?;
        let models = stmt
            .query_map([], |row| {
                let metadata_str: Option<String> = row.get("metadata")?;
                let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

                Ok(AiModel {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    models: row
                        .get::<_, String>("models")?
                        .split(',')
                        .map(|s| s.to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                    default_model: row.get("default_model").unwrap_or_default(),
                    api_provider: row.get("api_provider").unwrap_or_default(),
                    base_url: row.get("base_url").unwrap_or_default(),
                    api_key: row.get("api_key").unwrap_or_default(),
                    max_tokens: row.get("max_tokens").unwrap_or(0),
                    temperature: row.get("temperature").unwrap_or(0.0),
                    top_p: row.get("top_p").unwrap_or(0.0),
                    top_k: row.get("top_k").unwrap_or(0),
                    sort_index: row.get("sort_index").unwrap_or(0),
                    is_default: row.get("is_default").unwrap_or(false),
                    disabled: row.get("disabled").unwrap_or(false),
                    is_official: row.get("is_official").unwrap_or(false),
                    official_id: row.get("official_id").unwrap_or_default(),
                    metadata,
                })
            })
            .map_err(|e| {
                error!("Failed to query rows for all AI models: {}", e);
                StoreError::from(e)
            })?;
        models.collect::<Result<Vec<_>, _>>().map_err(|e| {
            error!("Failed to collect AI models: {}", e);
            StoreError::from(e)
        })
    }

    // AI Skill Operations

    /// Retrieves all AI skills from the database.
    ///
    /// Fetches all records from the `ai_skill` table ordered by `sort_index`.
    ///
    /// # Returns
    ///
    /// A vector of `AiSkill` instances.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub(crate) fn get_all_ai_skills(conn: &Connection) -> Result<Vec<AiSkill>, StoreError> {
        let mut stmt = conn.prepare("SELECT * FROM ai_skill ORDER BY sort_index ASC, id ASC")?;
        let skills = stmt.query_map([], |row| {
            let metadata_str: Option<String> = row.get("metadata")?;
            let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

            Ok(AiSkill {
                id: row.get("id")?,
                name: row.get("name").unwrap_or_default(),
                icon: row.get("icon").unwrap_or_default(),
                logo: row.get("logo").unwrap_or_default(),
                prompt: row.get("prompt").unwrap_or_default(),
                share_id: row.get("share_id").unwrap_or_default(),
                sort_index: row.get("sort_index").unwrap_or(0),
                disabled: row.get("disabled").unwrap_or(false),
                metadata,
            })
        })?;
        skills
            .collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Initializes the database by creating necessary tables.
    ///
    /// Creates all required tables if they don't exist.
    ///
    /// # Arguments
    ///
    /// * `conn` - A reference to the SQLite `Connection`.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if any database operation fails during initialization.
    fn init_db(conn: &mut Connection) -> Result<(), StoreError> {
        manager::run_migrations(conn).map_err(|e| {
            log::error!("Failed to initialize database: {}", e);
            e
        })
    }

    /// Migrates data from separate chat and config databases to the unified database.
    ///
    /// # Arguments
    ///
    /// * `_app` - A reference to the Tauri `AppHandle`.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if any database operation fails during migration.
    pub fn migrate_data(_app: &AppHandle) -> Result<(), StoreError> {
        #[cfg(debug_assertions)]
        let app_dir = &*crate::STORE_DIR.read();

        #[cfg(not(debug_assertions))]
        let app_dir = {
            let app_local_data_dir = _app
                .path()
                .app_data_dir()
                .expect(t!("db.failed_to_get_app_data_dir").into());
            std::fs::create_dir_all(&app_local_data_dir)
                .map_err(|e| StoreError::StringError(e.to_string()))?;
            app_local_data_dir
        };

        let chat_db_path = app_dir.join("chat.db");
        let config_db_path = app_dir.join("config.db");
        let new_db_path = app_dir.join("chatspeed.db");
        let backup_dir = app_dir.join("backup");

        if !chat_db_path.exists() && !config_db_path.exists() {
            log::info!("No old databases found, skipping migration");
            return Ok(());
        }

        // Create backup directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&backup_dir) {
            error!(
                "{}",
                t!("db.failed_to_create_backup_dir", error = e.to_string())
            );
            return Err(StoreError::StringError(e.to_string()));
        }

        // Create new database and initialize tables
        let mut new_conn = Connection::open(&new_db_path).map_err(|e| {
            StoreError::StringError(format!("Failed to create new database: {}", e))
        })?;
        Self::init_db(&mut new_conn)?;

        // Migrate chat data if old chat database exists
        if chat_db_path.exists() {
            log::info!("Starting chat database migration");
            let chat_conn = Connection::open(&chat_db_path).map_err(|e| {
                StoreError::StringError(format!("Failed to open chat database: {}", e))
            })?;

            // Migrate conversations
            let mut stmt = chat_conn
                .prepare("SELECT id, title, created_at, is_favorite FROM conversations")
                .map_err(|e| {
                    StoreError::StringError(format!("Failed to prepare conversations query: {}", e))
                })?;

            let conversations = stmt
                .query_map(params![], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, bool>(3)?,
                    ))
                })
                .map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_query_conversations", error = e.to_string()).to_string(),
                    )
                })?;

            let tx = new_conn.transaction()?;
            for conversation_result in conversations {
                let (conv_id, title, created_at, is_favorite) =
                    conversation_result.map_err(|e| {
                        StoreError::StringError(
                            t!("db.failed_to_read_conversation", error = e.to_string()).to_string(),
                        )
                    })?;

                tx.execute(
                    "INSERT INTO conversations (id, title, created_at, is_favorite) VALUES (?, ?, ?, ?)",
                    params![conv_id, title, created_at, is_favorite],
                ).map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_insert_conversation", error = e.to_string()).to_string(),
                    )
                })?;

                // Migrate messages for this conversation
                let mut msg_stmt = chat_conn.prepare(
                    "SELECT id, role, content, timestamp, metadata FROM messages WHERE conversation_id = ?"
                ).map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_prepare_messages_query", error = e.to_string()).to_string(),
                    )
                })?;

                let messages = msg_stmt
                    .query_map(params![conv_id], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    })
                    .map_err(|e| {
                        StoreError::StringError(
                            t!("db.failed_to_query_messages", error = e.to_string()).to_string(),
                        )
                    })?;

                for message_result in messages {
                    let (msg_id, role, content, timestamp, metadata) =
                        message_result.map_err(|e| {
                            StoreError::StringError(
                                t!("db.failed_to_read_message", error = e.to_string()).to_string(),
                            )
                        })?;

                    tx.execute(
                        "INSERT INTO messages (id, conversation_id, role, content, timestamp, metadata) VALUES (?, ?, ?, ?, ?, ?)",
                        params![msg_id, conv_id, role, content, timestamp, metadata.unwrap_or_default()],
                    ).map_err(|e| {
                        StoreError::StringError(
                            t!("db.failed_to_insert_message", error = e.to_string()).to_string(),
                        )
                    })?;
                }
            }
            tx.commit()?;

            // Explicitly close connections by dropping
            drop(stmt);
            drop(chat_conn);

            // Move chat.db to backup with timestamp
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let backup_path = backup_dir.join(format!("chat_{}.db.bak", timestamp));
            std::fs::rename(&chat_db_path, &backup_path).map_err(|e| {
                StoreError::StringError(
                    t!("db.failed_to_backup_chat_database", error = e.to_string()).to_string(),
                )
            })?;
            log::info!("Chat database migrated and backed up to {:?}", backup_path);
        }

        // Migrate config data if old config database exists
        if config_db_path.exists() {
            log::info!("Starting config database migration");
            let config_conn = Connection::open(&config_db_path).map_err(|e| {
                StoreError::StringError(format!("Failed to open config database: {}", e))
            })?;

            // Migrate config settings
            let mut stmt = config_conn
                .prepare("SELECT key, value FROM config")
                .map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_prepare_config_query", error = e.to_string()).to_string(),
                    )
                })?;

            let configs = stmt
                .query_map(params![], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_query_configs", error = e.to_string()).to_string(),
                    )
                })?;

            let tx = new_conn.transaction()?;
            for config_result in configs {
                let (key, value) = config_result.map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_read_config", error = e.to_string()).to_string(),
                    )
                })?;

                tx.execute(
                    "INSERT INTO config (key, value) VALUES (?, ?)",
                    params![key, value],
                )
                .map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_insert_config", error = e.to_string()).to_string(),
                    )
                })?;
            }

            // Migrate AI models
            let mut stmt = config_conn.prepare(
                "SELECT id, name, models, default_model, api_provider, base_url, api_key, max_tokens, \
                temperature, top_p, top_k, sort_index, is_default, disabled, is_official, official_id, metadata \
                FROM ai_model"
            ).map_err(|e| {
                StoreError::StringError(
                    t!("db.failed_to_prepare_ai_model_query", error = e.to_string()).to_string(),
                )
            })?;

            let models = stmt
                .query_map(params![], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i32>(7)?,
                        row.get::<_, f32>(8)?,
                        row.get::<_, f32>(9)?,
                        row.get::<_, i32>(10)?,
                        row.get::<_, i32>(11)?,
                        row.get::<_, bool>(12)?,
                        row.get::<_, bool>(13)?,
                        row.get::<_, bool>(14)?,
                        row.get::<_, String>(15)?,
                        row.get::<_, Option<String>>(16)?,
                    ))
                })
                .map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_query_ai_models", error = e.to_string()).to_string(),
                    )
                })?;

            for model in models {
                let (
                    id,
                    name,
                    models_str,
                    default_model,
                    api_provider,
                    base_url,
                    api_key,
                    max_tokens,
                    temperature,
                    top_p,
                    top_k,
                    sort_index,
                    is_default,
                    disabled,
                    is_official,
                    official_id,
                    metadata,
                ) = model.map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_read_ai_model", error = e.to_string()).to_string(),
                    )
                })?;

                tx.execute(
                    "INSERT INTO ai_model (id, name, models, default_model, api_provider, base_url, api_key, \
                    max_tokens, temperature, top_p, top_k, sort_index, is_default, disabled, is_official, official_id, metadata) \
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![id, name, models_str, default_model, api_provider, base_url, api_key,
                        max_tokens, temperature, top_p, top_k, sort_index, is_default, disabled,
                        is_official, official_id, metadata.unwrap_or_default()],
                ).map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_insert_ai_model", error = e.to_string()).to_string(),
                    )
                })?;
            }

            // Migrate AI skills
            let mut stmt = config_conn.prepare(
                "SELECT id, name, icon, logo, prompt, share_id, sort_index, disabled, metadata FROM ai_skill"
            ).map_err(|e| {
                StoreError::StringError(
                    t!("db.failed_to_prepare_ai_skill_query", error = e.to_string()).to_string(),
                )
            })?;

            let skills = stmt
                .query_map(params![], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i32>(6)?,
                        row.get::<_, bool>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                })
                .map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_query_ai_skills", error = e.to_string()).to_string(),
                    )
                })?;

            for skill in skills {
                let (id, name, icon, logo, prompt, share_id, sort_index, disabled, metadata) =
                    skill.map_err(|e| {
                        StoreError::StringError(
                            t!("db.failed_to_read_ai_skill", error = e.to_string()).to_string(),
                        )
                    })?;

                tx.execute(
                    "INSERT INTO ai_skill (id, name, icon, logo, prompt, share_id, sort_index, disabled, metadata) \
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![id, name, icon, logo, prompt, share_id.unwrap_or_default(), sort_index, disabled, metadata.unwrap_or_default()],
                ).map_err(|e| {
                    StoreError::StringError(
                        t!("db.failed_to_insert_ai_skill", error = e.to_string()).to_string(),
                    )
                })?;
            }

            tx.commit()?;

            // Explicitly close connections by dropping
            drop(stmt);

            // Move config.db to backup with timestamp
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let backup_path = backup_dir.join(format!("config_{}.db.bak", timestamp));
            std::fs::rename(&config_db_path, &backup_path).map_err(|e| {
                StoreError::StringError(
                    t!("db.failed_to_backup_config_database", error = e.to_string()).to_string(),
                )
            })?;
            log::info!(
                "Config database migrated and backed up to {:?}",
                backup_path
            );
        }

        log::info!("Database migration completed successfully");
        Ok(())
    }
}
