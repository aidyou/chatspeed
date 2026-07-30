use crate::ccproxy::adapter::unified::UnifiedThinking;
use crate::ccproxy::types::openai::{
    OpenAIMessageContent, OpenAIMessageContentPart, ZhipuThinking,
};
use crate::db::ThinkingConfig;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReasoningModelFamily {
    OpenAI,
    DeepSeek,
    Qwen,
    Kimi,
    MiniMax,
    Glm,
    Claude,
    Gemini,
    Ollama,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenAiCompatThinkingFields {
    pub thinking: Option<ZhipuThinking>,
    pub reasoning_effort: Option<String>,
    pub thinking_budget: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VendorThinkingParams {
    pub reasoning_effort: Option<String>,
    pub reasoning_split: Option<bool>,
    pub thinking: Option<ZhipuThinking>,
    pub enable_thinking: Option<bool>,
    pub thinking_budget: Option<i32>,
}

pub(crate) fn detect_family(model: &str) -> ReasoningModelFamily {
    let lower = model.to_lowercase();
    if lower.contains("ollama") {
        ReasoningModelFamily::Ollama
    } else if lower.starts_with("gpt-")
        || lower.starts_with("chatgpt")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
        || lower.starts_with("o4")
    {
        ReasoningModelFamily::OpenAI
    } else if lower.contains("deepseek") {
        ReasoningModelFamily::DeepSeek
    } else if lower.contains("qwen") || lower.contains("qwq") {
        ReasoningModelFamily::Qwen
    } else if lower.contains("kimi") || lower.contains("moonshot") {
        ReasoningModelFamily::Kimi
    } else if lower.contains("minimax") {
        ReasoningModelFamily::MiniMax
    } else if lower.contains("glm") {
        ReasoningModelFamily::Glm
    } else if lower.contains("claude") {
        ReasoningModelFamily::Claude
    } else if lower.contains("gemini") {
        ReasoningModelFamily::Gemini
    } else {
        ReasoningModelFamily::Other
    }
}

pub fn supports_native_reasoning_history_for_openai_backend(model: &str) -> bool {
    matches!(
        detect_family(model),
        ReasoningModelFamily::OpenAI
            | ReasoningModelFamily::DeepSeek
            | ReasoningModelFamily::Qwen
            | ReasoningModelFamily::Kimi
            | ReasoningModelFamily::MiniMax
            | ReasoningModelFamily::Glm
    )
}

pub fn merge_reasoning_into_openai_message_content(
    content: Option<OpenAIMessageContent>,
    reasoning: &str,
) -> Option<OpenAIMessageContent> {
    let trimmed_reasoning = reasoning.trim();
    if trimmed_reasoning.is_empty() {
        return content;
    }

    let think_block = format!("<think>\n{}\n</think>", trimmed_reasoning);

    match content {
        Some(OpenAIMessageContent::Text(text)) => {
            let trimmed_text = text.trim();
            if trimmed_text.is_empty() {
                Some(OpenAIMessageContent::Text(think_block))
            } else {
                Some(OpenAIMessageContent::Text(format!(
                    "{}\n\n{}",
                    think_block, trimmed_text
                )))
            }
        }
        Some(OpenAIMessageContent::Parts(mut parts)) => {
            parts.insert(0, OpenAIMessageContentPart::Text { text: think_block });
            Some(OpenAIMessageContent::Parts(parts))
        }
        None => Some(OpenAIMessageContent::Text(think_block)),
    }
}

fn normalize_effort(raw: &str) -> Option<String> {
    match raw.trim().to_lowercase().as_str() {
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" => {
            Some(raw.trim().to_lowercase())
        }
        _ => None,
    }
}

pub fn effort_from_budget_tokens(budget_tokens: Option<i32>) -> Option<String> {
    let budget = budget_tokens?;
    if budget <= 0 {
        return None;
    }

    let effort = if budget <= 1024 {
        "low"
    } else if budget <= 2048 {
        "medium"
    } else if budget <= 4096 {
        "high"
    } else {
        "xhigh"
    };

    Some(effort.to_string())
}

pub fn build_nvidia_deepseek_v4_chat_template_kwargs(
    include_thoughts: bool,
    reasoning_effort: Option<&str>,
    budget_tokens: Option<i32>,
) -> Value {
    if !include_thoughts {
        return json!({ "thinking": false });
    }

    let normalized_effort = reasoning_effort
        .and_then(normalize_effort)
        .or_else(|| effort_from_budget_tokens(budget_tokens));
    let reasoning_effort = match normalized_effort.as_deref() {
        Some("xhigh" | "max") => "max",
        _ => "high",
    };

    json!({
        "thinking": true,
        "reasoning_effort": reasoning_effort,
    })
}

pub fn normalize_nvidia_deepseek_v4_thinking_fields(
    body: &mut Value,
    provider_url: &str,
    model: &str,
) {
    let is_nvidia_nim = reqwest::Url::parse(provider_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.eq_ignore_ascii_case("integrate.api.nvidia.com"));
    let is_deepseek_v4 = model.to_lowercase().contains("deepseek-v4-");
    if !is_nvidia_nim || !is_deepseek_v4 {
        return;
    }

    let explicit_enabled = body
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .map(|value| value.eq_ignore_ascii_case("enabled"))
        .or_else(|| body.get("enable_thinking").and_then(Value::as_bool));
    let reasoning_effort = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let thinking_budget = body
        .get("thinking_budget")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let has_generic_thinking_signal = explicit_enabled.is_some()
        || reasoning_effort.is_some()
        || thinking_budget.is_some_and(|value| value > 0);

    if let Some(body_object) = body.as_object_mut() {
        if has_generic_thinking_signal {
            let include_thoughts = explicit_enabled.unwrap_or_else(|| {
                !reasoning_effort
                    .as_deref()
                    .is_some_and(|value| value.eq_ignore_ascii_case("none"))
            });
            let inferred_kwargs = build_nvidia_deepseek_v4_chat_template_kwargs(
                include_thoughts,
                reasoning_effort.as_deref(),
                thinking_budget,
            );
            let chat_template_kwargs = body_object
                .entry("chat_template_kwargs".to_string())
                .or_insert_with(|| json!({}));
            if let (Some(target), Some(inferred)) = (
                chat_template_kwargs.as_object_mut(),
                inferred_kwargs.as_object(),
            ) {
                for (key, value) in inferred {
                    target.entry(key.clone()).or_insert_with(|| value.clone());
                }
            } else {
                *chat_template_kwargs = inferred_kwargs;
            }
        }

        body_object.remove("thinking");
        body_object.remove("enable_thinking");
        body_object.remove("thinking_budget");
        body_object.remove("reasoning_effort");
    }
}

pub fn build_openai_compat_thinking_fields(
    model: &str,
    thinking: Option<&ThinkingConfig>,
) -> OpenAiCompatThinkingFields {
    let Some(thinking) = thinking else {
        return OpenAiCompatThinkingFields::default();
    };

    let family = detect_family(model);
    let enabled = thinking.r#type.eq_ignore_ascii_case("enabled");

    let reasoning_effort = if enabled {
        match family {
            ReasoningModelFamily::OpenAI
            | ReasoningModelFamily::DeepSeek
            | ReasoningModelFamily::Qwen
            | ReasoningModelFamily::MiniMax
            | ReasoningModelFamily::Glm
            | ReasoningModelFamily::Kimi => {
                effort_from_budget_tokens(thinking.budget_tokens.map(|v| v as i32))
            }
            _ => None,
        }
    } else {
        None
    };

    OpenAiCompatThinkingFields {
        thinking: Some(ZhipuThinking {
            r#type: if enabled {
                "enabled".to_string()
            } else {
                "disabled".to_string()
            },
        }),
        reasoning_effort,
        thinking_budget: if enabled {
            thinking.budget_tokens.map(|v| v as i32)
        } else {
            None
        },
    }
}

pub fn build_unified_thinking_from_openai_request(
    thinking: Option<&ZhipuThinking>,
    reasoning_effort: Option<&str>,
    reasoning_split: Option<bool>,
    thinking_budget: Option<i32>,
) -> Option<UnifiedThinking> {
    let explicit_enabled = thinking.map(|value| value.r#type.eq_ignore_ascii_case("enabled"));
    let has_any_signal =
        explicit_enabled.is_some() || reasoning_effort.is_some() || reasoning_split == Some(true);
    let has_budget_signal = thinking_budget.unwrap_or_default() > 0;

    if !has_any_signal && !has_budget_signal {
        return None;
    }

    Some(UnifiedThinking {
        include_thoughts: Some(explicit_enabled.unwrap_or(true)),
        budget_tokens: if explicit_enabled == Some(false) {
            Some(0)
        } else {
            thinking_budget
        },
    })
}

pub fn adapt_vendor_thinking_params_for_openai_backend(
    model: &str,
    thinking: Option<&UnifiedThinking>,
    reasoning_effort: Option<&str>,
    has_tools: bool,
) -> VendorThinkingParams {
    let family = detect_family(model);
    let Some(thinking) = thinking else {
        return VendorThinkingParams::default();
    };

    let include_thoughts = thinking.include_thoughts.unwrap_or(true);
    let normalized_effort = reasoning_effort
        .and_then(normalize_effort)
        .or_else(|| effort_from_budget_tokens(thinking.budget_tokens));

    match family {
        ReasoningModelFamily::OpenAI => VendorThinkingParams {
            reasoning_effort: if include_thoughts {
                normalized_effort
            } else {
                None
            },
            ..Default::default()
        },
        ReasoningModelFamily::DeepSeek => {
            let mapped_effort = if include_thoughts {
                let selected = if has_tools {
                    "max".to_string()
                } else {
                    normalized_effort.unwrap_or_else(|| "high".to_string())
                };
                let deepseek_effort = match selected.as_str() {
                    "none" | "minimal" | "low" | "medium" | "high" => "high",
                    "xhigh" | "max" => "max",
                    _ => "high",
                };
                Some(deepseek_effort.to_string())
            } else {
                None
            };

            VendorThinkingParams {
                reasoning_effort: mapped_effort,
                thinking: Some(ZhipuThinking {
                    r#type: if include_thoughts {
                        "enabled".to_string()
                    } else {
                        "disabled".to_string()
                    },
                }),
                ..Default::default()
            }
        }
        ReasoningModelFamily::Qwen => VendorThinkingParams {
            enable_thinking: Some(include_thoughts),
            thinking_budget: if include_thoughts {
                thinking.budget_tokens
            } else {
                None
            },
            ..Default::default()
        },
        ReasoningModelFamily::Kimi | ReasoningModelFamily::Glm => VendorThinkingParams {
            thinking: Some(ZhipuThinking {
                r#type: if include_thoughts {
                    "enabled".to_string()
                } else {
                    "disabled".to_string()
                },
            }),
            ..Default::default()
        },
        ReasoningModelFamily::MiniMax => VendorThinkingParams {
            reasoning_split: if include_thoughts { Some(true) } else { None },
            ..Default::default()
        },
        _ => VendorThinkingParams::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        adapt_vendor_thinking_params_for_openai_backend, build_openai_compat_thinking_fields,
        build_unified_thinking_from_openai_request, effort_from_budget_tokens,
        merge_reasoning_into_openai_message_content, normalize_nvidia_deepseek_v4_thinking_fields,
        supports_native_reasoning_history_for_openai_backend,
    };
    use crate::ccproxy::adapter::unified::UnifiedThinking;
    use crate::ccproxy::types::openai::{OpenAIMessageContent, ZhipuThinking};
    use crate::db::ThinkingConfig;

    #[test]
    fn budget_to_effort_mapping_matches_expected_ranges() {
        assert_eq!(effort_from_budget_tokens(Some(512)).as_deref(), Some("low"));
        assert_eq!(
            effort_from_budget_tokens(Some(1024)).as_deref(),
            Some("low")
        );
        assert_eq!(
            effort_from_budget_tokens(Some(1025)).as_deref(),
            Some("medium")
        );
        assert_eq!(
            effort_from_budget_tokens(Some(2048)).as_deref(),
            Some("medium")
        );
        assert_eq!(
            effort_from_budget_tokens(Some(4096)).as_deref(),
            Some("high")
        );
        assert_eq!(
            effort_from_budget_tokens(Some(4097)).as_deref(),
            Some("xhigh")
        );
    }

    #[test]
    fn openai_compat_fields_include_reasoning_effort_for_deepseek() {
        let fields = build_openai_compat_thinking_fields(
            "deepseek-chat",
            Some(&ThinkingConfig {
                r#type: "enabled".to_string(),
                budget_tokens: Some(2048),
            }),
        );
        assert_eq!(
            fields.thinking.as_ref().map(|value| value.r#type.as_str()),
            Some("enabled")
        );
        assert_eq!(fields.reasoning_effort.as_deref(), Some("medium"));
        assert_eq!(fields.thinking_budget, Some(2048));
    }

    #[test]
    fn nvidia_deepseek_v4_uses_chat_template_kwargs_instead_of_top_level_fields() {
        let mut body = serde_json::json!({
            "thinking": { "type": "enabled" },
            "reasoning_effort": "xhigh",
            "thinking_budget": 8192,
        });

        normalize_nvidia_deepseek_v4_thinking_fields(
            &mut body,
            "https://integrate.api.nvidia.com/v1/chat/completions",
            "deepseek-ai/deepseek-v4-flash",
        );

        assert_eq!(
            body["chat_template_kwargs"],
            serde_json::json!({ "thinking": true, "reasoning_effort": "max" })
        );
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn nvidia_deepseek_v4_preserves_explicit_chat_template_kwargs() {
        let mut body = serde_json::json!({
            "thinking": { "type": "enabled" },
            "reasoning_effort": "medium",
            "thinking_budget": 2048,
            "chat_template_kwargs": {
                "thinking": false,
                "reasoning_effort": "max",
                "custom": true
            }
        });

        normalize_nvidia_deepseek_v4_thinking_fields(
            &mut body,
            "https://integrate.api.nvidia.com/v1",
            "deepseek-ai/deepseek-v4-flash",
        );

        assert_eq!(
            body["chat_template_kwargs"],
            serde_json::json!({
                "thinking": false,
                "reasoning_effort": "max",
                "custom": true
            })
        );
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn nvidia_deepseek_v4_maps_none_effort_to_thinking_disabled() {
        let mut body = serde_json::json!({
            "reasoning_effort": "none",
            "thinking_budget": 0,
        });

        normalize_nvidia_deepseek_v4_thinking_fields(
            &mut body,
            "https://integrate.api.nvidia.com/v1",
            "deepseek-ai/deepseek-v4-flash",
        );

        assert_eq!(
            body["chat_template_kwargs"],
            serde_json::json!({ "thinking": false })
        );
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn openai_request_thinking_is_converted_into_unified_thinking() {
        let unified = build_unified_thinking_from_openai_request(
            Some(&ZhipuThinking {
                r#type: "enabled".to_string(),
            }),
            Some("high"),
            None,
            Some(1024),
        )
        .expect("unified thinking should exist");
        assert_eq!(unified.include_thoughts, Some(true));
        assert_eq!(unified.budget_tokens, Some(1024));
    }

    #[test]
    fn deepseek_vendor_mapping_promotes_agent_requests_to_max() {
        let params = adapt_vendor_thinking_params_for_openai_backend(
            "deepseek-chat",
            Some(&UnifiedThinking {
                include_thoughts: Some(true),
                budget_tokens: Some(1024),
            }),
            Some("medium"),
            true,
        );
        assert_eq!(
            params.thinking.as_ref().map(|value| value.r#type.as_str()),
            Some("enabled")
        );
        assert_eq!(params.reasoning_effort.as_deref(), Some("max"));
    }

    #[test]
    fn qwen_vendor_mapping_uses_enable_thinking_and_budget() {
        let params = adapt_vendor_thinking_params_for_openai_backend(
            "qwen3-235b-a22b",
            Some(&UnifiedThinking {
                include_thoughts: Some(true),
                budget_tokens: Some(8192),
            }),
            None,
            false,
        );
        assert_eq!(params.enable_thinking, Some(true));
        assert_eq!(params.thinking_budget, Some(8192));
    }

    #[test]
    fn native_reasoning_history_support_uses_vendor_whitelist() {
        assert!(supports_native_reasoning_history_for_openai_backend(
            "deepseek-reasoner"
        ));
        assert!(supports_native_reasoning_history_for_openai_backend(
            "gpt-5"
        ));
        assert!(!supports_native_reasoning_history_for_openai_backend(
            "claude-3-7-sonnet"
        ));
        assert!(!supports_native_reasoning_history_for_openai_backend(
            "gemini-2.5-pro"
        ));
        assert!(!supports_native_reasoning_history_for_openai_backend(
            "ollama/qwen3"
        ));
    }

    #[test]
    fn merge_reasoning_into_content_wraps_text_in_think_block() {
        let merged = merge_reasoning_into_openai_message_content(
            Some(OpenAIMessageContent::Text("Visible answer".to_string())),
            "Need to inspect first",
        );

        match merged {
            Some(OpenAIMessageContent::Text(text)) => assert_eq!(
                text,
                "<think>\nNeed to inspect first\n</think>\n\nVisible answer"
            ),
            other => panic!("unexpected merged content: {:?}", other),
        }
    }
}
