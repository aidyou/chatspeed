use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxRuntime {
    Msb,
    Docker,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxAvailabilityState {
    UnsupportedPlatform,
    NotInstalled,
    InstalledButUnhealthy,
    UnsupportedVersion,
    ReadyMissingImage,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRuntimeStatus {
    pub runtime: SandboxRuntime,
    pub state: SandboxAvailabilityState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub image_sizes: BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxRuntimeStatusSummary {
    pub msb: SandboxRuntimeStatus,
    pub docker: SandboxRuntimeStatus,
}

#[derive(Debug, Clone)]
pub struct SandboxDetectorOptions {
    pub msb_binary: String,
    pub docker_binary: String,
    pub timeout: Duration,
    pub required_images: Vec<String>,
}

impl Default for SandboxDetectorOptions {
    fn default() -> Self {
        Self {
            msb_binary: "msb".to_string(),
            docker_binary: "docker".to_string(),
            timeout: Duration::from_secs(3),
            required_images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxFailureReason {
    SpawnFailed,
    RunnerFailed,
    TimedOut,
    UnsupportedNetwork,
    InvalidPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxFailure {
    pub backend: ShellExecutionBackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<SandboxRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub reason: SandboxFailureReason,
    pub message: String,
    #[serde(rename = "execution_plan", skip_serializing_if = "Option::is_none")]
    pub execution_plan: Option<serde_json::Value>,
}

impl SandboxFailure {
    pub fn from_plan(
        plan: &ShellExecutionPlan,
        reason: SandboxFailureReason,
        message: impl Into<String>,
    ) -> Self {
        Self {
            backend: plan.backend.clone(),
            runtime: plan.runtime.clone(),
            profile: plan.profile.clone(),
            image: plan.image.clone(),
            reason,
            message: message.into(),
            execution_plan: serde_json::to_value(plan).ok(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellExecutionBackendKind {
    Msb,
    Docker,
    Host,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostFallbackReason {
    HostOnlyMode,
    SandboxConfigMissing,
    ProfileUnavailable,
    RuntimeUnavailable,
    MissingImage,
    AmbiguousRoute,
    MixedBackendCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellExecutionRiskFloor {
    Normal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellExecutionPlanStatus {
    Ready,
    ConsentRequired,
    Denied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShellExecutionBackendOrigin {
    #[default]
    HostOnly,
    ExplicitHostRule,
    SandboxProfile,
    HostFallbackPending,
    ApprovedHostFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SandboxMountPlan {
    pub host_path: String,
    pub guest_path: String,
    pub access: WorkspaceAccess,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ShellExecutionPlan {
    pub tool_call_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme_revision: Option<String>,
    pub backend: ShellExecutionBackendKind,
    #[serde(default)]
    pub backend_origin: ShellExecutionBackendOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<SandboxRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<SandboxResourceLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_access: Option<WorkspaceAccess>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<SandboxMountPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<HostFallbackReason>,
    pub risk_floor: ShellExecutionRiskFloor,
    pub status: ShellExecutionPlanStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ShellExecutionPlanDetails {
    pub command: String,
    pub execution_backend: ShellExecutionBackendKind,
    pub backend_origin: ShellExecutionBackendOrigin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<SandboxRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_access: Option<WorkspaceAccess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<SandboxResourceLimits>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<SandboxMountPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<HostFallbackReason>,
    pub tool_call_id: String,
}

impl ShellExecutionPlan {
    pub fn approval_details(&self) -> ShellExecutionPlanDetails {
        ShellExecutionPlanDetails {
            command: self.command.clone(),
            execution_backend: self.backend.clone(),
            backend_origin: self.backend_origin.clone(),
            runtime: self.runtime.clone(),
            sandbox_profile: self.profile.clone(),
            sandbox_image: self.image.clone(),
            network: self.network.clone(),
            workspace_access: self.workspace_access.clone(),
            limits: self.resources.clone(),
            mounts: self.mounts.clone(),
            fallback_reason: self.fallback_reason.clone(),
            tool_call_id: self.tool_call_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShellExecutionMode {
    Auto,
    SandboxOnly,
    HostOnly,
}

impl ShellExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::SandboxOnly => "sandbox_only",
            Self::HostOnly => "host_only",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "sandbox_only" => Some(Self::SandboxOnly),
            "host_only" => Some(Self::HostOnly),
            _ => None,
        }
    }
}

impl Default for ShellExecutionMode {
    fn default() -> Self {
        Self::HostOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxRuntimePreference {
    Auto,
    Msb,
    Docker,
}

impl Default for SandboxRuntimePreference {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxNetworkMode {
    None,
    Public,
    Host,
    Allowlist,
}

impl Default for SandboxNetworkMode {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxNetworkPolicy {
    #[serde(default)]
    pub mode: SandboxNetworkMode,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowlist: Vec<String>,
}

impl Default for SandboxNetworkPolicy {
    fn default() -> Self {
        Self {
            mode: SandboxNetworkMode::None,
            allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAccess {
    ReadOnly,
    ReadWrite,
}

impl Default for WorkspaceAccess {
    fn default() -> Self {
        Self::ReadWrite
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SandboxResourceLimits {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpus: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SandboxProfileConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_patterns: Vec<String>,
    #[serde(default)]
    pub runtime_preference: SandboxRuntimePreference,
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_size_bytes: Option<u64>,
    #[serde(default)]
    pub network: SandboxNetworkPolicy,
    #[serde(default)]
    pub resources: SandboxResourceLimits,
    #[serde(default)]
    pub workspace_access: WorkspaceAccess,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HostCapabilityRule {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities_all: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invocation_tags_any: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SandboxSchemeConfig {
    #[serde(default)]
    pub runtime_preference: SandboxRuntimePreference,
    #[serde(default)]
    pub profiles: Vec<SandboxProfileConfig>,
    #[serde(default)]
    pub host_rules: Vec<HostCapabilityRule>,
}

impl SandboxSchemeConfig {
    pub fn validate(&self) -> Result<(), String> {
        let mut profile_ids = std::collections::BTreeSet::new();
        let mut host_rule_ids = std::collections::BTreeSet::new();

        for profile in &self.profiles {
            if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
                return Err("sandbox profile id and name cannot be empty".to_string());
            }
            if !profile_ids.insert(profile.id.trim().to_string()) {
                return Err(format!("duplicate sandbox profile id: {}", profile.id));
            }
            if profile.image.trim().is_empty() {
                return Err(format!(
                    "sandbox profile {} image cannot be empty",
                    profile.name
                ));
            }
            for pattern in &profile.command_patterns {
                regex::Regex::new(pattern).map_err(|error| {
                    format!(
                        "sandbox profile {} command pattern is invalid: {error}",
                        profile.name
                    )
                })?;
            }
        }

        for rule in &self.host_rules {
            if rule.id.trim().is_empty() || rule.name.trim().is_empty() {
                return Err("host capability rule id and name cannot be empty".to_string());
            }
            if !host_rule_ids.insert(rule.id.trim().to_string()) {
                return Err(format!("duplicate host capability rule id: {}", rule.id));
            }
            if rule.capabilities_all.is_empty()
                && rule.invocation_tags_any.is_empty()
                && rule.command_patterns.is_empty()
            {
                return Err(format!(
                    "host capability rule {} has no match criteria",
                    rule.name
                ));
            }
            for pattern in &rule.command_patterns {
                regex::Regex::new(pattern).map_err(|error| {
                    format!(
                        "host capability rule {} command pattern is invalid: {error}",
                        rule.name
                    )
                })?;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSandboxConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme_revision: Option<String>,
    #[serde(default)]
    pub execution_mode: ShellExecutionMode,
    #[serde(default)]
    pub runtime_preference: SandboxRuntimePreference,
    #[serde(default = "default_profile")]
    pub default_profile: String,
    #[serde(default)]
    pub profiles: BTreeMap<String, SandboxProfileConfig>,
    #[serde(default)]
    pub host_rules: Vec<HostCapabilityRule>,
}

impl Default for AgentSandboxConfig {
    fn default() -> Self {
        Self {
            scheme_id: None,
            scheme_revision: None,
            execution_mode: ShellExecutionMode::Auto,
            runtime_preference: SandboxRuntimePreference::Auto,
            default_profile: default_profile(),
            profiles: BTreeMap::new(),
            host_rules: Vec::new(),
        }
    }
}

fn default_profile() -> String {
    "busybox".to_string()
}

impl AgentSandboxConfig {
    pub fn required_images(&self) -> Vec<String> {
        let mut images = self
            .profiles
            .values()
            .filter(|profile| profile.enabled)
            .map(|profile| profile.image.trim())
            .filter(|image| !image.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        images.sort();
        images.dedup();
        images
    }

    pub fn from_json(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }

    pub fn to_json(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}
