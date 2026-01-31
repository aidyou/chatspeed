//!
//! This module contains Tauri commands for managing settings, AI models and skills
//! within the configuration store. It provides functionalities to get, set,
//! update, and delete AI models and skills, as well as to synchronize the
//! application state. The commands are designed to be invoked from the
//! frontend, allowing seamless interaction with the AI capabilities of the
//! application.
//!
//! ## Overview
//!
//! - **AI Models**: Functions to manage AI models, including adding, updating,
//!   deleting, and retrieving models.
//! - **AI Skills**: Functions to manage AI skills, including adding, updating,
//!   deleting, and retrieving skills.
//! - **Synchronization**: A command to sync the application state with the
//!   frontend.
//!
//! ## Usage
//!
//! The commands can be invoked from the frontend using Tauri's `invoke`
//! function. Each command is annotated with detailed documentation, including
//! parameters, return types, and examples of usage.
//!
//! ## Example
//!
//! ```js
//! // Call from frontend to get all AI models:
//! import { invoke } from '@tauri-apps/api/core'
//! const aiModels = await invoke('get_all_ai_models');
//! console.log(aiModels);
//! ```
//!

use crate::constants::*;
use crate::db::{AiModel, AiSkill, MainStore, ModelConfig};
use crate::db::{BackupConfig, DbBackup};
use crate::libs::fs::{self, get_file_name};
use crate::tray::create_tray;

use rust_i18n::{set_locale, t};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tauri::State;
use tauri::{command, AppHandle};

use crate::error::{AppError, Result};

// =================================================
// About Configuration
// =================================================

/// Get the configuration information
///
/// This function is used to get the configuration information from the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
///
/// # Returns
/// * `Result<Value, String>` - Returns the configuration as a JSON value or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const config = await invoke('get_all_config');
/// console.log(config);
/// ```
#[command]
pub fn get_all_config(state: State<Arc<RwLock<MainStore>>>) -> Result<HashMap<String, Value>> {
    let config_store = state.read()?;
    let mut settings = config_store.config.settings.clone();
    settings.insert(
        "httpServer".to_string(),
        Value::String(get_static_var(&HTTP_SERVER)),
    );

    // show main window shortcut
    if config_store
        .config
        .get_setting(CFG_MAIN_WINDOW_VISIBLE_SHORTCUT)
        .is_none()
    {
        settings.insert(
            CFG_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string(),
            Value::String(DEFAULT_MAIN_WINDOW_VISIBLE_SHORTCUT.to_string()),
        );
    }
    // toggle assistant window visible shortcut
    if config_store
        .config
        .get_setting(CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT)
        .is_none()
    {
        settings.insert(
            CFG_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string(),
            Value::String(DEFAULT_ASSISTANT_WINDOW_VISIBLE_SHORTCUT.to_string()),
        );
    }
    Ok(settings)
}

/// Set the configuration information
///
/// This function is used to set the configuration information in the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `key` - The key of the configuration item to set
/// - `value` - The value of the configuration item (in JSON format)
///
/// # Returns
/// * `Result<(), String>` - Returns Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('set_config', { key: 'theme', value: 'dark' });
/// ```
#[command]
pub fn set_config(state: State<Arc<RwLock<MainStore>>>, key: &str, value: Value) -> Result<()> {
    let mut config_store = state.write()?;

    // Get previous value
    // let prev_value = config_store.get_config(key, Value::Null);

    // Set the configuration value
    match config_store.set_config(key, &value).map_err(AppError::Db) {
        Ok(_) => {
            match key {
                CFG_INTERFACE_LANGUAGE => {
                    let lang =
                        config_store.get_config::<String>(CFG_INTERFACE_LANGUAGE, "en".to_string());
                    set_locale(&lang);
                    #[cfg(debug_assertions)]
                    log::debug!("Language set to: {}", lang);
                }
                _ => {}
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Reload the configuration from the database
#[command]
pub fn reload_config(state: State<Arc<RwLock<MainStore>>>) -> Result<()> {
    let mut config_store = state.write()?;
    config_store.reload_config().map_err(AppError::Db)
}

// =================================================
// About AI Model
// =================================================

/// Get an AI model by its ID
///
/// Retrieves an AI model by its ID from the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `id` - The ID of the AI model to retrieve
///
/// # Returns
/// * `Result<AiModel, String>` - The AI model or an error message
#[command]
pub fn get_ai_model_by_id(state: State<Arc<RwLock<MainStore>>>, id: i64) -> Result<AiModel> {
    let config_store = state.read()?;
    config_store
        .config
        .get_ai_model_by_id(id)
        .map_err(AppError::Db)
}

/// Get all AI models
///
/// Retrieves a list of all AI models from the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
///
/// # Returns
/// * `Result<Vec<AiModel>, String>` - A vector of AI models or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const aiModels = await invoke('get_all_ai_models');
/// console.log(aiModels);
/// ```
#[command]
pub fn get_all_ai_models(state: State<Arc<RwLock<MainStore>>>) -> Result<Vec<AiModel>> {
    let config_store = state.read()?;
    Ok(config_store.config.get_ai_models())
}

/// Add a new AI model
///
/// Adds a new AI model to the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `name` - The name of the AI model to add
/// - `models` - A vector of model names associated with the new AI model
/// - `default_model` - The name of the default model to be used
/// - `base_url` - The base URL for the AI model's API
/// - `api_key` - The API key for accessing the AI model
/// - `disabled` - A boolean indicating whether the model is disabled
///
/// # Returns
/// * `Result<AiModel, String>` - The AI model or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// const newModelId = await invoke('add_ai_model', {
///     name: 'GPT-4',
///     models: ['gpt-4'],
///     defaultModel: 'gpt-4',
///     baseUrl: 'https://api.example.com',
///     apiKey: 'your_api_key',
///     maxTokens: 4096,
///     temperature: 1.0,
///     topP: 1.0,
///     topK: 40,
///     disabled: false
/// });
/// console.log(`Added AI Model with ID: ${newModelId}`);
/// ```
#[command]
pub fn add_ai_model(
    state: State<Arc<RwLock<MainStore>>>,
    name: String,
    models: Vec<ModelConfig>,
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
) -> Result<AiModel> {
    let mut config_store = state.write()?;

    // First add the model to get the ID
    let id = config_store
        .add_ai_model(
            name,
            models,
            default_model,
            api_protocol,
            base_url,
            api_key,
            max_tokens,
            temperature,
            top_p,
            top_k,
            disabled,
            metadata,
        )
        .map_err(AppError::Db)?;

    // Return the newly created model data
    config_store
        .config
        .get_ai_model_by_id(id)
        .map_err(AppError::Db)
}

/// Update an existing AI model
///
/// Updates the details of an existing AI model in the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `id` - The ID of the AI model to update
/// - `name` - The new name for the AI model
/// - `models` - A vector of model names associated with the AI model
/// - `default_model` - The name of the new default model to be used
/// - `base_url` - The new base URL for the AI model's API
/// - `api_key` - The new API key for accessing the AI model
/// - `max_tokens` - The new max tokens for the AI model
/// - `temperature` - The new temperature for the AI model
/// - `top_p` - The new top p for the AI model
/// - `top_k` - The new top k for the AI model
/// - `disabled` - A boolean indicating whether the model should be disabled
/// - `metadata` - The new metadata for the AI model
///
/// # Returns
/// * `Result<AiModel, String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('update_ai_model', {
///     id: 1,
///     name: 'GPT-4 Updated',
///     models: ['gpt-4'],
///     defaultModel: 'gpt-4',
///     baseUrl: 'https://api.example.com',
///     apiKey: 'your_new_api_key',
///     maxTokens: 4096,
///     temperature: 1.0,
///     topP: 1.0,
///     topK: 40,
///     disabled: false
/// });
/// console.log('AI Model updated successfully');
/// ```
#[command]
pub fn update_ai_model(
    state: State<Arc<RwLock<MainStore>>>,
    id: i64,
    name: String,
    models: Vec<ModelConfig>,
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
) -> Result<AiModel> {
    let mut config_store = state.write()?;

    config_store
        .update_ai_model(
            id,
            name,
            models,
            default_model,
            api_protocol,
            base_url,
            api_key,
            max_tokens,
            temperature,
            top_p,
            top_k,
            disabled,
            metadata,
        )
        .map_err(AppError::Db)?;

    config_store
        .config
        .get_ai_model_by_id(id)
        .map_err(AppError::Db)
}

/// Update the order of AI models
///
/// Updates the order of AI models in the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `model_ids` - A vector of IDs representing the new order of AI models
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('update_model_order', { modelIds: [1, 2, 3] });
/// console.log('AI Model order updated successfully');
#[command]
pub fn update_ai_model_order(
    state: State<Arc<RwLock<MainStore>>>,
    model_ids: Vec<i64>,
) -> Result<()> {
    let mut config_store = state.write()?;
    config_store
        .update_ai_model_order(model_ids)
        .map_err(AppError::Db)
}

/// Delete an AI model
///
/// Removes an AI model from the configuration store by its ID.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `id` - The ID of the AI model to delete
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('delete_ai_model', { id: 1 });
/// console.log('AI Model deleted successfully');
/// ```
#[command]
pub fn delete_ai_model(state: State<Arc<RwLock<MainStore>>>, id: i64) -> Result<()> {
    let mut config_store = state.write()?;
    config_store.delete_ai_model(id).map_err(AppError::Db)
}

// =================================================
// About AI Skill
// =================================================

/// Get an AI skill by its ID
///
/// Retrieves an AI skill by its ID from the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `id` - The ID of the AI skill to retrieve
///
/// # Returns
/// * `Result<AiSkill, String>` - The AI skill or an error message
#[command]
pub fn get_ai_skill_by_id(state: State<Arc<RwLock<MainStore>>>, id: i64) -> Result<AiSkill> {
    let config_store = state.read()?;
    config_store
        .config
        .get_ai_skill_by_id(id)
        .map_err(AppError::Db)
}

/// Get all AI skills
///
/// Retrieves a list of all AI skills from the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
///
/// # Returns
/// * `Result<Vec<AiSkill>, String>` - A vector of AI skills or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const aiSkills = await invoke('get_all_ai_skills');
/// console.log(aiSkills);
/// ```
#[command]
pub fn get_all_ai_skills(state: State<Arc<RwLock<MainStore>>>) -> Result<Vec<AiSkill>> {
    let config_store = state.read()?;
    Ok(config_store.config.get_ai_skills())
}

/// Add a new AI skill
///
/// Adds a new AI skill to the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `skill` - The AI skill to add
///
/// # Returns
/// * `Result<i64, String>` - The ID of the added skill or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// const newSkillId = await invoke('add_ai_skill', {  name: 'Natural Language Processing', prompt: 'This is a test prompt', icon: 'write', disabled: false });
/// console.log(`Added AI Skill with ID: ${newSkillId}`);
/// ```
#[command]
pub fn add_ai_skill(
    state: State<Arc<RwLock<MainStore>>>,
    name: String,
    icon: Option<String>,
    logo: Option<String>,
    prompt: String,
    disabled: bool,
    metadata: Option<Value>,
) -> Result<AiSkill> {
    let mut config_store = state.write()?;

    let logo_url = if let Some(logo) = logo {
        upload_logo(logo)?
    } else {
        "".to_string()
    };

    config_store
        .add_ai_skill(name, icon, Some(logo_url), prompt, disabled, metadata)
        .map_err(AppError::Db)
}

/// Update an existing AI skill
///
/// Updates the details of an existing AI skill in the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `skill` - The AI skill with updated information
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('update_ai_skill', { skill: { id: 1, name: 'Machine Learning', ... } });
/// console.log('AI Skill updated successfully');
/// ```
#[command]
pub fn update_ai_skill(
    state: State<Arc<RwLock<MainStore>>>,
    id: i64,
    name: String,
    icon: Option<String>,
    logo: Option<String>,
    prompt: String,
    disabled: bool,
    metadata: Option<Value>,
) -> Result<AiSkill> {
    let mut config_store = state.write()?;

    let logo_url = if let Some(logo) = logo {
        upload_logo(logo)?
    } else {
        "".to_string()
    };

    config_store
        .update_ai_skill(id, name, icon, Some(logo_url), prompt, disabled, metadata)
        .map_err(AppError::Db)
}

/// Update the order of AI skills
///
/// Updates the order of AI skills in the configuration store.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `skill_ids` - A vector of IDs representing the new order of AI skills
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
#[command]
pub fn update_ai_skill_order(
    state: State<Arc<RwLock<MainStore>>>,
    skill_ids: Vec<i64>,
) -> Result<()> {
    let mut config_store = state.write()?;
    config_store
        .update_ai_skill_order(skill_ids)
        .map_err(AppError::Db)
}

/// Delete an AI skill
///
/// Removes an AI skill from the configuration store by its ID.
///
/// # Arguments
/// - `state` - The state of the configuration store, automatically injected by Tauri
/// - `id` - The ID of the AI skill to delete
///
/// # Returns
/// * `Result<(), String>` - Ok if successful or an error message
///
/// # Example
///
/// ```js
/// // Call from frontend:
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('delete_ai_skill', { id: 1 });
/// console.log('AI Skill deleted successfully');
/// ```
#[command]
pub fn delete_ai_skill(state: State<Arc<RwLock<MainStore>>>, id: i64) -> Result<()> {
    let mut config_store = state.write()?;
    config_store.delete_ai_skill(id).map_err(AppError::Db)
}

/// Update the shortcut
///
/// Updates the shortcut for the main window or assistant window.
#[tauri::command]
pub async fn update_shortcut(app: tauri::AppHandle, key: &str, value: &str) -> Result<()> {
    crate::shortcut::update_shortcut(&app, value, key).map_err(|e| AppError::General {
        message: t!("setting.failed_to_update_shortcut", error = e.to_string()).to_string(),
    })
}

/// Uploads a logo image to the server.
///
/// This function takes the path of an image file, checks if a preview image exists in the temporary directory,
/// and either moves it to the upload directory or saves a new thumbnail image. The function organizes the
/// uploaded images by month.
///
/// # Arguments
/// - `image_path`: The path of the image file to upload.
///
/// # Returns
/// * `Result<String, String>` - Returns the relative path of the uploaded image or an error message.
fn upload_logo(image_path: String) -> Result<String> {
    if image_path == "" {
        return Ok("".to_string());
    }
    // if image_path contains upload_dir, it means the image is already uploaded
    if image_path.contains("/upload") {
        return Ok(image_path);
    }

    let file = Path::new(&image_path);
    if !file.exists() {
        return Err(AppError::General {
            message: t!("setting.file_not_exists", file_path = image_path).to_string(),
        });
    }

    // Save file by month
    let month = chrono::Local::now().format("%Y%m").to_string();
    let upload_dir = HTTP_SERVER_UPLOAD_DIR.read().clone();
    let upload_file_dir = Path::new(&upload_dir).join(month);
    std::fs::create_dir_all(&upload_file_dir).map_err(|e| AppError::General {
        message: t!(
            "setting.failed_to_create_upload_dir",
            path = upload_file_dir.display(),
            error = e.to_string()
        )
        .to_string(),
    })?;

    let http_server_dir = HTTP_SERVER_DIR.read().clone();
    // Check if the file is in the static/tmp directory
    let http_server_tmp_dir = HTTP_SERVER_TMP_DIR.read().clone();

    let save_name = get_file_name(&file);

    // When the user selects an image, the system automatically creates a preview image
    // If the preview image exists, move it to the upload directory
    let tmp_file_path = Path::new(&http_server_tmp_dir).join(&save_name);
    if tmp_file_path.exists() {
        let upload_file_path = upload_file_dir.join(&save_name);
        // Move the temporary file to upload directory
        std::fs::rename(&tmp_file_path, &upload_file_path).map_err(|e| AppError::General {
            message: t!(
                "setting.failed_to_move_logo_file",
                from = tmp_file_path.display(),
                to = upload_file_path.display(),
                error = e.to_string()
            )
            .to_string(),
        })?;
        return Ok(upload_file_path
            .to_string_lossy()
            .to_string()
            .replace(&http_server_dir, ""));
    }

    // If the preview image does not exist, save the image to the upload directory
    let save_path = fs::save_thumbnail_image(
        file,
        &upload_file_dir,
        Some(DEFAULT_THUMBNAIL_WIDTH),
        Some(DEFAULT_THUMBNAIL_HEIGHT),
    )
    .map_err(|e| AppError::General {
        message: t!(
            "setting.failed_to_save_logo_thumbnail",
            error = e.to_string()
        )
        .to_string(),
    })?;

    Ok(save_path
        .to_string_lossy()
        .to_string()
        .replace(&*http_server_dir, ""))
}

// =================================================
// Backup
// =================================================
#[tauri::command]
pub async fn backup_setting(
    app: AppHandle,
    backup_dir: Option<String>,
) -> Result<()> {
    let result = tokio::spawn(async move {
        DbBackup::new(
            &app,
            BackupConfig {
                backup_dir,
            },
        )
        .and_then(|mut backup| backup.backup_to_directory())
    })
    .await
    .map_err(|e| AppError::General {
        message: e.to_string(),
    })?;

    result.map_err(AppError::Db)
}

#[tauri::command]
pub async fn restore_setting(
    app: AppHandle,
    state: State<'_, Arc<RwLock<MainStore>>>,
    backup_dir: String,
) -> Result<()> {
    // 1. Define configuration keys that are machine-specific and should be preserved
    let machine_specific_keys = [
        "backupDir",
        CFG_WINDOW_POSITION,
        CFG_WINDOW_SIZE,
        CFG_ASSISTANT_WINDOW_SIZE,
        CFG_WORKFLOW_WINDOW_SIZE,
        CFG_WORKFLOW_WINDOW_POSITION,
        "proxyType",
        "proxyServer",
        "proxyUsername",
        "proxyPassword",
    ];

    // 2. Backup current machine-specific configurations
    let mut preserved_configs = HashMap::new();
    {
        let config_store = state.read()?;
        for &key in &machine_specific_keys {
            if let Some(value) = config_store.config.get_setting(key) {
                preserved_configs.insert(key.to_string(), value.clone());
            }
        }
    }

    let result = tokio::spawn(async move {
        let theme_dir = HTTP_SERVER_THEME_DIR.read().clone();
        let upload_dir = HTTP_SERVER_UPLOAD_DIR.read().clone();
        let schema_dir = SCHEMA_DIR.read().clone();
        let shared_dir = SHARED_DATA_DIR.read().clone();
        let static_dir = HTTP_SERVER_DIR.read().clone();
        let store_dir = STORE_DIR.read().clone();
        let mcp_sessions_dir = store_dir.join("mcp_sessions");
        DbBackup::new(
            &app,
            BackupConfig {
                backup_dir: Some(backup_dir.clone()),
            },
        )
        .and_then(|db_backup| {
            db_backup.restore_from_directory(
                &Path::new(&backup_dir),
                &Path::new(&*theme_dir),
                &Path::new(&*upload_dir),
                &Path::new(&mcp_sessions_dir),
                &Path::new(&*schema_dir),
                &Path::new(&*shared_dir),
                &Path::new(&*static_dir),
            )
        })
    })
    .await
    .map_err(|e| AppError::General {
        message: e.to_string(),
    })?;

    result.map_err(AppError::Db)?;

    // 3. Reload the configuration from the newly restored database file
    let mut config_store = state.write()?;
    config_store.reload_config().map_err(AppError::Db)?;

    // 4. Restore the preserved configurations to the new database
    for key in machine_specific_keys {
        if let Some(value) = preserved_configs.get(key) {
            config_store.set_config(key, value).map_err(AppError::Db)?;
        } else {
            // If the key didn't exist before, ensure it doesn't exist in the restored DB either
            let _ = config_store.delete_config(key);
        }
    }

    Ok(())
}

#[tauri::command]
pub fn get_all_backups(app: AppHandle, backup_dir: Option<String>) -> Result<Vec<String>> {
    let db_backup = DbBackup::new(
        &app,
        BackupConfig {
            backup_dir,
        },
    )?;

    let backups = db_backup.list_backups().map_err(AppError::Db)?;
    Ok(backups
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect())
}

#[tauri::command]
pub fn update_tray(app: AppHandle) -> Result<()> {
    #[cfg(debug_assertions)]
    log::debug!("update_tray");

    create_tray(&app, Some(TRAY_ID.to_string())).map_err(|e| AppError::General {
        message: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_logo() {
        assert_eq!(upload_logo("".to_string()).is_ok(), true);
        assert_eq!(
            upload_logo("/static/upload/202410/test.png".to_string()).unwrap(),
            "/static/upload/202410/test.png".to_string()
        );
        assert_eq!(
            upload_logo("/a/b/c/tmp/test.png".to_string()).is_ok(),
            false
        );
    }
}
