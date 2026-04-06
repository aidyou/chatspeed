# 场景1
前端界面浏览器控制台看到的日志：
```log
[Log] Sending message to workflow: – "帮我测试write_file工具" (useWorkflowCore.ts, line 367)
[Log] [Workflow][state] State changed: – {from: "pending", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
{from: "pending", to: "thinking", wait_reason: null, prevWaitReason: null}Object
[Log] [Workflow][state] State changed: – {from: "thinking", to: "executing", wait_reason: null, …} (useWorkflowCore.ts, line 159)
{from: "thinking", to: "executing", wait_reason: null, prevWaitReason: null}Object
[Log] [Workflow][state] State changed: – {from: "executing", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
{from: "executing", to: "thinking", wait_reason: null, prevWaitReason: null}Object
[Log] [Workflow][state] State changed: – {from: "thinking", to: "executing", wait_reason: null, …} (useWorkflowCore.ts, line 159)
{from: "thinking", to: "executing", wait_reason: null, prevWaitReason: null}Object
[Log] [Workflow][state] State changed: – {from: "executing", to: "awaiting_approval", wait_reason: "approval", …} (useWorkflowCore.ts, line 159)
{from: "executing", to: "awaiting_approval", wait_reason: "approval", prevWaitReason: null}Object
[Log] [Workflow][isAwaitingApproval] Detected by wait_reason=approval (useWorkflowCore.ts, line 58)
```

Rust端关键的日志:
```log
14:49:17.965 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> executing
14:49:17.967 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: executing -> awaiting_approval
14:49:17.969 [I] src/db/workflow.rs:402 [Workflow][session=***] snapshot.write - state=Waiting, wait_reason=Some(Approval), pending_tools=1
14:49:17.969 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Waiting, wait_reason=Some(Approval), pending_tools=1
14:49:18.021 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Approval)
```

# 场景2
在审核工具页面，右键刷新以后，通过临时按钮关闭审核对话框，然后发送用户消息。

前端控台的日志：
```log
[Log] [Workflow] Requesting confirm broadcast for workflow in awaiting_approval state with pending approval (useWorkflowCore.ts, line 239)
[Log] [Workflow] Confirm broadcast request sent successfully (useWorkflowCore.ts, line 245)
[Log] Scrolling to bottom after switching workflow (useWorkflowCore.ts, line 269)
[Log] pin state: – false – " window label:" – "workflow" (window.js, line 117)
[Log] Sending message to workflow: – "test" (useWorkflowCore.ts, line 367)
[Log] Signal sent successfully: – "Signal injected" (useWorkflowCore.ts, line 386)
```

后端开发编辑器的日志：
```log
19:10:09.933 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=unknown
19:10:09.936 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:10:09.937 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:10:09.938 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:10:09.940 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:10:09.941 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=request_confirm_broadcast
19:10:09.943 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=request_confirm_broadcast
19:10:09.943 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=unknown
19:10:09.944 [I] src/workflow/react/engine.rs:991 [Workflow][session=***][phase=wait] Signal received, type=request_confirm_broadcast, wait_reason=Some(Approval)
19:10:09.944 [D] src/workflow/react/engine.rs:1000 WorkflowExecutor 0pzvzspfg0400: Received signal while awaiting_approval: type=request_confirm_broadcast, has_content=false, content=<none>
19:10:09.945 [I] src/workflow/react/engine.rs:1071 WorkflowExecutor 0pzvzspfg0400: Received request to re-broadcast pending confirmations
19:10:09.945 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Approval)
19:10:25.918 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=user_message
19:10:25.919 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:10:25.920 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:10:25.920 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'user_message' routed successfully
19:10:25.921 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'user_message' routed successfully
19:10:25.921 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=user_message

19:10:25.922 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=user_message
19:10:25.922 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=user_message
19:10:25.923 [W] src/workflow/react/engine.rs:968 [Workflow][session=***][phase=wait][event=signal_rejected] Signal 'user_message' is not valid for wait_reason Some(Approval)
19:10:25.923 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Approval)
```

# 场景3
场景3的测试相对简单，在场景2的基础上，刷新页面后正常审批，整个流程让它走完即可。

前段浏览器控台日志：
```log
[Log] workflowStore: selecting workflow – "0pzvzspfg0400" (workflow.js, line 155)
[Log] workflowStore: snapshot loaded – {messages: Array, workflow: Object} (workflow.js, line 160)
[Log] [Workflow] selectWorkflow completed, currentWorkflow: – "0pzvzspfg0400" – "status:" – "awaiting_approval" (useWorkflowCore.ts, line 225)
[Log] [Workflow] Checking status for confirm broadcast: – "awaiting_approval" – "workflow:" – "0pzvzspfg0400" – "hasPendingApproval:" – true (useWorkflowCore.ts, line 237)
[Log] [Workflow] Requesting confirm broadcast for workflow in awaiting_approval state with pending approval (useWorkflowCore.ts, line 239)
[Log] [Workflow] Confirm broadcast request sent successfully (useWorkflowCore.ts, line 245)
[Log] [Workflow][state] State changed: – {from: "awaiting_approval", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "executing", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "executing", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "executing", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "executing", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "executing", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "executing", to: "completed", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "completed", to: "completed", wait_reason: null, …} (useWorkflowCore.ts, line 159)
```

后端日志：
```log
19:16:29.319 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=unknown
19:16:29.319 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:16:29.320 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:16:29.321 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:16:29.322 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:16:29.323 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=request_confirm_broadcast
19:16:29.324 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=request_confirm_broadcast
19:16:29.326 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=unknown
19:16:29.327 [I] src/workflow/react/engine.rs:991 [Workflow][session=***][phase=wait] Signal received, type=request_confirm_broadcast, wait_reason=Some(Approval)
19:16:29.328 [D] src/workflow/react/engine.rs:1000 WorkflowExecutor 0pzvzspfg0400: Received signal while awaiting_approval: type=request_confirm_broadcast, has_content=false, content=<none>
19:16:29.329 [I] src/workflow/react/engine.rs:1071 WorkflowExecutor 0pzvzspfg0400: Received request to re-broadcast pending confirmations
19:16:29.330 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Approval)
19:16:34.747 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=unknown
19:16:34.748 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:16:34.748 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:16:34.748 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:16:34.748 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:16:34.748 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=approval
19:16:34.749 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=approval
19:16:34.749 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=unknown
19:16:34.750 [I] src/workflow/react/engine.rs:991 [Workflow][session=***][phase=wait] Signal received, type=approval, wait_reason=Some(Approval)
19:16:34.750 [D] src/workflow/react/engine.rs:1000 WorkflowExecutor 0pzvzspfg0400: Received signal while awaiting_approval: type=approval, has_content=false, content=<none>
19:16:34.750 [I] src/workflow/react/engine.rs:1131 WorkflowExecutor 0pzvzspfg0400: User APPROVED tool 'write_file' (ID: tool_973f1303)
19:16:34.752 [I] src/workflow/react/engine.rs:2397 [Workflow][session=***][phase=wait][event=resume] Resuming from awaiting_approval to thinking
19:16:34.753 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: awaiting_approval -> thinking
19:16:34.754 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> thinking
19:17:03.046 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> executing
19:17:03.100 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: executing -> thinking
19:17:06.506 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> executing
19:17:06.575 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: executing -> thinking
19:17:10.623 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> executing
19:17:10.624 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: executing -> completed
19:17:10.626 [I] src/db/workflow.rs:402 [Workflow][session=***] snapshot.write - state=Completed, wait_reason=None, pending_tools=0
19:17:10.626 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Completed, wait_reason=None, pending_tools=0
19:17:10.627 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: completed -> completed
19:17:10.628 [I] src/db/workflow.rs:402 [Workflow][session=***] snapshot.write - state=Completed, wait_reason=None, pending_tools=0
19:17:10.629 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Completed, wait_reason=None, pending_tools=0
19:17:12.067 [I] src/workflow/react/manager.rs:105 [WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

# 场景4
前端浏览器控制台测试日志:
```log
[Log] [Workflow] selectWorkflow completed, currentWorkflow: – "0pzvzspfg0400" – "status:" – "awaiting_user" (useWorkflowCore.ts, line 225)
[Log] [Workflow] Checking status for confirm broadcast: – "awaiting_user" – "workflow:" – "0pzvzspfg0400" – "hasPendingApproval:" – false (useWorkflowCore.ts, line 237)
[Log] [Workflow] Not requesting confirm broadcast, status is: – "awaiting_user" – "hasPendingApproval:" – false (useWorkflowCore.ts, line 250)
[Log] Scrolling to bottom after switching workflow (useWorkflowCore.ts, line 269)
[Log] pin state: – false – " window label:" – "workflow" (window.js, line 117)
[Log] Continue signal sent successfully (useWorkflowCore.ts, line 448)
```

后端日志:r
```log
19:30:40.354 [I] src/workflow/react/engine.rs:991 [Workflow][session=***][phase=wait] Signal received, type=update_finalAudit, wait_reason=Some(UserInput)
19:30:40.355 [D] src/workflow/react/engine.rs:1000 WorkflowExecutor 0pzvzspfg0400: Received signal while awaiting_user: ***, has_content=false, content=<none>
19:30:40.355 [W] src/workflow/react/engine.rs:1392 WorkflowExecutor 0pzvzspfg0400: Received empty user input, continuing to wait
19:30:40.355 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(UserInput)
19:30:49.370 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=continue
19:30:49.372 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:30:49.373 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:30:49.374 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'continue' routed successfully
19:30:49.374 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'continue' routed successfully
19:30:49.375 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=continue
19:30:49.376 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=continue
19:30:49.377 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=continue
19:30:49.379 [W] src/workflow/react/engine.rs:968 [Workflow][session=***][phase=wait][event=signal_rejected] Signal 'continue' is not valid for wait_reason Some(UserInput)
19:30:49.380 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(UserInput)
```

# 场景5
前端浏览器控台日志：
```log
[Log] [Workflow] selectWorkflow completed, currentWorkflow: – "0pzvzspfg0400" – "status:" – "awaiting_user" (useWorkflowCore.ts, line 225)
[Log] [Workflow] Checking status for confirm broadcast: – "awaiting_user" – "workflow:" – "0pzvzspfg0400" – "hasPendingApproval:" – false (useWorkflowCore.ts, line 237)
[Log] [Workflow] Not requesting confirm broadcast, status is: – "awaiting_user" – "hasPendingApproval:" – false (useWorkflowCore.ts, line 250)
[Log] Sending message to workflow: – "很好，测试通过" (useWorkflowCore.ts, line 367)
[Log] Signal sent successfully: – "Signal injected" (useWorkflowCore.ts, line 386)
[Log] [Workflow][state] State changed: – {from: "awaiting_user", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "thinking", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "thinking", to: "executing", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "executing", to: "completed", wait_reason: null, …} (useWorkflowCore.ts, line 159)
[Log] [Workflow][state] State changed: – {from: "completed", to: "completed", wait_reason: null, …} (useWorkflowCore.ts, line 159)
```

后端测试日志:
```log
19:35:18.775 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=unknown
19:35:18.775 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:35:18.776 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:35:18.776 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:35:18.776 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'unknown' routed successfully
19:35:18.776 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=update_finalAudit
19:35:18.776 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=update_finalAudit
19:35:18.777 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=unknown
19:35:18.777 [I] src/workflow/react/engine.rs:991 [Workflow][session=***][phase=wait] Signal received, type=update_finalAudit, wait_reason=Some(UserInput)
19:35:18.777 [D] src/workflow/react/engine.rs:1000 WorkflowExecutor 0pzvzspfg0400: Received signal while awaiting_user: ***, has_content=false, content=<none>
19:35:18.778 [W] src/workflow/react/engine.rs:1392 WorkflowExecutor 0pzvzspfg0400: Received empty user input, continuing to wait
19:35:18.778 [I] src/workflow/react/engine.rs:953 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(UserInput)
19:35:26.177 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=user_message
19:35:26.178 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
19:35:26.179 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
19:35:26.180 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'user_message' routed successfully
19:35:26.181 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'user_message' routed successfully
19:35:26.182 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=user_message
19:35:26.183 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=user_message
19:35:26.184 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=user_message
19:35:26.185 [I] src/workflow/react/engine.rs:978 [Workflow][session=***][phase=wait][event=signal_received] Signal 'user_message' accepted for wait_reason Some(UserInput)
19:35:26.187 [I] src/workflow/react/engine.rs:991 [Workflow][session=***][phase=wait] Signal received, type=user_message, wait_reason=Some(UserInput)
19:35:26.188 [D] src/workflow/react/engine.rs:1000 WorkflowExecutor 0pzvzspfg0400: Received signal while awaiting_user: ***, has_content=true, content=很好，测试通过
19:35:26.189 [I] src/workflow/react/engine.rs:2563 WorkflowExecutor 0pzvzspfg0400: User message received while AwaitingUser, transitioning to Thinking
19:35:26.190 [I] src/workflow/react/engine.rs:2397 [Workflow][session=***][phase=wait][event=resume] Resuming from awaiting_user to thinking
19:35:26.191 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: awaiting_user -> thinking
19:35:26.196 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> thinking
19:35:26.196 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> thinking
19:35:30.423 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: thinking -> executing
19:35:30.425 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: executing -> completed
19:35:30.426 [I] src/db/workflow.rs:402 [Workflow][session=***] snapshot.write - state=Completed, wait_reason=None, pending_tools=0
19:35:30.427 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Completed, wait_reason=None, pending_tools=0
19:35:30.428 [I] src/workflow/react/engine.rs:2405 [Workflow][session=***][phase=state] State transition: completed -> completed
19:35:30.429 [I] src/db/workflow.rs:402 [Workflow][session=***] snapshot.write - state=Completed, wait_reason=None, pending_tools=0
19:35:30.429 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Completed, wait_reason=None, pending_tools=0
19:35:31.789 [I] src/workflow/react/manager.rs:105 [WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

# 场景6
```log
22:24:11.481 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=unknown
22:24:11.482 [D] src/workflow/react/manager.rs:129 [WorkflowManager][session=***][event=session_lookup_miss] Session not found
22:24:11.482 [I] src/commands/workflow.rs:1109 [WorkflowManager][session=***][event=session_lookup_miss] Session not found in manager, entering recovery
22:25:19.653 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=continue
22:25:19.654 [D] src/workflow/react/manager.rs:129 [WorkflowManager][session=***][event=session_lookup_miss] Session not found
22:25:19.654 [I] src/commands/workflow.rs:1109 [WorkflowManager][session=***][event=session_lookup_miss] Session not found in manager, entering recovery
22:25:19.656 [I] src/commands/workflow.rs:1203 [Workflow] Session 0pzz5zyc40400 is paused, resuming to process "continue" signal
22:25:19.656 [I] src/commands/workflow.rs:463 [Workflow][session=***][phase=start] Starting workflow, agent_id=0ppah4g8m0400, planning_mode=false
22:25:19.656 [D] src/workflow/react/manager.rs:129 [WorkflowManager][session=***][event=session_lookup_miss] Session not found
22:25:19.660 [I] src/workflow/react/gateway.rs:184 [Workflow][session=***][phase=gateway] Registering signal channel
22:25:19.661 [I] src/commands/workflow.rs:617 [Workflow] Session 0pzz5zyc40400 using approval level: Default
22:25:19.674 [I] src/workflow/react/engine.rs:381 WorkflowExecutor 0pzz5zyc40400: Workflow was paused, waiting for user to resume
22:25:19.675 [I] src/workflow/react/manager.rs:92 [WorkflowManager][session=***][event=session_registered] Session registered with status Active
22:25:19.675 [I] src/commands/workflow.rs:722 [Workflow][session=***][phase=start] Executor registered to WorkflowManager (primary) and BACKGROUND_TASKS (compat), spawning run_loop
22:25:19.676 [I] src/workflow/react/engine.rs:964 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Confirmation)
22:25:19.777 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=continue
22:25:19.779 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=continue
22:25:19.779 [I] src/commands/workflow.rs:1230 [Workflow] "continue" signal injected successfully after retry
22:25:19.779 [I] src/workflow/react/engine.rs:989 [Workflow][session=***][phase=wait][event=signal_received] Signal 'continue' accepted for wait_reason Some(Confirmation)
22:25:19.779 [I] src/workflow/react/engine.rs:1002 [Workflow][session=***][phase=wait] Signal received, type=continue, wait_reason=Some(Confirmation)
22:25:19.780 [D] src/workflow/react/engine.rs:1011 WorkflowExecutor 0pzz5zyc40400: Received signal while paused: type=continue, has_content=false, content=<none>
22:25:19.780 [I] src/workflow/react/engine.rs:1396 [Workflow][session=***][phase=wait][event=signal_received] Continue signal accepted for wait_reason Some(Confirmation)
22:25:19.780 [I] src/workflow/react/engine.rs:2393 [Workflow][session=***][phase=wait][event=resume] Resuming from paused to thinking
22:25:19.780 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: paused -> thinking
22:25:19.782 [I] src/workflow/react/engine.rs:1526 [Workflow][session=***][step] Step 1/6, approval_level=Default
22:25:19.782 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: thinking -> thinking
22:25:23.030 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: thinking -> executing22:25:23.033 [I] src/workflow/react/engine.rs:2361 WorkflowExecutor 0pzz5zyc40400: Auto-approved tool 'read_file' in Default (auto_approve list) mode
22:25:23.094 [I] src/workflow/react/engine.rs:1526 [Workflow][session=***][step] Step 2/6, approval_level=Default
22:25:27.273 [I] src/workflow/react/engine.rs:1526 [Workflow][session=***][step] Step 3/6, approval_level=Default
22:25:31.955 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: thinking -> executing
22:25:32.011 [I] src/workflow/react/engine.rs:1526 [Workflow][session=***][step] Step 4/6, approval_level=Default
22:25:36.235 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: thinking -> executing22:25:36.238 [I] src/workflow/react/engine.rs:2361 WorkflowExecutor 0pzz5zyc40400: Auto-approved tool 'read_file' in Default (auto_approve list) mode
22:25:36.295 [I] src/workflow/react/engine.rs:1526 [Workflow][session=***][step] Step 5/6, approval_level=Default
22:25:41.178 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: thinking -> executing22:25:41.182 [I] src/workflow/react/engine.rs:2361 WorkflowExecutor 0pzz5zyc40400: Auto-approved tool 'read_file' in Default (auto_approve list) mode
22:25:41.237 [I] src/workflow/react/engine.rs:1526 [Workflow][session=***][step] Step 6/6, approval_level=Default
22:25:49.345 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=stop
22:25:49.345 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=stop
22:25:56.571 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: thinking -> executing22:25:56.572 [I] src/workflow/react/engine.rs:2669 WorkflowExecutor 0pzz5zyc40400: Stop signal detected, cancelling workflow
22:25:56.572 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: executing -> cancelled
22:25:56.574 [I] src/db/workflow.rs:414 [Workflow][session=***] snapshot.write - state=Cancelled, wait_reason=None, pending_tools=0
22:25:56.574 [I] src/workflow/react/engine.rs:2982 [Workflow][session=***][phase=snapshot] Saved: state=Cancelled, wait_reason=None, pending_tools=0
22:25:56.574 [I] src/workflow/react/engine.rs:1736 WorkflowExecutor 0pzz5zyc40400: User cancelled operation: 操作已被用户取消
22:25:56.575 [I] src/workflow/react/engine.rs:2401 [Workflow][session=***][phase=state] State transition: cancelled -> cancelled
22:25:56.576 [I] src/db/workflow.rs:414 [Workflow][session=***] snapshot.write - state=Cancelled, wait_reason=None, pending_tools=0
22:25:56.576 [I] src/workflow/react/engine.rs:2982 [Workflow][session=***][phase=snapshot] Saved: state=Cancelled, wait_reason=None, pending_tools=0
22:25:58.177 [I] src/workflow/react/manager.rs:105 [WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

# 场景7
```log
22:42:48.182 [I] src/workflow/react/engine.rs:2982 [Workflow][session=***][phase=snapshot] Saved: state=Waiting, wait_reason=Some(Confirmation), pending_tools=0
22:42:48.183 [I] src/workflow/react/engine.rs:964 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Confirmation)
22:42:58.406 [I] src/commands/workflow.rs:1054 [Workflow][session=***][phase=signal] Signal received, type=user_message
22:42:58.406 [D] src/workflow/react/manager.rs:124 [WorkflowManager][session=***][event=session_lookup_hit] Session exists
22:42:58.407 [I] src/commands/workflow.rs:1065 [WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
22:42:58.407 [I] src/workflow/react/manager.rs:184 [WorkflowManager][session=***][event=signal_routed] Signal 'user_message' routed successfully
22:42:58.407 [I] src/commands/workflow.rs:1081 [WorkflowManager][session=***][event=signal_routed] Signal 'user_message' routed successfully
22:42:58.407 [D] src/workflow/react/gateway.rs:154 [Workflow][session=***][phase=gateway] Injecting signal, type=user_message
22:42:58.407 [D] src/workflow/react/gateway.rs:165 [Workflow][session=***][phase=gateway] Signal injected successfully, type=user_message
22:42:58.408 [I] src/commands/workflow.rs:1090 [Workflow][session=***][phase=signal] Signal injected successfully, type=user_message
22:42:58.408 [W] src/workflow/react/engine.rs:979 [Workflow][session=***][phase=wait][event=signal_rejected] Signal 'user_message' is not valid for wait_reason Some(Confirmation)
22:42:58.408 [I] src/workflow/react/engine.rs:964 [Workflow][session=***][phase=wait][event=enter] Entering wait state, reason=Some(Confirmation)
```

# 场景8
```log
23:04:05.337 [I] src/workflow/react/engine.rs:986 [Workflow][session=***][phase=wait][event=signal_received] Signal 'stop' accepted for wait_reason Some(UserInput)
23:04:05.337 [I] src/workflow/react/engine.rs:999 [Workflow][session=***][phase=wait] Signal received, type=stop, wait_reason=Some(UserInput)
23:04:05.338 [D] src/workflow/react/engine.rs:1008 WorkflowExecutor 0pzzh2qvw0400: Received signal while awaiting_user: ***, has_content=false, content=<none>
23:04:05.338 [I] src/workflow/react/engine.rs:1069 [Workflow][session=***][phase=wait][event=stop] Stop signal received in waiting state
23:04:05.338 [I] src/workflow/react/engine.rs:2398 [Workflow][session=***][phase=state] State transition: awaiting_user -> cancelled
23:04:05.339 [I] src/db/workflow.rs:414 [Workflow][session=***] snapshot.write - state=Cancelled, wait_reason=None, pending_tools=0
23:04:05.339 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Cancelled, wait_reason=None, pending_tools=0
23:04:05.339 [I] src/workflow/react/manager.rs:105 [WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

# 场景9
```log
23:21:38.847 [I] src/workflow/react/engine.rs:986 [Workflow][session=***][phase=wait][event=signal_received] Signal 'stop' accepted for wait_reason Some(Approval)
23:21:38.849 [I] src/workflow/react/engine.rs:999 [Workflow][session=***][phase=wait] Signal received, type=stop, wait_reason=Some(Approval)
23:21:38.850 [D] src/workflow/react/engine.rs:1008 WorkflowExecutor 0pzzh2qvw0400: Received signal while awaiting_approval: type=stop, has_content=false, content=<none>
23:21:38.851 [I] src/workflow/react/engine.rs:1069 [Workflow][session=***][phase=wait][event=stop] Stop signal received in waiting state
23:21:38.852 [I] src/workflow/react/engine.rs:2398 [Workflow][session=***][phase=state] State transition: awaiting_approval -> cancelled
23:21:38.856 [I] src/db/workflow.rs:414 [Workflow][session=***] snapshot.write - state=Cancelled, wait_reason=None, pending_tools=1
23:21:38.856 [I] src/workflow/react/engine.rs:2979 [Workflow][session=***][phase=snapshot] Saved: state=Cancelled, wait_reason=None, pending_tools=1
23:21:38.857 [I] src/workflow/react/manager.rs:105 [WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

# 场景10
ask_user工具的审批日志
```log
[Log] [Workflow][state] thinking -> executing | wait_reason: null | isWaiting: false (useWorkflowCore.ts, line 160)
[Log] [Workflow][state] executing -> awaiting_user | wait_reason: user_input | isWaiting: true (useWorkflowCore.ts, line 160)
```
审批工具的日志:
```log
[Log] [Workflow][state] executing -> awaiting_approval | wait_reason: approval | isWaiting: true (useWorkflowCore.ts, line 160)
[Log] [Workflow][isAwaitingApproval] Detected by wait_reason=approval (useWorkflowCore.ts, line 58)
```
