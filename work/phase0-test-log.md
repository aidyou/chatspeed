# 场景1
```log
18:04:08.821 [I] src/commands/workflow.rs:461 [Workflow][session=*** Starting workflow, agent_id=0ppah4g8m0400, planning_mode=false
18:04:08.824 [I] src/commands/workflow.rs:498 [Workflow] First message detected for session 0pz8s4fb00400, updating user_query
18:04:08.834 [I] src/workflow/react/gateway.rs:184 [Workflow][session=*** Registering signal channel
18:04:08.835 [I] src/commands/workflow.rs:604 [Workflow] Session 0pz8s4fb00400 using approval level: Default
18:04:08.958 [I] src/commands/workflow.rs:694 [Workflow][session=*** Executor registered to BACKGROUND_TASKS, spawning run_loop
18:04:08.960 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: pending -> thinking
18:04:25.134 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:04:25.135 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> completed
18:04:25.136 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: completed -> completed
```

# 场景2 ~ 3
测试过程当中刚好触发了Ask user，刷新后出现找不到Signal，不过工作流继续往下走了。写入文件的时候，我刷新了页面，它能够直接弹窗，批准后也写入成功了。

```log
18:12:04.314 [I] src/commands/workflow.rs:461 [Workflow][session=*** Starting workflow, agent_id=0ppah4g8m0400, planning_mode=false
18:12:04.315 [I] src/commands/workflow.rs:498 [Workflow] First message detected for session 0pz8v3a6r0400, updating user_query
18:12:04.325 [I] src/workflow/react/gateway.rs:184 [Workflow][session=*** Registering signal channel
18:12:04.325 [I] src/commands/workflow.rs:604 [Workflow] Session 0pz8v3a6r0400 using approval level: Default
18:12:04.427 [I] src/commands/workflow.rs:694 [Workflow][session=*** Executor registered to BACKGROUND_TASKS, spawning run_loop
18:12:04.427 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: pending -> thinking
18:12:09.815 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:12:09.877 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:12:14.728 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:12:14.780 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:12:21.834 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:12:21.835 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> awaiting_user
18:12:21.886 [I] src/workflow/react/engine.rs:848 [Workflow][session=*** Entering wait state, reason=user_input
18:12:28.868 [I] src/commands/workflow.rs:999 [Workflow][session=*** Signal received, type=update_allowed_paths
18:12:28.868 [D] src/workflow/react/gateway.rs:154 [Workflow][session=*** Injecting signal, type=update_allowed_paths
18:12:28.869 [D] src/workflow/react/gateway.rs:165 [Workflow][session=*** Signal injected successfully, type=update_allowed_paths
18:12:28.869 [I] src/commands/workflow.rs:1009 [Workflow][session=*** Signal injected successfully, type=update_allowed_paths
18:12:28.870 [I] src/workflow/react/engine.rs:863 [Workflow][session=*** Signal received, type=update_allowed_paths, wait_reason=user_input
18:12:28.871 [I] src/workflow/react/engine.rs:848 [Workflow][session=*** Entering wait state, reason=user_input
18:13:31.305 [I] src/commands/workflow.rs:461 [Workflow][session=*** Starting workflow, agent_id=0ppah4g8m0400, planning_mode=false
18:13:31.308 [I] src/workflow/react/gateway.rs:184 [Workflow][session=*** Registering signal channel
18:13:31.309 [I] src/commands/workflow.rs:604 [Workflow] Session 0pz8v3a6r0400 using approval level: Default
18:13:31.409 [I] src/commands/workflow.rs:694 [Workflow][session=*** Executor registered to BACKGROUND_TASKS, spawning run_loop
18:13:31.409 [E] src/commands/workflow.rs:713 [Workflow][session=*** Workflow error: General("Signal channel closed")

18:13:31.409 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: pending -> thinking
18:14:15.861 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:14:15.962 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:14:24.973 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:14:25.032 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:14:38.158 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:14:38.234 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:15:03.657 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:15:03.749 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:15:56.157 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:15:56.346 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:16:05.112 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:16:05.170 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> thinking
18:17:02.539 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:17:02.539 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> awaiting_approval
18:17:02.593 [I] src/workflow/react/engine.rs:848 [Workflow][session=*** Entering wait state, reason=approval
18:20:03.477 [I] src/commands/workflow.rs:999 [Workflow][session=*** Signal received, type=request_confirm_broadcast
18:20:03.477 [D] src/workflow/react/gateway.rs:154 [Workflow][session=*** Injecting signal, type=request_confirm_broadcast
18:20:03.477 [D] src/workflow/react/gateway.rs:165 [Workflow][session=*** Signal injected successfully, type=request_confirm_broadcast
18:20:03.478 [I] src/commands/workflow.rs:1009 [Workflow][session=*** Signal injected successfully, type=request_confirm_broadcast
18:20:03.478 [I] src/workflow/react/engine.rs:863 [Workflow][session=*** Signal received, type=request_confirm_broadcast, wait_reason=approval
18:20:03.482 [I] src/workflow/react/engine.rs:848 [Workflow][session=*** Entering wait state, reason=approval
18:20:16.411 [I] src/commands/workflow.rs:999 [Workflow][session=*** Signal received, type=approval
18:20:16.412 [D] src/workflow/react/gateway.rs:154 [Workflow][session=*** Injecting signal, type=approval
18:20:16.412 [D] src/workflow/react/gateway.rs:165 [Workflow][session=*** Signal injected successfully, type=approval
18:20:16.413 [I] src/commands/workflow.rs:1009 [Workflow][session=*** Signal injected successfully, type=approval
18:20:16.413 [I] src/workflow/react/engine.rs:863 [Workflow][session=*** Signal received, type=approval, wait_reason=approval
18:20:16.426 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: awaiting_approval -> thinking
18:20:16.426 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> thinking
18:20:42.750 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:20:42.751 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> completed
18:20:42.753 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: completed -> completed
```

# 场景4
下面日志空行之后，是是我点击停止按钮之后，然后我刷新页面的日志。
```log
18:32:38.159 [I] src/commands/workflow.rs:247 [Workflow][session=*** Creating workflow for agent_id=0ppah4g8m0400
18:32:38.164 [I] src/commands/workflow.rs:350 [Workflow][session=*** Workflow created successfully, agent_id=0ppah4g8m0400
18:33:23.714 [I] src/commands/workflow.rs:461 [Workflow][session=*** Starting workflow, agent_id=0ppah4g8m0400, planning_mode=false
18:33:23.732 [I] src/workflow/react/gateway.rs:184 [Workflow][session=*** Registering signal channel
18:33:23.841 [I] src/commands/workflow.rs:694 [Workflow][session=*** Executor registered to BACKGROUND_TASKS, spawning run_loop
18:33:23.842 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: pending -> thinking
18:33:39.021 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: thinking -> executing
18:33:39.023 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: executing -> awaiting_approval
18:33:39.075 [I] src/workflow/react/engine.rs:848 [Workflow][session=*** Entering wait state, reason=approval
18:34:12.687 [D] src/workflow/react/gateway.rs:154 [Workflow][session=*** Injecting signal, type=stop
18:34:12.688 [D] src/workflow/react/gateway.rs:165 [Workflow][session=*** Signal injected successfully, type=stop
18:34:12.689 [I] src/workflow/react/engine.rs:863 [Workflow][session=*** Signal received, type=stop, wait_reason=approval
18:34:12.696 [I] src/workflow/react/engine.rs:2251 [Workflow][session=*** State transition: awaiting_approval -> cancelled

18:37:07.238 [I] src/commands/workflow.rs:999 [Workflow][session=*** Signal received, type=request_confirm_broadcast
18:37:07.239 [D] src/workflow/react/gateway.rs:154 [Workflow][session=*** Injecting signal, type=request_confirm_broadcast
18:37:07.239 [W] src/commands/workflow.rs:1017 [Workflow][session=*** Signal injection failed: Gateway error: channel closed, attempting recovery
18:37:07.240 [I] src/commands/workflow.rs:1065 [Workflow] Session 0pz90021w0400 is not awaiting approval (status: cancelled), skipping confirm broadcast
```

# 场景5
在场景4的基础上，也就是点击取消之后，我刷新了页面，然后在页面当中发送消息，从日志来看应该是有返回数据的。因为TokenUsage有数据。但是奇怪的是，页面没有任何东西输出。然后我点击了继续执行按钮，也是一样，AI有反馈，但是界面没有任何东西。下面是详细的日志。
```log
18:39:20.212 [I] chatspeed_lib::workflow::react::engine - src/workflow/react/engine.rs:353 WorkflowExecutor 0pz90021w0400: Workflow was cancelled, waiting for user to resume
18:39:20.213 [I] chatspeed_lib::commands::workflow - src/commands/workflow.rs:694 [Workflow][session=*** Executor registered to BACKGROUND_TASKS, spawning run_loop
18:39:20.215 [D] chatspeed_lib::ai::network::client - src/ai/network/client.rs:315 Request URL: http://127.0.0.1:11436/v1/chat/completions
18:39:20.216 [D] chatspeed_lib::ccproxy::auth - src/ccproxy/auth.rs:37 Internal request authenticated successfully.
18:39:20.217 [I] chatspeed_lib::ccproxy::helper::common - src/ccproxy/helper/common.rs:678 ccproxy: model=glm-5, provider=Code Plan-腾讯, base_url=https://api.lkeap.cloud.tencent.com/coding/v3, protocol=openai, selected=eAlIfB7k
18:39:28.381 [D] chatspeed_lib::ccproxy::helper::stat_guard - src/ccproxy/helper/stat_guard.rs:56 StreamStatGuard dropped. Recording stat: provider='Code Plan-腾讯', model='glm-5', tokens=1840/334/0
[src/ai/chat/openai.rs:268:13] &token_usage = TokenUsage {
    total_tokens: 2174,
    prompt_tokens: 1840,
    completion_tokens: 334,
    tokens_per_second: 54.086852070804426,
}
18:40:37.692 [I] chatspeed_lib::commands::workflow - src/commands/workflow.rs:461 [Workflow][session=*** Starting workflow, agent_id=0ppah4g8m0400, planning_mode=false
18:40:37.694 [I] chatspeed_lib::workflow::react::gateway - src/workflow/react/gateway.rs:184 [Workflow][session=*** Registering signal channel
18:40:37.694 [I] chatspeed_lib::commands::workflow - src/commands/workflow.rs:604 [Workflow] Session 0pz90021w0400 using approval level: Default
18:40:37.694 [D] chatspeed_lib::workflow::react::skills - src/workflow/react/skills.rs:66 Builtin skills path from RESOURCE_DIR: "/Volumes/dev/personal/dev/ai/chatspeed-workflow-refactor/src-tauri/assets/skills"
18:40:37.697 [W] chatspeed_lib::workflow::react::security - src/workflow/react/security.rs:68 [PathGuard] Path does not exist and was ignored: "/Users/xc/Library/Application Support/ai.aidyou.chatspeed/planning/0pz90021w0400"
18:40:37.739 [I] chatspeed_lib::workflow::react::engine - src/workflow/react/engine.rs:353 WorkflowExecutor 0pz90021w0400: Workflow was cancelled, waiting for user to resume
18:40:37.739 [I] chatspeed_lib::commands::workflow - src/commands/workflow.rs:694 [Workflow][session=*** Executor registered to BACKGROUND_TASKS, spawning run_loop
18:40:37.740 [D] chatspeed_lib::ai::network::client - src/ai/network/client.rs:315 Request URL: http://127.0.0.1:11436/v1/chat/completions
18:40:37.741 [D] chatspeed_lib::ccproxy::auth - src/ccproxy/auth.rs:37 Internal request authenticated successfully.
18:40:37.742 [I] chatspeed_lib::ccproxy::helper::common - src/ccproxy/helper/common.rs:678 ccproxy: model=minimax-m2.5, provider=Code Plan-腾讯, base_url=https://api.lkeap.cloud.tencent.com/coding/v3, protocol=openai, selected=eAlIfB7k
18:40:45.028 [D] chatspeed_lib::ccproxy::helper::stat_guard - src/ccproxy/helper/stat_guard.rs:56 StreamStatGuard dropped. Recording stat: provider='Code Plan-腾讯', model='minimax-m2.5', tokens=1809/304/0
[src/ai/chat/openai.rs:268:13] &token_usage = TokenUsage {
    total_tokens: 2113,
    prompt_tokens: 1809,
    completion_tokens: 304,
    tokens_per_second: 48.55189287999068,
}
```