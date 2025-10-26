use crate::db::{error::StoreError, ProxyGroup};

use log::error;
use rusqlite::{Connection, Result};

use rust_i18n::t;
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::Mutex};

use super::{
    mcp::Mcp,
    sql::migrations::manager,
    types::{Config, ModelConfig},
    AiModel, AiSkill,
};

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
            .ok_or_else(|| {
                StoreError::NotFound(t!("db.ai_model_not_found_by_id", id = id).to_string())
            })
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
            .ok_or_else(|| {
                StoreError::NotFound(t!("db.ai_skill_not_found_by_id", id = id).to_string())
            })
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

    /// Retrieves a thread-safe clone of the MCP configurations.
    ///
    /// # Returns
    ///
    /// Returns an `Arc<Vec<ModelConfig>>` containing the MCP configurations.
    pub fn get_mcps(&self) -> Vec<Mcp> {
        self.mcps.clone()
    }

    /// Retrieves a MCP server by its ID
    ///
    /// # Arguments
    ///     * `id` - The ID of the MCP server to retrieve.
    ///
    /// # Return
    ///     - MCP server config
    pub fn get_mcp_by_id(&self, id: i64) -> Result<Mcp, StoreError> {
        self.mcps
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(t!("db.mcp_not_found_by_id", id = id).to_string()))
    }

    /// Sets the MCP configurations in the configuration.
    ///
    /// # Arguments
    /// * `mcps` - A vector of `ModelConfig` instances to set.
    pub fn set_mcps(&mut self, mcps: Vec<Mcp>) {
        self.mcps = mcps;
    }

    pub fn get_proxy_groups(&self) -> Vec<ProxyGroup> {
        self.proxy_groups.clone()
    }

    pub fn get_proxy_group_by_name(&self, name: &str) -> Result<ProxyGroup, StoreError> {
        if name.is_empty() {
            return Ok(ProxyGroup {
                name: "default".to_string(),
                temperature: Some(1.0),
                ..Default::default()
            });
        }
        self.proxy_groups
            .iter()
            .find(|p| p.name == name)
            .cloned()
            .ok_or_else(|| {
                StoreError::NotFound(t!("proxy.group.not_found_by_name", name = name).to_string())
            })
    }

    pub fn set_proxy_groups(&mut self, proxy_groups: Vec<ProxyGroup>) {
        self.proxy_groups = proxy_groups;
    }
}

/// Manages unified storage for the application, including chat history and configuration.
pub struct MainStore {
    pub(crate) conn: Mutex<Connection>,
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
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, StoreError> {
        let mut conn = Connection::open(&db_path).map_err(|e| {
            let err = t!("db.failed_to_open_db_connection", error = e.to_string()).to_string();
            log::error!("{}", err);
            StoreError::Query(err)
        })?;

        Self::init_db(&mut conn).map_err(|e| {
            let err = t!("db.failed_to_initialize_database", error = e.to_string()).to_string();
            log::error!("{}", err);
            StoreError::Query(err)
        })?;

        let db_dir = db_path.as_ref().parent().unwrap_or(db_path.as_ref());
        Self::migrate_data(&db_dir).map_err(|e| {
            let err = t!("db.failed_to_migrate_database", error = e.to_string()).to_string();
            log::error!("{}", err);
            StoreError::Query(err)
        })?;

        let conn = Mutex::new(conn);
        let config = {
            let locked_conn = conn
                .lock()
                .map_err(|e| StoreError::LockError(e.to_string()))?;
            Self::load_config(&locked_conn)?
        };

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
        let mcps = Self::get_all_mcps(conn)?;
        let proxy_groups = Self::proxy_group_list(conn)?;

        Ok(Config {
            settings,
            ai_models,
            ai_skills,
            mcps,
            proxy_groups,
        })
    }

    /// Reloads the configuration from the database.
    ///
    /// # Returns
    /// Returns a `Result` containing `()` if successful, or a `StoreError` if an error occurs.
    pub fn reload_config(&mut self) -> Result<(), StoreError> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| StoreError::LockError(e.to_string()))?;
        match Self::load_config(&conn) {
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
                let value: Value = serde_json::from_str(&value_str).unwrap_or_else(|e| {
                    error!(
                        "Failed to parse JSON for config key '{}': {}. Value: '{}'",
                        key, e, value_str
                    );
                    Value::Null
                });
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
                let metadata_str: Option<String> = row.get("metadata")?; // metadata is JSON string
                let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).map_err(|e| {
                    log::warn!("Failed to parse metadata JSON for AI Model (id: {:?}): {}, error: {}", row.get::<_, Option<i64>>("id").unwrap_or_default(), s, e);
                    e
                }).ok());
                // try to JSON parse models
                let models_str = row.get::<_, String>("models");
                let models = models_str
                    .and_then(|s| match serde_json::from_str::<Vec<ModelConfig>>(&s) {
                        Ok(models) => Ok(models),
                        Err(e) => {
                            // Check if it's a syntax error,
                            log::warn!("Failed to parse 'models' field for AI Model (id: {:?}) as JSON array: {}. Falling back to comma-separated. Error: {}", row.get::<_, Option<i64>>("id").unwrap_or_default(), s, e);
                            // which might be an old format(comma-separated)
                            if e.is_syntax() || e.is_data() {
                                // Handle old format: comma-separated, trim spaces
                                Ok(s.split(',')
                                    .map(|part| part.trim())
                                    .filter(|part| !part.is_empty())
                                    .map(|part| ModelConfig {
                                        id: part.to_string(),
                                        name: part.to_string(),
                                        ..ModelConfig::default()
                                    })
                                    .collect())
                            } else {
                                // Other errors (like IO errors, theoretically won't happen), return default value or error
                                error!("Unexpected error parsing 'models' field for AI Model (id: {:?}): {}. Error: {}", row.get::<_, Option<i64>>("id").unwrap_or_default(), s, e);
                                Err(rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(StoreError::JsonError(t!("db.json_parse_failed_models", error = e.to_string()).to_string()))))
                            }
                        }
                    })
                    .unwrap_or_default();

                Ok(AiModel {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    models,
                    default_model: row.get("default_model").unwrap_or_default(),
                    api_protocol: row.get("api_protocol").unwrap_or_default(),
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
            let metadata_str: Option<String> = row.get("metadata")?; // metadata is JSON string
            let metadata = metadata_str.and_then(|s| {
                serde_json::from_str(&s)
                    .map_err(|e| {
                        log::warn!(
                            "Failed to parse metadata JSON for AI Skill (id: {:?}): {}, error: {}",
                            row.get::<_, Option<i64>>("id").unwrap_or_default(),
                            s,
                            e
                        );
                        e
                    })
                    .ok()
            });

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

    /// Checks if the unified database exists and creates it if necessary.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the database directory
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if any database operation fails.
    pub fn migrate_data<P: AsRef<Path>>(db_path: P) -> Result<(), StoreError> {
        let new_db_path = db_path.as_ref().join("chatspeed.db");

        // Create new database and initialize tables if it doesn't exist
        if !new_db_path.exists() {
            let mut new_conn = Connection::open(&new_db_path).map_err(|e| {
                StoreError::Query(
                    t!(
                        "db.failed_to_create_new_database_at",
                        path = new_db_path.display(),
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;
            Self::init_db(&mut new_conn)?;
            log::info!("Created new unified database: {:?}", new_db_path);
        } else {
            log::info!("Unified database already exists: {:?}", new_db_path);
        }

        Ok(())
    }
}
