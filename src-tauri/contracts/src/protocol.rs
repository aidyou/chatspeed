use serde::{Deserialize, Serialize};

pub const PROTOCOL_MAJOR: u32 = 1;
pub const PROTOCOL_VERSION: &str = "1";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MetaResponse {
    pub service: String,
    pub protocol_version: String,
    pub schema_version: u32,
    pub server_instance_id: String,
    pub pid: u32,
}
