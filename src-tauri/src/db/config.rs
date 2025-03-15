use super::types::AiSkill;
use crate::constants::{CFG_WINDOW_POSITION, HTTP_SERVER_DIR};
use crate::db::error::StoreError;
use crate::db::main_store::MainStore;
use crate::window::WindowSize;
use crate::MainWindowPosition;

use log::error;
use rusqlite::Result;
use serde_json::Value;
use std::path::Path;

impl MainStore {
    /// Sets a configuration item in the database.
    ///
    /// Inserts or replaces a configuration key-value pair. The value is stored in JSON format.
    ///
    /// # Arguments
    ///
    /// * `key` - The key of the configuration item.
    /// * `value` - The value of the configuration item as a `serde_json::Value`.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn set_config(&mut self, key: &str, value: &Value) -> Result<(), StoreError> {
        if let Err(e) = self.conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?, ?)",
            [key, &value.to_string()],
        ) {
            error!("Failed to set config for key '{}': {}", key, e);
            return Err(StoreError::from(e));
        }
        self.config.update_setting(key, value.clone());
        Ok(())
    }

    /// Adds a new AI model to the database.
    ///
    /// # Arguments
    /// * `name` - Name of the AI model
    /// * `models` - List of supported model names
    /// * `default_model` - Default model to use
    /// * `api_protocol` - API provider name
    /// * `base_url` - Base URL for API endpoint
    /// * `api_key` - API key for authentication
    /// * `max_tokens` - Maximum tokens allowed
    /// * `temperature` - Temperature parameter
    /// * `top_p` - Top P parameter
    /// * `top_k` - Top K parameter
    /// * `disabled` - Whether the model is disabled
    /// * `metadata` - Additional metadata as JSON
    ///
    /// # Returns
    /// The ID of the newly inserted AI model
    ///
    /// # Errors
    /// Returns a `StoreError` if the database operation fails
    pub fn add_ai_model(
        &mut self,
        name: String,
        models: Vec<String>,
        default_model: String,
        api_protocol: String,
        base_url: String,
        api_key: String,
        max_tokens: i32,
        temperature: f32,
        top_p: f32,
        top_k: i32,
        disabled: bool,
        metadata: Option<Value>,
    ) -> Result<i64, StoreError> {
        let max_sort_index: i32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(sort_index), -1) FROM ai_model",
                [],
                |row| row.get(0),
            )
            .map_err(|e| {
                error!("Failed to get max sort_index: {}", e);
                e
            })?;

        let new_sort_index = max_sort_index + 1;
        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| StoreError::TauriError(e.to_string()))?;

        self.conn
            .execute(
                "INSERT INTO ai_model (
                    name, models, default_model, api_protocol, base_url, api_key,
                    max_tokens, temperature, top_p, top_k, sort_index, disabled,
                    is_default, is_official, official_id, metadata
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    name,
                    models.join(","),
                    default_model,
                    api_protocol,
                    base_url,
                    api_key,
                    max_tokens,
                    temperature,
                    top_p,
                    top_k,
                    new_sort_index,
                    disabled,
                    false,
                    false,
                    "",
                    metadata_str,
                ),
            )
            .map_err(|e| {
                error!("Failed to add AI model: {}", e);
                e
            })?;

        if let Ok(models) = Self::get_all_ai_models(&self.conn) {
            self.config.set_ai_models(models);
        }
        Ok(self.conn.last_insert_rowid())
    }

    /// Updates an existing AI model in the database.
    ///
    /// Modifies the record corresponding to the given `AiModel` based on its ID.
    ///
    /// # Arguments
    ///
    /// * `model` - A reference to the `AiModel` with updated data.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn update_ai_model(
        &mut self,
        id: i64,
        name: String,
        models: Vec<String>,
        default_model: String,
        api_protocol: String,
        base_url: String,
        api_key: String,
        max_tokens: i32,
        temperature: f32,
        top_p: f32,
        top_k: i32,
        disabled: bool,
        metadata: Option<Value>,
    ) -> Result<(), StoreError> {
        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| StoreError::TauriError(e.to_string()))?;

        self.conn.execute(
            "UPDATE ai_model SET name = ?, models = ?, default_model = ?, api_protocol = ?,
             base_url = ?, api_key = ?, max_tokens = ?, temperature = ?, top_p = ?,
             top_k = ?, disabled = ?, metadata = ? WHERE id = ?",
            (
                name,
                models.join(","),
                default_model,
                api_protocol,
                base_url,
                api_key,
                max_tokens,
                temperature,
                top_p,
                top_k,
                disabled,
                metadata_str,
                id,
            ),
        )?;
        if let Ok(models) = Self::get_all_ai_models(&self.conn) {
            self.config.set_ai_models(models);
        }
        Ok(())
    }

    /// Updates the order of AI models in the database.
    ///
    /// Modifies the `sort_index` for each AI model based on the provided list of IDs.
    ///
    /// # Arguments
    ///
    /// * `models` - A vector of IDs representing the new order of AI models.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn update_ai_model_order(&mut self, model_ids: Vec<i64>) -> Result<(), StoreError> {
        for (index, id) in model_ids.iter().enumerate() {
            self.conn.execute(
                "UPDATE ai_model SET sort_index = ? WHERE id = ?",
                (index as i64, id),
            )?;
        }
        if let Ok(models) = Self::get_all_ai_models(&self.conn) {
            self.config.set_ai_models(models);
        }
        Ok(())
    }

    /// Deletes an AI model from the database.
    ///
    /// Removes the record with the specified ID from the `ai_model` table.
    ///
    /// # Arguments
    ///
    /// * `id` - The ID of the AI model to be deleted.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn delete_ai_model(&mut self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM ai_model WHERE id = ?", [id])?;
        if let Ok(models) = Self::get_all_ai_models(&self.conn) {
            self.config.set_ai_models(models);
        }
        Ok(())
    }

    /// Adds a new AI skill to the database.
    ///
    /// Inserts a new record into the `ai_skill` table and returns the generated ID.
    ///
    /// # Arguments
    /// * `name` - The name of the AI skill.
    /// * `icon` - The icon of the AI skill.
    /// * `logo` - The logo of the AI skill.
    /// * `prompt` - The prompt of the AI skill.
    /// * `disabled` - The disabled status of the AI skill.
    ///
    /// # Returns
    /// The ID of the newly inserted AI skill.
    ///
    /// # Errors
    /// Returns a `StoreError` if the database operation fails.
    pub fn add_ai_skill(
        &mut self,
        name: String,
        icon: Option<String>,
        logo: Option<String>,
        prompt: String,
        disabled: bool,
        metadata: Option<Value>,
    ) -> Result<AiSkill, StoreError> {
        // Get the current maximum sort_index
        let max_sort_index: i32 = self.conn.query_row(
            "SELECT COALESCE(MAX(sort_index), -1) FROM ai_skill",
            [],
            |row| row.get(0),
        )?;

        let new_sort_index = max_sort_index + 1;
        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| StoreError::TauriError(e.to_string()))?;

        self.conn.execute(
            "INSERT INTO ai_skill ( name, icon, logo, prompt, sort_index, disabled, metadata)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            (
                name,
                icon,
                logo,
                prompt,
                new_sort_index,
                disabled,
                metadata_str,
            ),
        )?;

        let last_id = self.conn.last_insert_rowid();
        if let Ok(skills) = Self::get_all_ai_skills(&self.conn) {
            self.config.set_ai_skills(skills);
        }

        self.config.get_ai_skill_by_id(last_id)
    }

    /// Updates an existing AI skill in the database.
    ///
    /// Modifies the record corresponding to the given `AiSkill` based on its ID.
    ///
    /// # Arguments
    /// * `id` - The ID of the AI skill to be updated.
    /// * `name` - The new name of the AI skill.
    /// * `icon - The new icon of the AI skill.
    /// * `logo` - The new logo of the AI skill.
    /// * `prompt` - The new prompt of the AI skill.
    /// * `disabled` - The new disabled status of the AI skill.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn update_ai_skill(
        &mut self,
        id: i64,
        name: String,
        icon: Option<String>,
        logo: Option<String>,
        prompt: String,
        disabled: bool,
        metadata: Option<Value>,
    ) -> Result<AiSkill, StoreError> {
        if let Ok(old_logo) = self.get_skill_logo(id) {
            if !old_logo.is_empty() {
                if logo.is_none() || logo.as_ref().map_or(true, |new_logo| new_logo != &old_logo) {
                    let http_server_dir = &*HTTP_SERVER_DIR.read();
                    let old_logo = old_logo.trim_start_matches('/');
                    let logo_path = Path::new(http_server_dir).join(old_logo);
                    #[cfg(debug_assertions)]
                    {
                        log::debug!("Updating logo - old logo: {}", old_logo);
                        log::debug!("Full logo path to delete: {:?}", logo_path);
                    }

                    if let Err(e) = std::fs::remove_file(&logo_path) {
                        log::error!("Failed to delete old logo image at {:?}: {}", logo_path, e);
                    } else {
                        log::debug!("Successfully deleted old logo at {:?}", logo_path);
                    }
                }
            }
        }

        let metadata_str = metadata
            .map(|m| serde_json::to_string(&m))
            .transpose()
            .map_err(|e| StoreError::TauriError(e.to_string()))?;

        self.conn.execute(
            "UPDATE ai_skill SET name = ?, icon = ?, logo = ?, prompt = ?, disabled = ?, metadata = ?
             WHERE id = ?",
            (name,icon, logo,  prompt, disabled, metadata_str, id),
        )?;
        if let Ok(skills) = Self::get_all_ai_skills(&self.conn) {
            self.config.set_ai_skills(skills);
        }
        self.config.get_ai_skill_by_id(id)
    }

    /// Updates the order of AI skills in the database.
    ///
    /// Modifies the `sort_index` for each AI skill based on the provided list of IDs.
    ///
    /// # Arguments
    ///
    /// * `skill_ids` - A vector of IDs representing the new order of AI skills.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn update_ai_skill_order(&mut self, skill_ids: Vec<i64>) -> Result<(), StoreError> {
        for (index, id) in skill_ids.iter().enumerate() {
            self.conn.execute(
                "UPDATE ai_skill SET sort_index = ? WHERE id = ?",
                (index as i64, id),
            )?;
        }
        if let Ok(skills) = Self::get_all_ai_skills(&self.conn) {
            self.config.set_ai_skills(skills);
        }
        Ok(())
    }

    /// Retrieves the logo of an AI skill from the database.
    ///
    /// # Arguments
    /// * `id` - The ID of the AI skill.
    ///
    /// # Returns
    /// The logo of the AI skill.
    pub fn get_skill_logo(&mut self, id: i64) -> Result<String, StoreError> {
        return self
            .conn
            .query_row("SELECT logo FROM ai_skill WHERE id = ?", [id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| StoreError::from(e));
    }

    /// Deletes an AI skill from the database.
    ///
    /// Removes the record with the specified ID from the `ai_skill` table.
    ///
    /// # Arguments
    /// * `id` - The ID of the AI skill to be deleted.
    ///
    /// # Errors
    /// Returns a `StoreError` if the database operation fails.
    pub fn delete_ai_skill(&mut self, id: i64) -> Result<(), StoreError> {
        // delete the logo image if exists
        if let Ok(logo) = self.get_skill_logo(id) {
            if !logo.is_empty() {
                let http_server_dir = &*HTTP_SERVER_DIR.read();
                let logo = logo.trim_start_matches('/');
                let logo_path = Path::new(http_server_dir).join(logo);
                #[cfg(debug_assertions)]
                {
                    log::debug!("Deleting logo: {}", logo);
                    log::debug!("Full logo path: {:?}", logo_path);
                }

                if let Err(e) = std::fs::remove_file(&logo_path) {
                    log::error!("Failed to delete logo image at {:?}: {}", logo_path, e);
                } else {
                    log::debug!("Successfully deleted logo at {:?}", logo_path);
                }
            }
        }

        self.conn
            .execute("DELETE FROM ai_skill WHERE id = ?", [id])?;
        if let Ok(skills) = Self::get_all_ai_skills(&self.conn) {
            self.config.set_ai_skills(skills);
        }
        Ok(())
    }

    /// Saves the window size to the configuration.
    ///
    /// # Arguments
    ///
    /// * `width` - The width of the window in pixels.
    /// * `size` - The size of the window.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn set_window_size(&mut self, size: WindowSize) -> Result<(), StoreError> {
        self.set_config(crate::constants::CFG_WINDOW_SIZE, &serde_json::json!(size))?;
        Ok(())
    }

    /// Saves the window position to the configuration.
    ///
    /// # Arguments
    /// * `window` - The window to save the position of.
    ///
    /// # Errors
    ///
    /// Returns a `StoreError` if the database operation fails.
    pub fn save_window_position(&mut self, pos: MainWindowPosition) -> Result<(), StoreError> {
        self.set_config(CFG_WINDOW_POSITION, &serde_json::json!(pos))?;
        Ok(())
    }

    /// Retrieves a configuration value of the specified type.
    ///
    /// # Type Parameters
    /// * `T`: The target type that implements Deserialize
    ///
    /// # Arguments
    /// * `key`: The configuration key to retrieve
    /// * `default`: The default value to return if the key doesn't exist or conversion fails
    ///
    /// # Returns
    /// The configuration value of type T, or the default value if not found
    pub fn get_config<T>(&self, key: &str, default: T) -> T
    where
        T: serde::de::DeserializeOwned + Default + std::fmt::Debug,
    {
        self.config
            .get_setting(key)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or(default)
    }
}
