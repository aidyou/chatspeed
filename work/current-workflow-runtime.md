# 工作流运行时架构文档

> 生成时间：2025-04-03
> 目的：记录当前运行时关键路径和日志锚点，便于调试和阶段0验收

---

## 1. 关键运行时路径

### 1.1 Workflow 创建路径

```
Frontend                     Backend (commands/workflow.rs)          Database
    │                              │                                   │
    │── create_workflow ───────>│                                   │
    │                              │── generate session_id (TSID)     │
    │                              │── log: [Workflow][session=...][phase=create]
    │                              │── build AgentConfig               │
    │                              │── store.create_workflow() ───────>│
    │                              │── log: [Workflow][session=...][phase=create]
    │<── return session_id ──────│                                   │
```

**日志锚点：**
- `[Workflow][session={id}][phase=create] Creating workflow for agent_id={agent_id}`
- `[Workflow][session={id}][phase=create] Workflow created successfully`

---

### 1.2 Workflow 启动路径

```
Frontend                     Backend (commands/workflow.rs)          Engine
    │                              │                                   │
    │── workflow_start ──────────>│                                   │
    │                              │── log: [Workflow][session=...][phase=start]
    │                              │── load snapshot & agent config      │
    │                              │── create signal channel (100 buf)   │
    │                              │── gateway.register_session_tx() ───>│
    │                              │                                   │── log: [Workflow][session=...][phase=gateway]
    │                              │── create executor                  │
    │                              │── executor.init()                  │
    │                              │── BACKGROUND_TASKS.insert()        │
    │                              │── log: [Workflow][session=...][phase=start]
    │                              │── tokio::spawn(run_loop) ─────────>│
    │                              │                                   │── update_state(Thinking)
    │                              │                                   │── log: [Workflow][session=...][phase=state]
    │<── return session_id ──────│                                   │
```

**日志锚点：**
- `[Workflow][session={id}][phase=start] Starting workflow, agent_id={agent_id}, planning_mode={bool}`
- `[Workflow][session={id}][phase=gateway] Registering input sender`
- `[Workflow][session={id}][phase=start] Executor registered to BACKGROUND_TASKS, spawning run_loop`
- `[Workflow][session={id}][phase=state] State transition: Pending -> Thinking`

---

### 1.3 信号注入路径

```
Frontend                     Backend (commands/workflow.rs)          Gateway
    │                              │                                   │
    │── workflow_signal ─────────>│                                   │
    │                              │── parse signal_type               │
    │                              │── log: [Workflow][session=...][phase=signal]
    │                              │── gateway.inject_input() ─────────>│
    │                              │                                   │── find sender in map
    │                              │                                   │── log: [Workflow][session=...][phase=gateway]
    │                              │                                   │── tx.send(signal)
    │                              │<── Ok/Err ─────────────────────────│
    │                              │── log: success/failure             │
    │<── return result ───────────│                                   │
```

**日志锚点：**
- `[Workflow][session={id}][phase=signal] Signal received, type={type}`
- `[Workflow][session={id}][phase=gateway] Injecting signal, type={type}`
- `[Workflow][session={id}][phase=signal] Signal injected successfully` (成功)
- `[Workflow][session={id}][phase=signal] Signal injection failed: {error}` (失败)

---

### 1.4 等待状态转换路径

```
Engine (engine.rs)
    │
    │── update_state(AwaitingApproval/AwaitingUser/Paused)
    │── log: [Workflow][session=...][phase=state]
    │── log: [Workflow][session=...][phase=wait] Entering wait state
    │
    │=== signal_rx.recv() (阻塞等待) ===
    │
    │── log: [Workflow][session=...][phase=wait] Signal received
    │── process signal (approval/user_input/stop)
    │── update_state(Thinking)
    │── log: [Workflow][session=...][phase=state]
```

**日志锚点：**
- `[Workflow][session={id}][phase=state] State transition: {old} -> {new}`
- `[Workflow][session={id}][phase=wait] Entering wait state, reason={reason}`
- `[Workflow][session={id}][phase=wait] Signal received, type={type}, wait_reason={reason}`

---

### 1.5 恢复路径（页面刷新后）

```
Frontend                     Backend (commands/workflow.rs)          Engine
    │                              │                                   │
    │── workflow_signal ─────────>│                                   │
    │                              │── inject_input fails (no channel)  │
    │                              │── log: Signal injection failed     │
    │                              │                                   │
    │                              │── check signal type                │
    │                              │── if user_input:                   │
    │                              │── workflow_start(resume) ─────────>│
    │                              │                                   │── engine.init()
    │                              │                                   │── check state == AwaitingApproval
    │                              │                                   │── log: Re-broadcasting pending approvals
    │                              │                                   │── rebuild pending_approvals from messages
    │                              │                                   │── send Confirm to UI
```

**日志锚点：**
- `[Workflow][session={id}][phase=signal] Signal injection failed, attempting recovery`
- `[Workflow][session={id}][phase=init] Re-broadcasting pending approvals to UI`

---

### 1.6 引擎崩溃路径

```
Engine (engine.rs)                 Backend (commands/workflow.rs)
    │                                   │
    │── run_loop() returns Err          │
    │── log: [Workflow][session=...][phase=run_loop][event=crash]
    │── send Error state to UI          │
    │                                   │
    │── BACKGROUND_TASKS.remove()       │
```

**日志锚点：**
- `[Workflow][session={id}][phase=run_loop][event=crash] Workflow error: {error}`
- `[Workflow][session={id}][phase=run_loop][event=cancelled] Workflow session was cancelled`

---

## 2. 日志搜索模式

### 2.1 追踪特定 session 的完整生命周期

```bash
# macOS/Linux
grep "\[Workflow\]\[session=ABC123\]" app.log

# Windows PowerShell
Select-String "\[Workflow\]\[session=ABC123\]" app.log
```

### 2.2 查找所有状态转换

```bash
grep "\[phase=state\]" app.log
```

### 2.3 查找所有信号注入

```bash
grep "\[phase=signal\]" app.log
```

### 2.4 查找等待点

```bash
grep "\[phase=wait\]" app.log
```

### 2.5 查找崩溃

```bash
grep "\[event=crash\]" app.log
```

---

## 3. 当前问题点

### 3.1 信号通道关闭

**现象：** `Signal channel closed` 错误

**可能原因：**
1. 页面刷新后，executor 已不在内存中
2. BACKGROUND_TASKS 已移除，但 UI 仍尝试发送信号
3. 子代理的 signal_tx 被立即丢弃（orchestrator.rs:74）

**调试：**
```bash
grep "No input channel found" app.log
grep "Signal injection failed" app.log
```

### 3.2 Approval 恢复依赖消息解析

**现象：** 页面刷新后 approval 请求丢失

**当前机制：**
- 从 `workflow_messages` 扫描 "Awaiting approval" 文本
- 从 assistant 消息中提取 tool_call_id
- 重新插入 `pending_approvals`

**问题：** 非结构化恢复，易出错

**调试：**
```bash
grep "Re-broadcasting pending approvals" app.log
grep "Restored and Re-notifying UI for tool" app.log
```

---

## 4. 阶段0验收标准

### 4.1 必须能通过日志回答的问题

1. **创建流程：** 谁创建了 workflow？agent_id 是什么？
   - 搜索：`[phase=create]`

2. **启动流程：** 何时启动？是否成功注册到 BACKGROUND_TASKS？
   - 搜索：`[phase=start]`

3. **状态转换：** 状态如何流转？
   - 搜索：`[phase=state]`

4. **等待点：** 何时进入等待？等待什么？收到什么信号？
   - 搜索：`[phase=wait]`

5. **信号路由：** 信号是否成功注入？失败后是否进入恢复？
   - 搜索：`[phase=signal]`

6. **崩溃点：** 是否有引擎崩溃？错误是什么？
   - 搜索：`[event=crash]`

### 4.2 验收通过标准

- [ ] 创建 workflow 后能看到 `[phase=create]` 日志
- [ ] 启动 workflow 后能看到 `[phase=start]` 日志
- [ ] 状态变化后能看到 `[phase=state]` 日志
- [ ] 进入等待后能看到 `[phase=wait] Entering wait state` 日志
- [ ] 发送信号后能看到 `[phase=signal]` 日志
- [ ] 引擎崩溃后能看到 `[event=crash]` 日志
