//! Persisted budgets and typed resource-protection outcomes.

use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const CONFIG_KEY: &str = "agent_resource_limits";

pub struct TaskGuard {
    pub(super) controller: Arc<super::controller::Controller>,
    pub(super) pid: u32,
    pub(super) failure: Arc<parking_lot::Mutex<Option<ResourceViolation>>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceLimits {
    pub enabled: bool,
    pub total_memory_mb: u32,
    pub max_concurrent_tasks: u32,
    pub task_memory_mb: u32,
    pub task_max_processes: u32,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            enabled: true,
            total_memory_mb: 8192,
            max_concurrent_tasks: 4,
            task_memory_mb: 4096,
            task_max_processes: 128,
        }
    }
}

impl ResourceLimits {
    pub fn validate(&self) -> Result<(), ResourceViolation> {
        if !(128..=1_048_576).contains(&self.total_memory_mb)
            || !(128..=self.total_memory_mb).contains(&self.task_memory_mb)
            || !(1..=64).contains(&self.max_concurrent_tasks)
            || !(1..=4096).contains(&self.task_max_processes)
        {
            return Err(ResourceViolation::InvalidConfig);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceViolation {
    InvalidConfig,
    ConcurrentTasks,
    TaskMemory,
    TotalMemory,
    ProcessCount,
    CircuitOpen,
    MonitoringUnavailable,
}

impl ResourceViolation {
    pub fn message(self) -> String {
        let reason = match self {
            Self::InvalidConfig => rust_i18n::t!("runtime.resource_reason_invalid_config"),
            Self::ConcurrentTasks => rust_i18n::t!("runtime.resource_reason_concurrent_tasks"),
            Self::TaskMemory => rust_i18n::t!("runtime.resource_reason_task_memory"),
            Self::TotalMemory => rust_i18n::t!("runtime.resource_reason_total_memory"),
            Self::ProcessCount => rust_i18n::t!("runtime.resource_reason_process_count"),
            Self::CircuitOpen => rust_i18n::t!("runtime.resource_reason_circuit_open"),
            Self::MonitoringUnavailable => {
                rust_i18n::t!("runtime.resource_reason_monitoring_unavailable")
            }
        };
        rust_i18n::t!("runtime.resource_limit", reason = reason).to_string()
    }
}

#[derive(Clone, Debug)]
pub(super) struct ProcessSample {
    pub pid: u32,
    pub parent: u32,
    pub group: u32,
    pub started: u64,
    pub memory_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_limits_validate_task_and_total_separately() {
        let mut limits = ResourceLimits::default();
        assert!(limits.validate().is_ok());
        limits.task_memory_mb = limits.total_memory_mb + 1;
        assert_eq!(limits.validate(), Err(ResourceViolation::InvalidConfig));
        limits.task_memory_mb = 128;
        limits.max_concurrent_tasks = 0;
        assert!(limits.validate().is_err());
        limits.max_concurrent_tasks = 1;
        limits.task_max_processes = 0;
        assert!(limits.validate().is_err());
    }

    #[test]
    fn resource_limits_wire_contract_rejects_unknown_and_missing_fields() {
        let value = serde_json::to_value(ResourceLimits::default()).unwrap();
        assert_eq!(value["totalMemoryMb"], 8192);
        assert_eq!(value["taskMemoryMb"], 4096);
        let mut invalid = value.clone();
        invalid["unexpected"] = true.into();
        assert!(serde_json::from_value::<ResourceLimits>(invalid).is_err());
        assert!(serde_json::from_value::<ResourceLimits>(serde_json::json!({})).is_err());
    }
}
