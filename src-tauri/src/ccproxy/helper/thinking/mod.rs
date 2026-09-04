mod amd;
mod claude;
mod common;
mod deepseek;
mod doubao;
mod gemini;
mod glm;
mod hunyuan;
mod kimi;
mod mimo;
mod mistral;
mod openai;
mod qwen;
mod sensenova;
mod stepfun;

use crate::ai::model_catalog::ThinkingAdapter;
use serde_json::Value;

/// Normalizes vendor-specific thinking fields using a resolved catalog adapter.
pub fn normalize_request_with_adapter(
    body: &mut Value,
    model: &str,
    base_url: &str,
    adapter: Option<ThinkingAdapter>,
) {
    let Some(adapter) = adapter else {
        return;
    };
    match adapter {
        ThinkingAdapter::DeepSeek => deepseek::normalize_request(body, model, base_url),
        ThinkingAdapter::Doubao => doubao::normalize_request(body, model, base_url),
        ThinkingAdapter::Mistral => mistral::normalize_request(body, model, base_url),
        ThinkingAdapter::Mimo => mimo::normalize_request(body, model, base_url),
        ThinkingAdapter::HunyuanHy4Preview => hunyuan::normalize_request(body, model, base_url),
        ThinkingAdapter::Glm => glm::normalize_request(body, model, base_url),
        ThinkingAdapter::Kimi => kimi::normalize_request(body, model, base_url),
        ThinkingAdapter::Qwen => qwen::normalize_request(body, model, base_url),
        ThinkingAdapter::StepFun => stepfun::normalize_request(body, model, base_url),
        ThinkingAdapter::Claude => claude::normalize_request(body, model, base_url),
        ThinkingAdapter::Gemini => gemini::normalize_request(body, model, base_url),
        ThinkingAdapter::OpenAi => openai::normalize_request(body, model, base_url),
        ThinkingAdapter::SenseNova => sensenova::normalize_request(body, model, base_url),
        ThinkingAdapter::Amd => amd::normalize_request(body, model, base_url),
        ThinkingAdapter::Minimax | ThinkingAdapter::NvidiaNim => {
            common::normalize_request(body, model, base_url)
        }
    }
}

/// Normalizes vendor-specific thinking fields immediately before a provider request is
/// forwarded. Unknown providers use the no-op common fallback to preserve their contracts.
#[cfg(test)]
pub fn normalize_request(body: &mut Value, model: &str, base_url: &str) {
    if doubao::applies_to(model) {
        doubao::normalize_request(body, model, base_url);
    } else if mistral::applies_to(model) {
        mistral::normalize_request(body, model, base_url);
    } else if mimo::applies_to(model) {
        mimo::normalize_request(body, model, base_url);
    } else if hunyuan::applies_to(model) {
        hunyuan::normalize_request(body, model, base_url);
    } else if glm::applies_to(base_url) {
        glm::normalize_request(body, model, base_url);
    } else if kimi::applies_to(base_url) {
        kimi::normalize_request(body, model, base_url);
    } else if qwen::applies_to(base_url) {
        qwen::normalize_request(body, model, base_url);
    } else if deepseek::applies_to(base_url) {
        deepseek::normalize_request(body, model, base_url);
    } else if stepfun::applies_to(base_url) {
        stepfun::normalize_request(body, model, base_url);
    } else if claude::applies_to(base_url) {
        claude::normalize_request(body, model, base_url);
    } else if gemini::applies_to(base_url) {
        gemini::normalize_request(body, model, base_url);
    } else if openai::applies_to(base_url) {
        openai::normalize_request(body, model, base_url);
    } else if sensenova::applies_to(model) {
        sensenova::normalize_request(body, model, base_url);
    } else if amd::applies_to(base_url) {
        amd::normalize_request(body, model, base_url);
    } else {
        common::normalize_request(body, model, base_url);
    }
}

pub fn is_kimi_k3_model(model: &str) -> bool {
    kimi::is_k3_model(model)
}

pub fn is_kimi_k2_7_code_model(model: &str) -> bool {
    kimi::is_k2_7_code_model(model)
}

#[cfg(test)]
mod tests {
    use super::normalize_request;
    use serde_json::json;

    #[test]
    fn glm_5_3_forces_thinking_and_maps_effort() {
        let mut body = json!({
            "thinking": { "type": "disabled" },
            "reasoning_effort": "xhigh",
        });

        normalize_request(
            &mut body,
            "glm-5.3-flash",
            "https://open.bigmodel.cn/api/paas/v4/chat/completions",
        );

        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "max");
    }

    #[test]
    fn kimi_k3_removes_thinking_and_maps_effort() {
        let mut body = json!({
            "thinking": { "type": "disabled" },
            "thinking_budget": 4096,
            "reasoning_effort": "medium",
        });

        normalize_request(
            &mut body,
            "kimi-k3",
            "https://api.moonshot.cn/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert!(body.get("thinking_budget").is_none());
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn qwen_thinking_only_model_forces_enable_thinking() {
        let mut body = json!({
            "thinking": { "type": "disabled" },
            "reasoning_effort": "high",
        });

        normalize_request(
            &mut body,
            "qwen3-235b-a22b-thinking-2507",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
        );

        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["enable_thinking"], true);
    }

    #[test]
    fn deepseek_maps_xhigh_to_high() {
        let mut body = json!({ "reasoning_effort": "xhigh" });

        normalize_request(
            &mut body,
            "deepseek-v4-flash",
            "https://api.deepseek.com/v1/chat/completions",
        );

        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn hunyuan_hy4_preview_maps_unsupported_effort_to_high() {
        let mut body = json!({ "reasoning_effort": "medium" });

        normalize_request(
            &mut body,
            "hy4-preview",
            "https://api.lkeap.cloud.tencent.com/v1",
        );

        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn stepfun_maps_xhigh_to_high() {
        let mut body = json!({ "reasoning_effort": "xhigh" });

        normalize_request(
            &mut body,
            "step-3.7-flash",
            "https://api.stepfun.ai/v1/chat/completions",
        );

        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn mimo_replays_reasoning_for_tool_history() {
        let mut body = json!({
            "thinking": { "type": "enabled" },
            "messages": [
                { "role": "assistant", "content": "", "tool_calls": [{}] },
                { "role": "tool", "content": "result" }
            ]
        });

        normalize_request(&mut body, "mimo-v2.5", "https://api.xiaomimimo.com/v1");

        assert_eq!(body["messages"][0]["reasoning_content"], "");
    }

    #[test]
    fn mimo_preserves_raw_reasoning_for_tool_history() {
        let mut body = json!({
            "thinking": { "type": "enabled" },
            "messages": [
                {
                    "role": "assistant",
                    "reasoning_content": "raw reasoning from the previous turn",
                    "content": "",
                    "tool_calls": [{}]
                },
                { "role": "tool", "content": "result" }
            ]
        });

        normalize_request(&mut body, "mimo-v2.5", "https://api.xiaomimimo.com/v1");

        assert_eq!(
            body["messages"][0]["reasoning_content"],
            "raw reasoning from the previous turn"
        );
    }

    #[test]
    fn doubao_removes_unsupported_effort_and_budget() {
        let mut body = json!({
            "thinking": { "type": "auto" },
            "reasoning_effort": "high",
            "thinking_budget": 2048,
        });

        normalize_request(
            &mut body,
            "ByteDance/doubao-seed-1.6",
            "https://example.com/v1",
        );

        assert_eq!(body["thinking"]["type"], "auto");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking_budget").is_none());
    }

    #[test]
    fn gemini_maps_xhigh_to_supported_thinking_level() {
        let mut body = json!({
            "generationConfig": { "thinkingConfig": { "thinkingLevel": "xhigh" } }
        });

        normalize_request(
            &mut body,
            "gemini-3.1-flash-lite-image",
            "https://generativelanguage.googleapis.com/v1beta/models/gemini:generateContent",
        );

        assert_eq!(
            body["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "high"
        );
    }

    #[test]
    fn gemini_leaves_undocumented_models_unchanged() {
        let mut body = json!({
            "generationConfig": { "thinkingConfig": { "thinkingLevel": "xhigh" } }
        });
        let original = body.clone();

        normalize_request(
            &mut body,
            "gemini-2.0-flash",
            "https://generativelanguage.googleapis.com/v1beta/models/gemini:generateContent",
        );

        assert_eq!(body, original);
    }

    #[test]
    fn mistral_maps_effort_to_high_or_none() {
        let mut body = json!({ "reasoning_effort": "medium" });

        normalize_request(
            &mut body,
            "mistral-small-latest",
            "https://api.mistral.ai/v1",
        );

        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn claude_opus_5_enables_adaptive_thinking_for_max_effort() {
        let mut body = json!({
            "thinking": { "type": "disabled" },
            "effort": "max",
        });

        normalize_request(
            &mut body,
            "claude-opus-5",
            "https://api.anthropic.com/v1/messages",
        );

        assert_eq!(body["thinking"]["type"], "adaptive");
    }

    #[test]
    fn amd_deepseek_v4_maps_effort_to_two_tiers() {
        let mut body = json!({
            "thinking": { "type": "enabled" },
            "reasoning_effort": "xhigh",
            "thinking_budget": 2048,
        });

        normalize_request(
            &mut body,
            "DeepSeek-V4-Flash",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert!(body.get("thinking_budget").is_none());
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn amd_deepseek_v4_disables_thinking_by_omitting_effort() {
        let mut body = json!({
            "thinking": { "type": "disabled" },
            "reasoning_effort": "high",
        });

        normalize_request(
            &mut body,
            "DeepSeek-V4-Flash",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn amd_deepseek_v4_enabled_without_effort_defaults_to_high() {
        let mut body = json!({ "thinking": { "type": "enabled" } });

        normalize_request(
            &mut body,
            "DeepSeek-V4-Flash",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn amd_qwen38_maps_high_to_medium() {
        let mut body = json!({
            "thinking": { "type": "enabled" },
            "reasoning_effort": "high",
        });

        normalize_request(
            &mut body,
            "Qwen3.8-Flash-Next",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert_eq!(body["reasoning_effort"], "medium");
    }

    #[test]
    fn amd_qwen38_cannot_disable_thinking_and_falls_back_to_low() {
        let mut body = json!({ "thinking": { "type": "disabled" } });

        normalize_request(
            &mut body,
            "Qwen3.8-Flash-Next",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert_eq!(body["reasoning_effort"], "low");
    }

    #[test]
    fn amd_qwen38_enabled_without_effort_keeps_provider_default() {
        let mut body = json!({ "thinking": { "type": "enabled" } });

        normalize_request(
            &mut body,
            "Qwen3.8-Flash-Next",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn amd_deepseek_v4_removes_invalid_effort_to_disable_thinking() {
        let mut body = json!({ "reasoning_effort": "unsupported" });

        normalize_request(
            &mut body,
            "DeepSeek-V4-Flash",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn amd_qwen38_maps_invalid_effort_to_low() {
        let mut body = json!({ "reasoning_effort": "unsupported" });

        normalize_request(
            &mut body,
            "Qwen3.8-Flash-Next",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert_eq!(body["reasoning_effort"], "low");
    }

    #[test]
    fn amd_leaves_unknown_model_unchanged() {
        let mut body = json!({
            "thinking": { "type": "enabled" },
            "enable_thinking": true,
            "thinking_budget": 2048,
            "reasoning_effort": "xhigh",
        });
        let original = body.clone();

        normalize_request(
            &mut body,
            "unknown-amd-model",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert_eq!(body, original);
    }

    #[test]
    fn amd_leaves_request_without_thinking_signal_untouched() {
        let mut body = json!({
            "model": "DeepSeek-V4-Flash",
            "messages": [{ "role": "user", "content": "hi" }],
        });
        let original = body.clone();

        normalize_request(
            &mut body,
            "DeepSeek-V4-Flash",
            "https://developer.amd.com.cn/radeon/api/v1/chat/completions",
        );

        assert_eq!(body, original);
    }

    #[test]
    fn fallback_preserves_unknown_provider_request() {
        let mut body = json!({ "reasoning_effort": "xhigh" });
        let original = body.clone();

        normalize_request(
            &mut body,
            "unknown-model",
            "https://example.com/v1/chat/completions",
        );

        assert_eq!(body, original);
    }
}
