1. 前端模型切换的时候，没将当前模型的设置的最大上下文窗口和温度运用上去；
2. 前端点击 stop 后，如果处于 thinking 状态，会导致ai 返回的数据为：
[src/ai/chat/openai.rs:268:13] &token_usage = TokenUsage {
    total_tokens: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    tokens_per_second: 0.0,
}
然后触发 20:18:47.988 [W] chatspeed_lib::workflow::react::engine - src/workflow/react/engine.rs:1581 WorkflowExecutor 0pt4me7mr0400: No tool calls in response (consecutive: 2)
然后又进一步调用 llm，用户再点一次才真正停止，整个日志：
```log
20:18:47.584 [D] chatspeed_lib::workflow::react::gateway - src/workflow/react/gateway.rs:149 [Workflow Gateway] Injecting input for session 0pt4me7mr0400: {"type": "stop"}
20:18:47.976 [D] chatspeed_lib::ccproxy::helper::stat_guard - src/ccproxy/helper/stat_guard.rs:56 StreamStatGuard dropped. Recording stat: provider='Code Plan Claude-腾讯', model='minimax-m2.5', tokens=25070/0/0
[src/ai/chat/openai.rs:268:13] &token_usage = TokenUsage {
    total_tokens: 0,
    prompt_tokens: 0,
    completion_tokens: 0,
    tokens_per_second: 0.0,
}
20:18:47.988 [W] chatspeed_lib::workflow::react::engine - src/workflow/react/engine.rs:1581 WorkflowExecutor 0pt4me7mr0400: No tool calls in response (consecutive: 2)
20:18:48.096 [D] chatspeed_lib::ai::network::client - src/ai/network/client.rs:315 Request URL: http://127.0.0.1:11436/v1/chat/completions
20:18:48.098 [D] chatspeed_lib::ccproxy::auth - src/ccproxy/auth.rs:37 Internal request authenticated successfully.
20:18:48.098 [I] chatspeed_lib::ccproxy::helper::common - src/ccproxy/helper/common.rs:678 ccproxy: model=minimax-m2.5, provider=Code Plan Claude-腾讯, base_url=https://api.lkeap.cloud.tencent.com/coding/anthropic/v1, protocol=claude, selected=eAlIfB7k
20:18:48.098 [D] chatspeed_lib::ccproxy::adapter::backend::common - src/ccproxy/adapter/backend/common.rs:30 preprocess_unified_request: Starting with 1 messages
20:18:48.098 [D] chatspeed_lib::ccproxy::adapter::backend::common - src/ccproxy/adapter/backend/common.rs:129 preprocess_unified_request: Completed processing, final message count: 1
20:18:52.577 [D] chatspeed_lib::workflow::react::gateway - src/workflow/react/gateway.rs:149 [Workflow Gateway] Injecting input for session 0pt4me7mr0400: {"type": "stop"}
20:19:24.734 [D] chatspeed_lib::ccproxy::helper::stat_guard - src/ccproxy/helper/stat_guard.rs:56 StreamStatGuard dropped. Recording stat: provider='Code Plan Claude-腾讯', model='minimax-m2.5', tokens=2131/898/0
[src/ai/chat/openai.rs:268:13] &token_usage = TokenUsage {
    total_tokens: 3029,
    prompt_tokens: 2131,
    completion_tokens: 898,
    tokens_per_second: 25.303190412848416,
}
```