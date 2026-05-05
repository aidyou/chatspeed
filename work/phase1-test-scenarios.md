# 阶段1测试验收文档

> 测试目的：验证 WorkflowManager 正确管理 session 生命周期
> 测试日期：2026-04-04
> 更新：按审计意见完成收口

---

## 一、自动化测试（已完成）

根据 plan.md 第 8.9 节和 8A.12 节要求，以下测试已通过：

| 测试项 | 状态 | 说明 |
|--------|------|------|
| `register_session` 后 `has_session == true` | ✅ | manager.rs 内联测试 |
| `remove_session` 后 `has_session == false` | ✅ | manager.rs 内联测试 |
| `get_executor` 能返回正确 executor 引用 | ✅ | manager.rs 内联测试 |
| 重复注册同一个 `session_id` 时行为明确 | ✅ | 返回 SessionAlreadyExists 错误 |
| `update_session_status` 后状态可读 | ✅ | manager.rs 内联测试 |
| 已结束 session 不可再路由普通 signal | ✅ | route_signal 返回 SignalRejected |
| 活跃 session 可以成功路由 signal | ✅ | route_signal 返回 Ok |
| session 移除后 manager 查询不到 | ✅ | has_session 返回 false |

---

## 二、审计收口项（已完成）

根据 phase1-audit-report.md 第 7 节要求：

| 收口项 | 状态 | 实现说明 |
|--------|------|----------|
| workflow_signal 先通过 manager 判断 | ✅ | 先调用 has_session/route_signal，再决定是否恢复 |
| workflow_start 先 manager 注册 | ✅ | manager 注册成功后才写入 BACKGROUND_TASKS |
| manager 注册失败阻断执行 | ✅ | 返回错误而非仅 warning |
| workflow_approve_plan 纳入 manager | ✅ | 添加 workflow_manager 参数，注册和移除 session |

---

## 三、日志验证（plan.md 第 8.8 节）

### 必须出现的日志事件

| 日志事件 | 触发条件 |
|----------|----------|
| `session_registered` | workflow_start / workflow_approve_plan 调用时 |
| `session_removed` | workflow 完成或取消时 |
| `session_lookup_hit` | 发送信号时 session 存在 |
| `session_lookup_miss` | 发送信号时 session 不存在 |
| `signal_routed` | 信号成功路由 |
| `signal_rejected` | 信号被拒绝（session 已结束） |

### 日志格式示例

```
[WorkflowManager][session=***][event=session_registered] Session registered with status Active
[WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
[WorkflowManager][session=***][event=signal_routed] Signal 'user_input' routed successfully
[WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

---

## 四、手工验收步骤（plan.md 第 8.10 节和 8A.13 节）

### 步骤清单

1. **启动应用**
2. **创建一个 workflow**
3. **启动执行**
4. **检查日志中是否出现 `session_registered`**
5. **刷新 workflow 页面**
6. **发送一条普通消息或 approval 信号**
7. **检查日志中是否优先命中 `session_lookup_hit`**
8. **结束 workflow**
9. **检查日志中是否出现 `session_removed`**

### 验收标准

**重要：验收时请确保日志级别设置为 INFO 或 DEBUG**

- [ ] 日志出现 `[WorkflowManager] Initialized`
- [ ] 启动 workflow 时出现 `session_registered`
- [ ] 刷新后发送信号时出现 `session_lookup_hit`（而非 `session_lookup_miss`）
- [ ] 信号路由时出现 `signal_routed`
- [ ] 结束 workflow 时出现 `session_removed`

### 验收日志示例

```
# 1. 启动应用
[WorkflowManager] Initialized

# 2. 创建并启动 workflow
[WorkflowManager][session=***][event=session_registered] Session registered with status Active
[Workflow][session=***][phase=start] Executor registered to WorkflowManager (primary)...

# 3. 刷新页面后发送消息
[Workflow][session=***][phase=signal] Signal received, type=user_input
[WorkflowManager][session=***][event=session_lookup_hit] Session exists in manager
[WorkflowManager][session=***][event=signal_routed] Signal 'user_input' routed successfully
[Workflow][session=***][phase=signal] Signal injected successfully, type=user_input

# 4. 结束 workflow
[WorkflowManager][session=***][event=session_removed] Session removed, was in status Active
```

---

## 五、完成定义检查（plan.md 第 8.11 节和 8A.15 节）

| 检查项 | 状态 |
|--------|------|
| `WorkflowManager` 已存在并注册为全局状态 | ✅ |
| `workflow_start` 已调用 manager 注册 session | ✅ |
| `workflow_signal` 已优先通过 manager 判断 session 活跃性 | ✅ |
| workflow 结束时 manager 会移除 session | ✅ |
| 自动化测试通过 | ✅ |
| 没有引入 snapshot、review gate、handoff 等超范围改动 | ✅ |
| `workflow_approve_plan` 已纳入 manager 生命周期 | ✅ |
| manager 注册失败时阻断执行 | ✅ |
| 手工验收通过 | 待测试 |

---

## 六、回退策略

保留旧 `BACKGROUND_TASKS` 查询支路作为兼容层，可通过 git 回退到阶段0稳定点。
