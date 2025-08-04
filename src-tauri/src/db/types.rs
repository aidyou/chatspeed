use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::ProxyGroup;

use super::mcp::Mcp;

// use crate::plugins::traits::PluginType;

// =================================================
// conversation and message
// =================================================
/// Represents a message in a conversation.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Message {
    pub id: Option<i64>,
    #[serde(rename = "conversationId")]
    pub conversation_id: i64,
    pub role: String,
    pub content: String,
    pub timestamp: String,
    pub metadata: Option<Value>,
}

/// Represents a conversation topic.
#[derive(Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: Option<i64>,
    pub title: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "isFavorite")]
    pub is_favorite: bool,
}

// =================================================
// config
// =================================================
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    pub group: String,
    pub reasoning: bool,
    #[serde(rename = "functionCall")]
    pub function_call: bool,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            group: String::new(),
            reasoning: false,
            function_call: false,
        }
    }
}

/// Represents an AI model with various attributes.
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct AiModel {
    /// Optional identifier for the AI model.
    pub id: Option<i64>,
    /// Name of the AI model.
    pub name: String,
    /// Supported models include: gpt-4, gpt-3.5, gemini-1.5, and others.
    pub models: Vec<ModelConfig>,
    /// Default model to use.
    #[serde(rename = "defaultModel")]
    pub default_model: String,
    /// API provider, including openai, azure, and others.
    #[serde(rename = "apiProtocol")]
    pub api_protocol: String,
    /// Base URL for the model's API endpoint.
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// API key used for authenticating requests.
    #[serde(rename = "apiKey")]
    pub api_key: String,
    /// Max tokens for the model.
    #[serde(rename = "maxTokens")]
    pub max_tokens: i32,
    /// Temperature for the model.
    pub temperature: f32,
    /// Top P for the model.
    #[serde(rename = "topP")]
    pub top_p: f32,
    /// Top K for the model.
    #[serde(rename = "topK")]
    pub top_k: i32,
    /// Index used for sorting models.
    #[serde(rename = "sortIndex")]
    pub sort_index: i32,
    /// Flag indicating whether the model is the default.
    #[serde(rename = "isDefault")]
    pub is_default: bool,
    /// Flag indicating whether the model is disabled.
    pub disabled: bool,
    /// Flag indicating whether the model is official.
    #[serde(rename = "isOfficial")]
    pub is_official: bool,
    /// Official identifier associated with the model.
    #[serde(rename = "officialId")]
    pub official_id: String,
    /// Additional metadata stored as JSON
    pub metadata: Option<Value>,
}

/// Represents an AI skill with relevant details.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiSkill {
    /// Optional identifier for the AI skill.
    pub id: Option<i64>,
    /// Name of the AI skill.
    pub name: String,
    /// Icon of the AI skill.
    pub icon: String,
    /// Optional logo image for the skill
    pub logo: Option<String>,
    /// Prompt associated with the skill.
    pub prompt: String,
    /// Share identifier for the skill.
    #[serde(rename = "shareId")]
    pub share_id: Option<String>,
    /// Index used for sorting skills.
    #[serde(rename = "sortIndex")]
    pub sort_index: i32,
    /// Flag indicating whether the skill is disabled.
    pub disabled: bool,
    /// Additional metadata stored as JSON
    pub metadata: Option<Value>,
}

/// Represents the configuration settings for the application, including AI models and skills.
pub struct Config {
    /// A HashMap storing the settings as key-value pairs.
    pub settings: HashMap<String, Value>,
    /// A vector of AI models.
    pub ai_models: Vec<AiModel>,
    /// A vector of AI skills.
    pub ai_skills: Vec<AiSkill>,
    /// Mcp server configurations.
    pub mcps: Vec<Mcp>,
    pub proxy_groups: Vec<ProxyGroup>,
}

// =================================================
// plugin config struct
// =================================================

// #[derive(Debug, Serialize, Deserialize)]
// pub enum RuntimeType {
//     Python,
//     JavaScript,
//     TypeScript,
// }

// impl From<RuntimeType> for PluginType {
//     fn from(runtime_type: RuntimeType) -> Self {
//         match runtime_type {
//             RuntimeType::Python => PluginType::Python,
//             RuntimeType::JavaScript => PluginType::JavaScript,
//             RuntimeType::TypeScript => PluginType::JavaScript,
//         }
//     }
// }

// #[derive(Debug, Serialize, Deserialize)]
// pub struct Plugin {
//     pub uuid: String,
//     pub name: String,
//     pub description: Option<String>,
//     pub author: String,
//     pub version: String,
//     pub runtime_type: RuntimeType,
//     pub input_schema: Option<Value>,
//     pub output_schema: Option<Value>,
//     pub icon: Option<String>,
//     pub readme: Option<String>,
//     pub checksum: String,
//     pub created_at: String,
//     pub updated_at: String,
// }

// #[derive(Debug, Serialize, Deserialize)]
// pub struct PluginFile {
//     pub uuid: String,
//     pub plugin_id: String,
//     pub filename: String,
//     pub content: String,
//     pub is_entry: bool,
//     pub created_at: String,
//     pub updated_at: String,
// }

// #[derive(Debug, Serialize, Deserialize)]
// pub struct PluginListItem {
//     pub uuid: String,
//     pub name: String,
//     pub description: Option<String>,
//     pub author: String,
//     pub version: String,
//     pub runtime_type: RuntimeType,
//     pub icon: Option<String>,
//     pub created_at: String,
//     pub updated_at: String,
// }
