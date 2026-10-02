use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const STREAM_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct StreamEnvelope {
    pub schema_version: u32,
    pub server_instance_id: String,
    pub sequence: u64,
    pub session_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResetRequired {
    pub schema_version: String,
    pub error: ResetError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResetError {
    pub code: String,
    pub reason: String,
}

impl ResetRequired {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            schema_version: "1".to_string(),
            error: ResetError {
                code: "reset_required".to_string(),
                reason: reason.into(),
            },
        }
    }
}
