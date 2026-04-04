# 阶段 1 验收评审意见

> 评审日期：2026-04-04
> 最终更新：基于代码复核、`cargo check` 与最终手工测试日志
> 评审依据：
> - `work/plan.md` 第 8 节「阶段 1：引入 WorkflowManager」
> - `work/phase1-test-log.md`
> - `work/phase1-test-scenarios.md`
> - `src-tauri/src/lib.rs`
> - `src-tauri/src/commands/workflow.rs`
> - `src-tauri/src/workflow/react/manager.rs`

---

## 1. 评审结论

阶段 1 的核心目标是把 workflow session 生命周期从命令层和 `BACKGROUND_TASKS` 中抽离，统一收敛到 `WorkflowManager`。

基于当前代码实现、自动化检查和最终手工测试日志，本阶段可以判定为：

- **阶段 1 目标已达成**
- **自动化检查通过**
- **手工验收通过**
- **可以进入下一阶段开发**

本次最终测试已经补齐了阶段 1 之前缺失的关键证据：

1. approval 等待态在多次刷新后，`request_confirm_broadcast` 仍然稳定命中 manager 并成功注入 signal
2. `ask_user` 等待态在刷新后，普通 `user_input` 能正确命中活跃 session，而不是错误重新 `workflow_start`
3. 文件审批场景中，`approval` signal 也能正确命中 manager 并恢复执行
4. 整个 session 在完成后能正常从 manager 中移除

因此，阶段 1 已不再停留在“代码结构基本正确”，而是已经具备了计划要求的“可运行、可测试、可观测”。

---

## 2. 与计划目标的对照结论

### 2.1 阶段目标是否达成

`plan.md` 对阶段 1 的定义是：

> 把 session 生命周期从命令层和 `BACKGROUND_TASKS` 中抽离，交给一个后端全局管理器。

从当前结果看，这一目标已 **达成**：

- `WorkflowManager` 已存在并注册为全局状态。
- `workflow_start` 已以 manager 作为主注册表。
- `workflow_signal` 已先通过 manager 判断 session，再决定是否路由或恢复。
- `workflow_approve_plan` 已纳入 manager 生命周期。
- `BACKGROUND_TASKS` 仍保留，但已退化为兼容层。

### 2.2 阶段 1 验收标准是否通过

依据 `work/plan.md` 第 8.11 节和 8A.15 节：

| 验收标准 | 结论 | 说明 |
|---|---|---|
| `workflow_signal` 已通过 manager 路由 | 通过 | 最终日志已出现稳定的 `session_lookup_hit`、`signal_routed`、`Signal injected successfully` |
| session 生命周期不再由命令层零散管理 | 通过 | 主启动、signal 路由、结束移除均已收口到 manager |
| 日志能看出每个 session 的注册、更新、移除 | 通过 | 注册、命中、路由、等待、移除均有明确日志 |

结论：**阶段 1 按验收标准可判定通过。**

---

## 3. 已达成项

### 3.1 `WorkflowManager` 模块与全局注册已完成

当前已经新增并启用：

- `src-tauri/src/workflow/react/manager.rs`

同时在 [lib.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/lib.rs#L645) 至 [lib.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/lib.rs#L647) 完成了全局状态注册。

这满足了 `plan.md` 第 8.6 节和 8A.6 节要求。

### 3.2 `workflow_start` 已改为 manager 主注册表

在 [workflow.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/commands/workflow.rs) 当前实现中可以确认：

- 先调用 `workflow_manager.register_session(...)`
- 若注册失败，直接返回错误
- 注册成功后才写入 `BACKGROUND_TASKS`

这符合 `plan.md` 第 8A.7 节“manager 是主注册表，BACKGROUND_TASKS 是兼容层”的要求。

### 3.3 `workflow_signal` 已改为 manager-first

在 [workflow.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/commands/workflow.rs) 当前实现中可以确认：

- 先 `has_session`
- 命中后调用 `route_signal`
- 只有 miss 才进入恢复逻辑

这符合 `plan.md` 第 8.6、8A.8、8A.15 节对 `workflow_signal` 的核心要求。

### 3.4 `workflow_approve_plan` 已纳入 manager 生命周期

在 [workflow.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/commands/workflow.rs) 当前实现中可以确认：

- `workflow_approve_plan` 已注册 manager
- 运行结束时已从 manager 移除

这意味着阶段 1 范围内的活跃 workflow session 已统一纳入 manager 托管。

### 3.5 自动化验证通过

已验证：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml workflow::react::manager
```

结果：

- `cargo check` 通过
- manager 单测通过

当前 `cargo check` 的警告都集中在 `manager.rs` 中尚未完全消费的预留字段和接口上，例如：

- `ManagedSession` 的部分字段
- `touch`
- `InvalidState`
- `get_session_status`
- `get_executor`
- `update_session_status`

这些警告不影响阶段 1 验收，可以视为后续阶段扩展能力时再消费的预留结构。

---

## 4. 手工测试结论

### 4.1 approval 等待态在刷新后可稳定恢复

最终测试日志中，多次出现以下完整链路：

- `Signal received, type=request_confirm_broadcast`
- `session_lookup_hit`
- `signal_routed`
- `Signal injected successfully`
- executor 收到 `request_confirm_broadcast`
- 再次进入 `wait_reason=approval`

这说明：

- 刷新页面后，approval 等待态没有丢失活跃 session
- manager 可以持续命中同一个 session
- signal 可以稳定注入到现有 executor

### 4.2 `ask_user` 等待态在刷新后可稳定恢复

最终测试日志中，`awaiting_user` 后出现了如下关键链路：

- `Signal received, type=user_input`
- `session_lookup_hit`
- `signal_routed`
- `Signal injected successfully`
- executor 收到带内容的 `user_input`
- 状态从 `awaiting_user -> thinking`

这说明之前“刷新后点击/输入会错误触发 `workflow_start`”的问题已经被收口，当前实现已经符合阶段 1 对 waiting 恢复的预期。

### 4.3 文件审批场景可恢复并继续执行

最终测试日志中，approval 场景出现了如下链路：

- `Signal received, type=approval`
- `session_lookup_hit`
- `signal_routed`
- `Signal injected successfully`
- executor 收到 approval
- `awaiting_approval -> thinking`
- 后续继续执行直至 `completed`

这说明 approval signal 的恢复路径也已在手工场景下打通。

### 4.4 session 完成后可正常移除

最终日志末尾可以看到：

- workflow 状态进入 `completed`
- manager 输出 `session_removed`

这满足了阶段 1 对 session 生命周期闭环的要求。

---

## 5. 对本阶段范围内问题的最终判断

此前阶段 1 的主要阻塞点包括：

1. `ask_user` 刷新后错误触发 `workflow_start`
2. 刷新恢复链路中出现 `Signal channel closed`
3. manager-first signal 路由缺少实证日志

结合最终日志，这几项现在都可以判定为：

- **已得到有效收口或已有足够正向证据覆盖**

尤其是最终日志中没有再出现此前阻塞阶段 1 验收的这类错误链路：

- `Session already exists`
- 恢复时误触发新的 `phase=start`
- `Signal channel closed`

因此，本阶段范围内的阻塞问题已经解除。

---

## 6. 已发现但不作为阶段 1 阻塞项的问题

以下问题在测试过程中曾被观察到，但不作为本阶段是否通过的主判定依据：

1. planning 模式下审批弹窗表现存在过不稳定现象
2. 某些非核心交互仍可能存在前端层面的展示抖动

这些问题建议在后续阶段按 waiting / approval UI 或更完整的恢复体验继续处理，但不影响阶段 1 关闭。

---

## 7. 最终建议

当前最准确的正式结论应为：

> 阶段 1 已完成。`WorkflowManager` 已经成为 workflow session 生命周期的主控制面，`workflow_start`、`workflow_signal`、approval/ask_user 等 waiting 恢复路径都已具备稳定的 manager-first 路由与执行能力，测试日志也已提供足够证据证明刷新后的活跃 session 可以继续被找到、继续被注入 signal，并最终正常结束。因此可以进入下一阶段开发。
