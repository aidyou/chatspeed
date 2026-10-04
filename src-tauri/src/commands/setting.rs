//! Settings, AI model/skill and backup Tauri commands.
//!
//! The configuration store, the AI model/skill tables and the database backup
//! maintenance are runtime-owned and reached through
//! `/control/v1/data-commands/*`. Desktop-only side effects (locale, tray,
//! shortcuts, the local static-file server) are applied in the wrappers, after a
//! successful runtime reply, and the local `httpServer` URL is merged back into
//! the configuration read.

use crate::constants::*;
use crate::db::{AiModel, AiSkill, ModelConfig};
use crate::error::{AppError, Result as AppResult};
use crate::libs::fs::{self, get_file_name};
use crate::runtime_client::RuntimeSupervisor;
use crate::runtime_data::{
    AddAiModelBody, AddAiSkillBody, RestoreSettingResponse, UpdateAiModelBody, UpdateAiSkillBody,
};
use crate::tray::create_tray;
use crate::workflow::react::idle_sleep::WORKFLOW_IDLE_SLEEP_INHIBITOR;

use rust_i18n::{set_locale, t};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tauri::{command, AppHandle, Manager, State};

// =================================================
// About Configuration
// =================================================

/// Returns the whole runtime configuration map plus the local `httpServer` url.
#[command]
pub async fn get_all_config(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<HashMap<String, Value>, String> {
    let value = crate::runtime_data::get_all_config(supervisor.inner().as_ref()).await?;
    let mut settings: HashMap<String, Value> =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    settings.insert(
        "httpServer".to_string(),
        Value::String(get_static_var(&HTTP_SERVER)),
    );
    Ok(settings)
}

/// Sets (or deletes) a configuration key, then applies local desktop effects.
#[command]
pub async fn set_config(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    key: &str,
    value: Value,
) -> Result<(), String> {
    let should_refresh_tray = crate::shortcut::is_shortcut_key(key);
    crate::runtime_data::set_config(supervisor.inner().as_ref(), key.to_string(), value.clone())
        .await?;

    // The runtime owns the configuration; mirror the just-written value into the
    // local cache so the synchronous readers (webview proxy, window geometry and
    // tray shortcut hints) observe it without waiting for the next async load. A
    // failed refresh is logged and never changes the successful write result.
    if let Some(cache) = app.try_state::<Arc<crate::runtime_config::RuntimeConfigCache>>() {
        if let Err(error) = cache.refresh(supervisor.inner().as_ref()).await {
            log::warn!(
                "Failed to refresh the runtime configuration cache after set_config: {error}"
            );
        }
    }

    match key {
        CFG_INTERFACE_LANGUAGE => {
            let lang =
                crate::libs::lang::normalize_interface_locale(value.as_str().unwrap_or_default());
            set_locale(lang);
            #[cfg(debug_assertions)]
            log::debug!("Language set to: {}", lang);
        }
        CFG_WORKFLOW_PREVENT_IDLE_SLEEP => {
            WORKFLOW_IDLE_SLEEP_INHIBITOR.set_enabled(value.as_bool().unwrap_or(false));
        }
        _ => {}
    }

    if should_refresh_tray {
        let _ = create_tray(&app, Some(TRAY_ID.to_string()));
    }

    Ok(())
}

/// Reloads the runtime configuration and mirrors it into the local cache.
#[command]
pub async fn reload_config(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<(), String> {
    crate::runtime_data::reload_config(supervisor.inner().as_ref()).await?;

    // Mirror the reloaded runtime configuration into the local cache so the
    // synchronous readers observe the latest values. The command exists to
    // propagate that configuration, so a failed refresh is surfaced instead of
    // being reported as success; the runtime's own error is never masked.
    if let Some(cache) = app.try_state::<Arc<crate::runtime_config::RuntimeConfigCache>>() {
        cache.refresh(supervisor.inner().as_ref()).await?;
    }
    Ok(())
}

/// Returns the API-key encryption status.
#[command]
pub async fn get_api_key_encryption_status(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    crate::runtime_data::get_api_key_encryption_status(supervisor.inner().as_ref()).await
}

/// Activates a client-chosen API-key file.
#[command]
pub async fn activate_api_key_file(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    path: String,
) -> Result<Value, String> {
    crate::runtime_data::activate_api_key_file(supervisor.inner().as_ref(), path).await
}

/// Generates and activates an API-key file at a client-chosen path.
#[command]
pub async fn generate_api_key_file(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    path: String,
) -> Result<Value, String> {
    crate::runtime_data::generate_api_key_file(supervisor.inner().as_ref(), path).await
}

// =================================================
// About AI Model
// =================================================

/// Returns an AI model by id.
#[command]
pub async fn get_ai_model_by_id(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<AiModel, String> {
    crate::runtime_data::get_ai_model_by_id(supervisor.inner().as_ref(), id).await
}

/// Returns all AI models.
#[command]
pub async fn get_all_ai_models(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<AiModel>, String> {
    crate::runtime_data::get_all_ai_models(supervisor.inner().as_ref()).await
}

/// Adds an AI model and returns the stored record.
#[command]
#[allow(clippy::too_many_arguments)]
pub async fn add_ai_model(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
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
) -> Result<AiModel, String> {
    crate::runtime_data::add_ai_model(
        supervisor.inner().as_ref(),
        AddAiModelBody {
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
        },
    )
    .await
}

/// Updates an AI model and returns the stored record.
#[command]
#[allow(clippy::too_many_arguments)]
pub async fn update_ai_model(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
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
) -> Result<AiModel, String> {
    crate::runtime_data::update_ai_model(
        supervisor.inner().as_ref(),
        UpdateAiModelBody {
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
        },
    )
    .await
}

/// Persists the AI model order.
#[command]
pub async fn update_ai_model_order(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    model_ids: Vec<i64>,
) -> Result<(), String> {
    crate::runtime_data::update_ai_model_order(supervisor.inner().as_ref(), model_ids).await
}

/// Deletes an AI model.
#[command]
pub async fn delete_ai_model(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<(), String> {
    crate::runtime_data::delete_ai_model(supervisor.inner().as_ref(), id).await
}

// =================================================
// About AI Skill
// =================================================

/// Returns an AI skill by id.
#[command]
pub async fn get_ai_skill_by_id(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<AiSkill, String> {
    crate::runtime_data::get_ai_skill_by_id(supervisor.inner().as_ref(), id).await
}

/// Returns all AI skills.
#[command]
pub async fn get_all_ai_skills(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<AiSkill>, String> {
    crate::runtime_data::get_all_ai_skills(supervisor.inner().as_ref()).await
}

/// Adds an AI skill; the desktop uploads the logo before the remote call.
#[command]
pub async fn add_ai_skill(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    name: String,
    icon: Option<String>,
    logo: Option<String>,
    prompt: String,
    disabled: bool,
    metadata: Option<Value>,
) -> Result<AiSkill, String> {
    let logo_url = match logo {
        Some(logo) => Some(upload_logo(logo)?),
        None => None,
    };
    crate::runtime_data::add_ai_skill(
        supervisor.inner().as_ref(),
        AddAiSkillBody {
            name,
            icon,
            logo: logo_url,
            prompt,
            disabled,
            metadata,
        },
    )
    .await
}

/// Updates an AI skill; the desktop uploads the logo before the remote call.
#[command]
pub async fn update_ai_skill(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    name: String,
    icon: Option<String>,
    logo: Option<String>,
    prompt: String,
    disabled: bool,
    metadata: Option<Value>,
) -> Result<AiSkill, String> {
    let logo_url = match logo {
        Some(logo) => Some(upload_logo(logo)?),
        None => None,
    };
    crate::runtime_data::update_ai_skill(
        supervisor.inner().as_ref(),
        UpdateAiSkillBody {
            id,
            name,
            icon,
            logo: logo_url,
            prompt,
            disabled,
            metadata,
        },
    )
    .await
}

/// Persists the AI skill order.
#[command]
pub async fn update_ai_skill_order(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    skill_ids: Vec<i64>,
) -> Result<(), String> {
    crate::runtime_data::update_ai_skill_order(supervisor.inner().as_ref(), skill_ids).await
}

/// Deletes an AI skill.
#[command]
pub async fn delete_ai_skill(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<(), String> {
    crate::runtime_data::delete_ai_skill(supervisor.inner().as_ref(), id).await
}

/// Updates the shortcut for the main window or assistant window.
#[tauri::command]
pub async fn update_shortcut(
    app: tauri::AppHandle,
    key: &str,
    value: Option<String>,
) -> AppResult<()> {
    let shortcut_value = value.unwrap_or_else(|| {
        crate::shortcut::get_default_shortcut(key)
            .unwrap_or_default()
            .to_string()
    });

    crate::shortcut::update_shortcut(&app, &shortcut_value, key).map_err(|e| {
        AppError::General {
            message: t!("setting.failed_to_update_shortcut", error = e.to_string()).to_string(),
        }
    })?;

    Ok(())
}

/// Uploads a logo image to the local static-file server.
///
/// This is a desktop capability: it moves the file the user picked into the
/// local upload directory (or saves a thumbnail) and returns the relative url
/// that the runtime then persists with the skill.
fn upload_logo(image_path: String) -> AppResult<String> {
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

/// Flushes and writes a full backup under the runtime-owned data directory.
#[tauri::command]
pub async fn backup_setting(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    backup_dir: Option<String>,
) -> Result<(), String> {
    crate::runtime_data::backup_setting(supervisor.inner().as_ref(), backup_dir).await
}

/// Restores a full backup.
#[tauri::command]
pub async fn restore_setting(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    backup_dir: String,
) -> Result<RestoreSettingResponse, String> {
    crate::runtime_data::restore_setting(supervisor.inner().as_ref(), backup_dir).await
}

/// Lists available backups.
#[tauri::command]
pub async fn get_all_backups(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    backup_dir: Option<String>,
) -> Result<Vec<String>, String> {
    crate::runtime_data::get_all_backups(supervisor.inner().as_ref(), backup_dir).await
}

#[tauri::command]
pub fn update_tray(app: AppHandle) -> AppResult<()> {
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
