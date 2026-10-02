//! Proxy-group Tauri commands.
//!
//! Proxy groups and the active-group setting are owned by the runtime store.
//! These wrappers only translate the Tauri wire into
//! `/control/v1/data-commands/*` calls through the [`RuntimeSupervisor`].

use std::sync::Arc;

use serde_json::Value;
use tauri::{command, State};

use crate::db::ProxyGroup;
use crate::runtime_client::RuntimeSupervisor;
use crate::runtime_data::ProxyGroupBatchUpdateBody;

/// Lists all proxy groups.
#[command]
pub async fn proxy_group_list(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<ProxyGroup>, String> {
    crate::runtime_data::proxy_group_list(supervisor.inner().as_ref()).await
}

/// Adds a proxy group and returns its id.
#[command]
pub async fn proxy_group_add(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    item: ProxyGroup,
) -> Result<i64, String> {
    crate::runtime_data::proxy_group_add(supervisor.inner().as_ref(), item).await
}

/// Updates a proxy group.
#[command]
pub async fn proxy_group_update(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    item: ProxyGroup,
) -> Result<(), String> {
    crate::runtime_data::proxy_group_update(supervisor.inner().as_ref(), item).await
}

/// Applies prompt-injection edits to many proxy groups.
#[command]
#[allow(clippy::too_many_arguments)]
pub async fn proxy_group_batch_update(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    ids: Vec<i64>,
    prompt_injection: Option<String>,
    prompt_text: Option<String>,
    tool_filter: Option<String>,
    injection_position: Option<String>,
    injection_condition: Option<String>,
    prompt_replace: Option<Value>,
) -> Result<(), String> {
    crate::runtime_data::proxy_group_batch_update(
        supervisor.inner().as_ref(),
        ProxyGroupBatchUpdateBody {
            ids,
            prompt_injection,
            prompt_text,
            tool_filter,
            injection_position,
            injection_condition,
            prompt_replace,
        },
    )
    .await
}

/// Deletes a proxy group.
#[command]
pub async fn proxy_group_delete(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<(), String> {
    crate::runtime_data::proxy_group_delete(supervisor.inner().as_ref(), id).await
}

/// Sets the active proxy group name.
#[command]
pub async fn set_active_proxy_group(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    name: String,
) -> Result<(), String> {
    crate::runtime_data::set_active_proxy_group(supervisor.inner().as_ref(), name).await
}

/// Returns the active proxy group name.
#[command]
pub async fn get_active_proxy_group(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<String, String> {
    crate::runtime_data::get_active_proxy_group(supervisor.inner().as_ref()).await
}
