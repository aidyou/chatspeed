use lazy_static::*;
use phf::phf_map;

use crate::ai::interaction::key_rotator::ApiKeyRotator;

lazy_static! {
    pub static ref API_KEY_ROTATOR: ApiKeyRotator = ApiKeyRotator::new();
}

pub static BASE_URL: phf::Map<&'static str, &'static str> = phf_map! {
    "aliyuncs" => "https://dashscope.aliyuncs.com/compatible-mode/v1", // 阿里云
    "baichuan" => "https://api.baichuan-ai.com/v1", // 百川
    "baidubce" => "https://qianfan.baidubce.com/v2", // 百度云千帆
    "bigmodel" => "https://open.bigmodel.cn/api/paas/v4", //智普
    "deepseek" => "https://api.deepseek.com/v1",
    "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai", // gemini https://generativelanguage.googleapis.com/v1beta/models
    "gitee" => "https://ai.gitee.com/v1", // gitee ai
    "groq" => "https://api.groq.com/openai/v1",
    "huggingface" => "https://router.huggingface.co/hf-inference/models", // will auto add {model}/v1
    "hunyuan" => "https://api.hunyuan.cloud.tencent.com/v1", // 混元
    "lingyiwanwu" => "https://api.lingyiwanwu.com/v1", // 零一万物
    "minimax" => "https://api.minimax.chat/v1",
    "mistral" => "https://api.mistral.ai/v1",
    "modelscope" => "https://api-inference.modelscope.cn/v1", // 魔塔
    "moonshot" => "https://api.moonshot.cn/v1", // 月之暗面
    "nvidia" => "https://integrate.api.nvidia.com/v1", // NVIDIA
    "ollama" => "http://localhost:11434/v1",
    "openai" => "https://api.openai.com/v1",
    "openrouter" => "https://openrouter.ai/api/v1",
    "perplexity" => "https://api.perplexity.ai", // perplexity
    "siliconflow" => "https://api.siliconflow.cn/v1", // 硅基流动
    "stepfun" => "https://api.stepfun.com/v1", // 阶跃星辰
    "togetherai" => "https://api.together.xyz/v1",
    "volces" => "https://ark.cn-beijing.volces.com/api/v3", // 火山引擎
    "x" => "https://api.x.ai/v1",
};

/// Token usage keys
pub const TOKENS: &str = "tokens";
pub const TOKENS_TOTAL: &str = "total";
pub const TOKENS_PROMPT: &str = "prompt";
pub const TOKENS_COMPLETION: &str = "completion";
pub const TOKENS_PER_SECOND: &str = "tokensPerSecond";
