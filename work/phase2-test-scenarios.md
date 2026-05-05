# 阶段2测试验收文档

> 测试目的：验证 ExecutionContext Snapshot 保存、读取、恢复与 fallback
> 测试日期：2026-04-04

---

## 一、阶段目标对照

根据 `work/plan.md` 第 9 节和 9A 节，阶段2的核心目标是：

1. 将最小运行态写入结构化 `ExecutionContext Snapshot`
2. waiting 恢复优先使用 snapshot，而不是 transcript 解析
3. waiting 状态可以在页面刷新和应用重启后恢复
4. 旧 transcript 解析只保留为 fallback

因此，本阶段测试不能只验证“能恢复”，还必须验证：

- 恢复是否优先走 snapshot
- snapshot 缺失时是否才走 fallback
- waiting / terminal 关键点是否都发生 snapshot 写入

---

## 二、核心验证点

1. Snapshot 在等待/终止状态自动保存
2. Approval 恢复优先使用 Snapshot，而非 transcript 解析
3. Snapshot 缺失时 Fallback 到 transcript 解析
4. UserInput waiting 也能通过 snapshot 恢复
5. Terminal 状态（completed / cancelled）会写入 snapshot

---

## 三、日志关键字对照

测试时建议重点关注以下实际日志：

```log
[Workflow][session=...][phase=snapshot] Saved: state=..., wait_reason=..., pending_tools=...
[Workflow][session=...] snapshot.read - state=..., wait_reason=..., pending_tools=...
[Workflow][session=...][phase=restore] Restoring ... pending approvals from snapshot
[Workflow][session=...][phase=restore] snapshot.fallback_legacy - No snapshot found, falling back to transcript parsing
```

说明：

- `workflow.snapshot.write` 在当前代码里实际体现为 `phase=snapshot` 的 `Saved: ...`
- `workflow.snapshot.read` 在 DB 层日志里体现为 `snapshot.read - ...`
- `workflow.snapshot.restore` 在当前代码里实际体现为 `phase=restore`
- `workflow.snapshot.fallback_legacy` 与计划一致

---

## 四、必须执行的手工测试

以下手工测试建议全部执行。只有完成这些场景，才足以判断阶段2是否达到计划要求。

### 场景1：Approval Snapshot 保存与刷新恢复

**重要说明：**

- 这个场景必须提供一份“干净日志”
- 测试前必须确认 `workflow_snapshots` 表已经存在
- 本场景日志中不能再出现 `no such table: workflow_snapshots`
- 这一条是阶段2是否通过的主阻塞场景之一

**步骤：**
1. 触发需要审批的工具调用，例如需要人工确认的文件编辑或 shell 工具
2. 审批弹窗出现后，刷新页面
3. 验证审批弹窗重新出现

**关键日志：**
```log
[Workflow][session=...][phase=snapshot] Saved: state=Waiting, wait_reason=Approval, pending_tools=1
[Workflow][session=...] snapshot.read - state=Waiting, wait_reason=Approval, pending_tools=1
[Workflow][session=...][phase=restore] Restoring 1 pending approvals from snapshot
```

**验收：**
- ✅ 看到 snapshot 保存日志
- ✅ 看到 `snapshot.read`
- ✅ 刷新后看到 `Restoring ... from snapshot`
- ✅ 没有 `snapshot.fallback_legacy` 警告
- ✅ 审批信息正确显示
- ✅ 没有 `no such table: workflow_snapshots`

---

### 场景2：应用重启后恢复 Waiting 态

**步骤：**
1. 触发审批，弹窗出现后关闭应用
2. 重启应用，打开该 workflow
3. 验证审批弹窗重新出现

**验收：**
- ✅ 重启后看到 `snapshot.read`
- ✅ 重启后看到 `Restoring ... from snapshot`
- ✅ 没有 `snapshot.fallback_legacy` 警告
- ✅ 审批信息正确恢复

---

### 场景3：Snapshot 缺失时 Fallback 到旧逻辑

**重要：** 必须重启应用才能触发 `init_internal` 的恢复逻辑。刷新页面不会触发此逻辑，因为 executor 仍在内存中。

**步骤：**
1. 触发审批后，**关闭应用**
2. 删除 snapshot：
   ```sql
   DELETE FROM workflow_snapshots WHERE session_id = '<session_id>';
   ```
3. **重启应用**，打开该 workflow
4. 验证仍能恢复，通过 transcript 解析兜底

**关键日志：**
```log
[Workflow][session=...][phase=restore] snapshot.fallback_legacy - No snapshot found, falling back to transcript parsing
```

**验收：**
- ✅ 看到 `snapshot.fallback_legacy` 警告
- ✅ 审批仍能恢复

---

### 场景4：UserInput Waiting Snapshot 保存与恢复

这是阶段2必须补充的手工场景，因为 `plan.md` 第 `9A.4` 明确要求覆盖 `user input 等待后`。

**步骤：**
1. 让 workflow 进入 `ask_user` 或其他 `awaiting_user` 状态
2. 确认进入等待后刷新页面
3. 验证等待 UI 仍然存在
4. 发送一条新的用户输入
5. 验证 workflow 继续执行

**关键日志：**
```log
[Workflow][session=...][phase=snapshot] Saved: state=Waiting, wait_reason=UserInput, pending_tools=0
[Workflow][session=...] snapshot.read - state=Waiting, wait_reason=UserInput, pending_tools=0
```

**验收：**
- ✅ 看到 snapshot 保存日志，且 `wait_reason=UserInput`
- ✅ 刷新后看到 `snapshot.read`
- ✅ 用户输入后 workflow 能继续执行
- ✅ 不需要依赖 transcript 反推才能恢复等待态

---

### 场景5：Completed 状态 Snapshot 写入验证

这是阶段2必须补充的 terminal 场景之一，对应 `plan.md` 第 `9.5`、`9A.4`。

**步骤：**
1. 运行一个正常完成的 workflow
2. workflow 完成后检查日志
3. 如有 DB 查询条件，可检查该 session 的 snapshot

**关键日志：**
```log
[Workflow][session=...][phase=snapshot] Saved: state=Completed, wait_reason=None, pending_tools=0
```

**验收：**
- ✅ workflow 完成时写入 completed snapshot
- ✅ snapshot 中不再带 waiting 专用字段

---

### 场景6：Cancelled 状态 Snapshot 写入验证

这是阶段2必须补充的 terminal 场景之一，对应 `plan.md` 第 `9.5`、`9A.4`。

**步骤：**
1. 在 waiting 或 running 状态下停止 workflow
2. 检查日志

**关键日志：**
```log
[Workflow][session=...][phase=snapshot] Saved: state=Cancelled, wait_reason=None, pending_tools=0
```

**验收：**
- ✅ workflow 取消时写入 cancelled snapshot

---

## 五、自动化测试要求

当前阶段的自动化测试不能只跑 `workflow::react::types`。那只能证明结构体可序列化，不能证明阶段2主目标达成。

### 最低要求

至少应补齐并执行以下类型的测试：

1. snapshot 写入成功
2. snapshot 读取成功
3. `pending_tools` 能正确 round-trip
4. approval 恢复优先走 snapshot
5. snapshot 缺失时 fallback 到旧逻辑

### 当前建议命令

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react::types -- --nocapture
```

如果后续已经增加 DB / engine 级测试，应补充相应命令，例如：

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture
```

或至少在测试记录中明确：

- 哪些测试已实现
- 哪些测试暂时只有手工覆盖

---

## 六、恢复点覆盖矩阵

这是阶段2验收时建议附带的一张覆盖表，用于确认 `plan.md 9A.4` 要求的恢复点是否都被验证。

| 恢复点 | 手工测试 | 自动化测试 | 是否必须 |
|--------|----------|------------|----------|
| approval 等待后 | 场景1/2/3 | 应覆盖 | 是 |
| approval 响应后 | 场景1/2 + approval 操作 | 建议覆盖 | 是 |
| user input 等待后 | 场景4 | 建议覆盖 | 是 |
| completed | 场景5 | 建议覆盖 | 是 |
| failed | 不要求手工覆盖 | 建议自动化覆盖 | 否 |
| cancelled | 场景6 | 建议覆盖 | 是 |

---

## 七、完成定义检查

只有满足以下条件，才建议判定阶段2通过：

| 检查项 | 状态 |
|--------|------|
| `workflow_snapshots` 表已落地 | ⬜ |
| Snapshot 写入日志可见 | ⬜ |
| Snapshot 读取日志可见 | ⬜ |
| approval 恢复优先走 snapshot | ⬜ |
| snapshot 缺失时 fallback 可见 | ⬜ |
| user_input waiting 可恢复 | ⬜ |
| completed / cancelled 已覆盖关键 terminal 场景 | ⬜ |
| failed 场景已明确转自动化或后续补充 | ⬜ |
| 自动化测试达到阶段2最低要求 | ⬜ |

---

## 八、验收清单

| 场景 | 状态 |
|------|------|
| 场景1：Approval Snapshot 保存与刷新恢复 | ⬜ |
| 场景2：应用重启恢复 Waiting 态 | ⬜ |
| 场景3：Fallback 机制 | ⬜ |
| 场景4：UserInput Waiting 恢复 | ⬜ |
| 场景5：Completed Snapshot 写入 | ⬜ |
| 场景6：Cancelled Snapshot 写入 | ⬜ |
| 自动化测试通过 | ⬜ |
