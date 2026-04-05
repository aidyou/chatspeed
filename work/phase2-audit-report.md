# 阶段 2 验收评审意见

> 评审日期：2026-04-05
> 评审依据：
> - `work/plan.md` 第 9 节「阶段 2：引入最小 Snapshot 与 ExecutionContext」
> - `work/phase2-test-log.md`
> - `work/phase2-test-scenarios.md`
> - `src-tauri/src/db/workflow.rs`
> - `src-tauri/src/workflow/react/engine.rs`
> - `src-tauri/src/workflow/react/types.rs`

---

## 1. 评审结论

阶段 2 的核心目标是把最小运行态从内存抽出来，写入结构化 `ExecutionContext Snapshot`，使 waiting 恢复优先依赖 snapshot，而不是 transcript 解析。

基于当前代码实现、`cargo check` 结果和最终手工测试日志，本阶段可以判定为：

- **阶段 2 目标已达成**
- **手工验收通过**
- **可以进入下一阶段开发**

当前已经具备阶段 2 关闭所需的关键证据：

1. approval 等待态可以写入 snapshot
2. approval 在应用重启后可从 snapshot 正向恢复
3. user input 等待态可以从 snapshot 正向恢复
4. snapshot 缺失时可 fallback 到旧 transcript 逻辑
5. terminal 状态中的 completed / cancelled 已有 snapshot 写入证据

因此，阶段 2 已满足“结构化 snapshot 成为 waiting 恢复主路径，旧 transcript 反推仅作为 fallback”的完成定义。

---

## 2. 与计划目标的对照结论

### 2.1 阶段目标是否达成

`plan.md` 对阶段 2 的定义是：

> 把运行态最小真相从内存抽出来，写入结构化快照，解决 waiting 恢复依赖 transcript 的问题。

从当前结果看，这一目标已 **达成**：

- `workflow_snapshots` 相关读写路径已经存在
- `ExecutionContext` 已定义并被用于导出最小运行态
- waiting / terminal 关键点已经发生 snapshot 写入
- approval 与 user input 的恢复都已有 snapshot 主路径证据
- transcript 反推仍保留为 fallback，并有明确日志

### 2.2 阶段 2 验收标准是否通过

依据 `work/plan.md` 第 9.11 节和 9A.7 节：

| 验收标准 | 结论 | 说明 |
|---|---|---|
| approval 恢复不再依赖 `workflow_messages` 解析 | 通过 | 已有 approval 正向 snapshot 恢复日志 |
| waiting 状态可在应用重启后恢复 | 通过 | approval 与 user input 均已有恢复证据 |
| `ExecutionContext` 成为恢复的主要结构 | 通过 | approval / user input 均先读 snapshot，fallback 单独记录 |

结论：**阶段 2 按验收标准可判定通过。**

---

## 3. 已达成项

### 3.1 Snapshot 读写链路已落地

当前代码中已经实现：

- snapshot 读取 API：[workflow.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/db/workflow.rs#L347)
- snapshot 写入 API：[workflow.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/db/workflow.rs#L380)

并且读写日志可见：

- `snapshot.read`
- `snapshot.write`

### 3.2 Approval 恢复已优先走 snapshot

approval 的恢复主路径位于 [engine.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/engine.rs#L419) 附近，当前逻辑已表现为：

1. 恢复时先读取 snapshot
2. 若存在 `pending_tools`，则重建 `pending_approvals`
3. 再向 UI re-broadcast
4. 只有 snapshot 缺失时才 fallback 到 transcript

这满足了阶段 2 最关键的设计要求。

### 3.3 UserInput waiting 已纳入 snapshot 恢复

`AwaitingUser` 的恢复逻辑位于 [engine.rs](/home/xc/dev/rust/chatspeed-workflow-refactor/src-tauri/src/workflow/react/engine.rs#L379)。

当前日志已证明：

- `AwaitingUser` 恢复时会先读 snapshot
- 当 `wait_reason=UserInput` 时，会按 snapshot 恢复 waiting 状态

这意味着阶段 2 不再只覆盖 approval waiting，也覆盖了 user input waiting。

### 3.4 Terminal 状态 snapshot 写入已具备关键证据

当前手工测试中已经确认：

- completed 会写入 snapshot
- cancelled 会写入 snapshot

虽然 failed 未做手工复现，但阶段 2 的手工关闭条件并不要求 failed 必须手工稳定复现，后续可由自动化补足。

### 3.5 `cargo check` 通过

已验证：

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

当前只有少量未使用告警，主要集中在：

- `delete_execution_context`
- `WorkflowManager` 的预留字段和接口
- `ExecutionContext::new`

这些不构成阶段 2 阻塞。

---

## 4. 手工测试结论

### 4.1 Approval Snapshot 保存与恢复

在 `work/phase2-test-log.md` 的场景 1 中，可以确认：

- approval waiting 时发生 snapshot.write
- 审批流程最终可继续执行并正常 completed

这证明 approval waiting 的 snapshot 写入链路存在。

### 4.2 UserInput 在重启后可从 snapshot 恢复

在 `work/phase2-test-log.md` 的场景 2 中，已经明确出现：

- `Workflow is awaiting user input, restoring from snapshot`
- `snapshot.read - state=Waiting, wait_reason=Some(UserInput), pending_tools=0`
- `Restoring user_input waiting state from snapshot`

这组日志是阶段 2 的关键正向证据，说明 `user_input waiting` 已通过 snapshot 恢复，而不是靠 transcript 或纯内存态兜底。

### 4.3 Fallback 机制成立

在 `work/phase2-test-log.md` 的场景 3 中，已经明确出现：

- `snapshot.fallback_legacy - No snapshot found, falling back to transcript parsing`

并且后续 approval 仍能恢复，这说明：

- snapshot 缺失时，旧逻辑还能工作
- fallback 符合阶段 2 回退策略

### 4.4 Approval 在重启后可从 snapshot 正向恢复

这是阶段 2 最关键的一条最终补充证据。

在 `work/phase2-test-log.md` 的场景 7 中，已经明确出现：

- `snapshot.write - state=Waiting, wait_reason=Some(Approval), pending_tools=1`
- 重启后：
  - `snapshot.read - state=Waiting, wait_reason=Some(Approval), pending_tools=1`
  - `Restoring 1 pending approvals from snapshot`

而且该场景没有出现 `snapshot.fallback_legacy`。

这意味着：

- approval 正向恢复主路径已经打通
- `ExecutionContext` 已真正成为 approval 恢复的主结构

### 4.5 Completed / Cancelled 的 snapshot 写入已验证

在现有日志中可以看到：

- completed 时的 `snapshot.write`
- cancelled 时的 `snapshot.write`

这覆盖了阶段 2 对 terminal 状态的关键写入验证。

---

## 5. 对本阶段范围内问题的最终判断

此前阶段 2 的主要阻塞点包括：

1. approval 是否真正优先走 snapshot 恢复
2. user input waiting 是否真正纳入 snapshot 恢复
3. fallback 是否有明确废弃日志
4. terminal 状态是否发生 snapshot 写入

结合当前最终日志，这几项现在都可以判定为：

- **已具备足够正向证据**

因此，本阶段范围内的阻塞问题已经解除。

---

## 6. 已发现但不作为阶段 2 阻塞项的问题

### 6.1 Stop 信号响应体验

在场景 2 与场景 4 中，用户观察到“停止后流程还会短暂继续向前，再真正进入 cancelled”。

从日志判断，实际行为是：

1. 第一次 stop 已成功注入
2. executor 在下一次循环检查点检测到 stop
3. 随后进入 cancelled 并写入 snapshot
4. 用户因为 UI 上还未立即体现停止结果，又再次点击了 stop

这更像：

- stop 响应时机与 UI 反馈的体验问题

而不是：

- snapshot / waiting 恢复逻辑错误

因此，该问题建议作为后续优化项处理，不作为阶段 2 阻塞项。

---

## 7. 最终建议

当前最准确的正式结论应为：

> 阶段 2 已完成。`ExecutionContext Snapshot` 已成为 waiting 恢复的主要结构，approval 与 user input 等待态均已具备 snapshot 主恢复路径，snapshot 缺失时也能明确 fallback 到旧 transcript 逻辑；completed 与 cancelled 状态也已有 snapshot 写入证据。因此可以进入下一阶段开发。至于 stop 信号响应体验问题，应记录为后续优化项，而不是阶段 2 的阻塞项。
