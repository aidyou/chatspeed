//! Models.dev catalog wire types shared by the desktop and the runtime backend.
//!
//! The Models.dev catalog DTOs, the catalog parser, the snapshot cache/loader
//! and the endpoint-bound profile/transport resolvers are all runtime-owned:
//! they live in the desktop-free backend (`crate::model_catalog_engine`), which
//! the desktop never compiles. This module keeps only the transport-neutral
//! types both crates name — the provider-preset wire contract, the thinking
//! adapter, the capability/reasoning policy and the resolved profile — so the
//! desktop links no second parser and no embedded catalog asset.

use serde::{Deserialize, Serialize};

#[cfg(not(feature = "desktop"))]
pub mod pricing;

// The provider display payload is a shared wire contract: the desktop command,
// the runtime data-command core and the runtime client all name the same type
// instead of each keeping a catalog display definition.
pub use chatspeed_contracts::model_catalog::ModelsDevPresetProviderDto as ModelsDevPresetProvider;

use crate::db::PricingConfig;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingAdapter {
    #[serde(alias = "openai")]
    OpenAi,
    Claude,
    Gemini,
    #[serde(alias = "deepseek")]
    DeepSeek,
    Qwen,
    Glm,
    Kimi,
    #[serde(alias = "stepfun")]
    StepFun,
    HunyuanHy4Preview,
    Doubao,
    #[serde(alias = "sensenova")]
    SenseNova,
    Mistral,
    Mimo,
    Minimax,
    NvidiaNim,
    Amd,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub function_call: Option<bool>,
    #[serde(default)]
    pub image_input: Option<bool>,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            reasoning: None,
            function_call: None,
            image_input: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReasoningPolicy {
    pub supported_efforts: Vec<String>,
    pub default_effort: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedModelProfile {
    pub catalog_version: u32,
    pub matched_profile_ids: Vec<String>,
    pub family: Option<String>,
    pub capabilities: Capabilities,
    pub attachment: Option<bool>,
    pub structured_output: Option<bool>,
    pub audio_input: Option<bool>,
    pub audio_output: Option<bool>,
    pub video_input: Option<bool>,
    pub pdf_input: Option<bool>,
    pub context_size: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub recommended_temperature: Option<f32>,
    pub reasoning: Option<ReasoningPolicy>,
    pub thinking_adapter: Option<ThinkingAdapter>,
    pub matched_transport_id: Option<String>,
    pub pricing: Option<PricingConfig>,
}
