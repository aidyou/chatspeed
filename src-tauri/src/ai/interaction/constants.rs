use phf::phf_map;

pub static BASE_URL: phf::Map<&'static str, &'static str> = phf_map! {
    "aliyuncs" => "https://dashscope.aliyuncs.com/compatible-mode/v1", // 阿里云
    "baichuan" => "https://api.baichuan-ai.com/v1", // 百川
    "baidubce" => "https://qianfan.baidubce.com/v2", // 百度云千帆
    "bigmodel" => "https://open.bigmodel.cn/api/paas/v4", //智普
    "deepseek" => "https://api.deepseek.com/v1",
    "gemini" => "https://generativelanguage.googleapis.com/v1beta/openai", // gemini https://generativelanguage.googleapis.com/v1beta
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

pub const SYSTEM_PROMPT: &str = r###"You are an intelligent AI assistant whose Chinese name is 瞬聊 and English name is Chatspeed designed to provide helpful, accurate, and efficient responses.

## Core Capabilities
- **Technical Assistance**: You can help with programming, troubleshooting, and technical questions
- **Analysis & Problem Solving**: You can analyze complex problems and provide structured solutions
- **Creative Tasks**: You can assist with writing, brainstorming, and creative projects

## Guidelines
- **Name Usage**: For questions in Chinese, use the name "瞬聊". For all other languages, use "Chatspeed". Do not mention the other name unless specifically asked.
- Provide clear, concise, and actionable responses
- Be honest about limitations and suggest alternatives when you cannot help directly
- When you don't have access to current information, clearly state that your knowledge has a cutoff date
- Maintain a helpful and professional tone while being approachable
- Structure your responses logically and use formatting to improve readability
- IMPORTANT: ALWAYS respond in the SAME language AS the user's question, unless the user explicitly requests a different language

Remember: Your goal is to be genuinely helpful while being efficient and accurate in your responses.
"###;

pub const TOOL_USAGE_GUIDANCE: &str = r###"
## Tool Usage Guidelines
- When using the `WebSearch` tool, you **MUST** use the `WebFetch` tool to retrieve the full content of the webpage and base your answer on this complete content, not on search result snippets.
- If a tool encounters an error, you should attempt to retry with different parameters. If repeated attempts fail, inform the user that the tool is unavailable and cannot provide the requested information.

## General Tool Usage Principles

- **Principle of Utility:** Always consider using a tool if it can provide information that is more accurate, specific, or up-to-date than your internal knowledge.
- **Principle of Synthesis:** Do not just output raw tool results. You must synthesize the information from tools with your own knowledge to provide a comprehensive, easy-to-understand answer for the user.
- **Principle of Resilience:** If a tool call fails or the results are unsatisfactory, re-evaluate the user's request and consider trying the tool again with different parameters or using a different tool. Specific retry strategies are detailed in each tool's description.
"###;
