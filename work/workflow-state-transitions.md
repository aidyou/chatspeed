# WorkflowState 状态流转表

> 生成时间：2025-04-03
> 目的：记录当前状态机行为，为阶段0日志增强和后续重构提供参考

---

## 1. WorkflowState 枚举定义

**位置**: `src-tauri/src/workflow/react/types.rs:7-19`

```rust
pub enum WorkflowState {
    Pending,           // 初始状态
    Thinking,          // LLM 思考中
    Executing,         // 执行工具中
    Auditing,          // 最终审计中
    Paused,            // 暂停（未使用）
    AwaitingUser,      // 等待用户输入
    AwaitingApproval,  // 等待工具审批
    AwaitingAutoApproval, // Full 模式自动审批过渡态
    Completed,         // 完成
    Error,             // 错误
    Cancelled,         // 取消
}
```

---

## 2. 状态转换矩阵

### 2.1 从 Pending 开始

| 源状态 | 目标状态 | 触发点 | 文件:行号 |
|--------|----------|--------|-----------|
| Pending | Thinking | `init_internal()` 初始化完成 | engine.rs:351 |

### 2.2 Thinking 状态转换

| 源状态 | 目标状态 | 触发点 | 文件:行号 |
|--------|----------|--------|-----------|
| Thinking | Executing | 工具执行开始 | engine.rs:1576 |
| Thinking | AwaitingApproval | 工具需要审批 | interceptors.rs:151, 444 |
| Thinking | AwaitingAutoApproval | Full 模式提交计划 | interceptors.rs:146 |
| Thinking | AwaitingUser | ask_user 工具调用 | interceptors.rs:178 |
| Thinking | Auditing | finish_task 且启用审计 | interceptors.rs:249 |
| Thinking | Completed | finish_task 无需审计 | interceptors.rs:267, engine.rs:1418, 1681 |
| Thinking | Cancelled | stop 信号 | engine.rs:916, 1588, 2449 |
| Thinking | Thinking | 信号处理后恢复 | engine.rs:1217, 1233, 1318, 1325, 1367, 1411, 1451, 2338 |

### 2.3 Executing 状态转换

| 源状态 | 目标状态 | 触发点 | 文件:行号 |
|--------|----------|--------|-----------|
| Executing | Thinking | 工具执行完成 | engine.rs:2338 |

### 2.4 等待状态转换

| 源状态 | 目标状态 | 触发点 | 文件:行号 |
|--------|----------|--------|-----------|
| AwaitingApproval | Thinking | approval 信号（批准/拒绝） | engine.rs:1217 |
| AwaitingApproval | Cancelled | stop 信号 | engine.rs:916 |
| AwaitingUser | Thinking | user_input 信号 | engine.rs:1318 |
| AwaitingUser | Paused | continue 信号（暂未实现完整流程） | engine.rs:1367 |
| AwaitingAutoApproval | Thinking | 自动批准过渡 | interceptors.rs:146 → 引擎循环处理 |

### 2.5 终态

| 源状态 | 目标状态 | 触发点 | 文件:行号 |
|--------|----------|--------|-----------|
| * | Completed | finish_task 成功 | interceptors.rs:267, engine.rs:1418, 1681 |
| * | Error | 引擎崩溃 | commands/workflow.rs:700 |
| * | Cancelled | stop 信号或用户取消 | engine.rs:916, 1588, 2449 |

---

## 3. update_state 函数

**位置**: `src-tauri/src/workflow/react/engine.rs:2224-2255`

```rust
pub(crate) async fn update_state(
    &mut self,
    new_state: WorkflowState,
) -> Result<(), WorkflowEngineError> {
    self.state = new_state.clone();
    
    // 离开审批等待态时清理 pending_approvals
    if matches!(
        new_state,
        WorkflowState::Thinking | WorkflowState::Executing | WorkflowState::Completed
    ) {
        self.pending_approvals.clear();
    }
    
    // 发送状态到前端
    self.gateway.send(&self.session_id, GatewayPayload::State { state: new_state.clone() }).await?;
    
    // 持久化状态到数据库
    let _ = store.update_workflow_status(&self.session_id, &new_state.to_string());
    
    Ok(())
}
```

---

## 4. 状态等待与信号处理

### 4.1 主循环等待逻辑

**位置**: `src-tauri/src/workflow/react/engine.rs:836-920`

```rust
// AwaitingApproval 等待
if self.state == WorkflowState::AwaitingApproval {
    let signal_str = signal_rx.recv().await
        .ok_or_else(|| WorkflowEngineError::General("Signal channel closed".into()))?;
    // 处理 approval / stop 信号
}

// AwaitingUser 等待
if self.state == WorkflowState::AwaitingUser {
    let signal_str = signal_rx.recv().await
        .ok_or_else(|| WorkflowEngineError::General("Signal channel closed".into()))?;
    // 处理 user_input / stop 信号
}

// Paused 等待
if self.state == WorkflowState::Paused {
    let signal_str = signal_rx.recv().await
        .ok_or_else(|| WorkflowEngineError::General("Signal channel closed".into()))?;
    // 处理 continue / stop 信号
}
```

### 4.2 信号类型与处理

| 信号类型 | 接受状态 | 处理逻辑 |
|----------|----------|----------|
| `stop` | 所有等待态 | → Cancelled |
| `approval` | AwaitingApproval | 执行/拒绝工具 → Thinking |
| `user_input` | AwaitingUser | 添加消息 → Thinking |
| `continue` | Paused | → Thinking |
| `request_confirm_broadcast` | AwaitingApproval | 重播确认弹窗 |
| `remove_auto_approved_tool` | 任意 | 更新 auto_approve 列表 |
| `remove_shell_policy_item` | 任意 | 更新 shell_policy |

---

## 5. 当前问题汇总

### 5.1 状态分散问题

1. **Thinking → Thinking 自循环过多**
   - 8 处直接调用 `update_state(Thinking)`
   - 缺少明确的状态变化原因记录

2. **AwaitingAutoApproval 是过渡态**
   - 仅用于 Full 模式 submit_plan
   - 没有明确的退出路径

3. **Paused 状态未完全实现**
   - 仅有入口，缺少完整语义

### 5.2 恢复逻辑问题

**位置**: `src-tauri/src/workflow/react/engine.rs:371-469`

```rust
// 从消息历史恢复 pending_approvals
if self.state == WorkflowState::AwaitingApproval {
    // 扫描 assistant 消息找 "Awaiting approval" 文本
    // 从 tool 消息提取 tool_call_id
    // 重新插入 pending_approvals
}
```

**问题**：
- 依赖消息文本 "Awaiting approval" 推断状态
- 非结构化恢复，易出错

---

## 6. 待添加日志的关键点

根据阶段0要求，需在以下位置添加统一日志：

| 位置 | 日志事件 | 关键字段 |
|------|----------|----------|
| `update_state()` | `workflow.state_changed` | session_id, state_before, state_after |
| `init_internal()` 恢复分支 | `workflow.recovery.restored` | session_id, state, pending_tool_count |
| `run_loop_internal()` 等待前 | `workflow.wait.enter` | session_id, wait_reason |
| `run_loop_internal()` 信号接收 | `workflow.signal.received` | session_id, signal_type |
| `run_loop_internal()` 信号拒绝 | `workflow.signal.rejected` | session_id, signal_type, reason |
| `inject_input()` 成功 | `workflow.signal.injected` | session_id, signal_type |
| `inject_input()` 失败 | `workflow.signal.inject_failed` | session_id, reason |
| `register_session_tx()` | `workflow.channel.registered` | session_id |
| 引擎崩溃 | `workflow.engine.crashed` | session_id, error |
