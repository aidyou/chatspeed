use serde::{Deserialize, Serialize};

pub const DISCOVERY_FILE_NAME: &str = "control-plane-v1.json";
pub const CONTROL_PLANE_HOST: &str = "127.0.0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ControlPlaneDiscovery {
    pub protocol_version: String,
    pub server_instance_id: String,
    pub pid: u32,
    pub host: String,
    pub port: u16,
    pub token: String,
    pub started_at: String,
}
