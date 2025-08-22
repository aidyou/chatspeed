use std::sync::Arc;

use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{ccproxy::errors::ProxyResult, db::MainStore};

#[derive(Debug, Deserialize)]
pub struct ShowRequest {
    model: String,
}

/// Handles the `/api/show` request for ollama.
pub async fn handle_ollama_show(
    _main_store: Arc<std::sync::RwLock<MainStore>>,
    Json(payload): Json<ShowRequest>,
) -> ProxyResult<Json<Value>> {
    let _model_name = payload.model;
    let response = json!({
        "modelfile": "",
        "parameters": "",
        "template": "",
        "details": {
            "parent_model": "",
            "format": "gguf",
            "family": "",
            "families": null,
            "parameter_size": "",
            "quantization_level": ""
        }
    });

    Ok(Json(response))
}
