# 阶段4测试验收文档

> 测试目的：验证结构化 Event Store 的落地、写入顺序与可观测性  
> 对应计划：`work/plan.md` 第 11 节和 11A 节

---

## 一、阶段测试边界

本阶段只验证 Event Store 自身能力，不验证 Replay、Handoff、Task Ledger。

- 必测：migration、append/list、关键事件链、`last_event_id` 对齐、失败可见性
- 不测：基于 event 的恢复执行（属于阶段5）

---

## 二、核心验证点

1. `workflow_events` 表和索引创建成功
2. 关键节点能写入结构化事件，且顺序正确
3. waiting/approval 相关事件链完整
4. snapshot 的 `last_event_id` 与事件链末尾可对齐
5. event append 失败不会 silent fail（有日志可见）

---

## 三、日志关键字对照

```log
workflow.event.append
workflow.event.append_failed
workflow.event.sequence_gap
```

建议同时关注：

```log
[Workflow][session=...][phase=snapshot] ...
```

用于核对 `last_event_id` 对齐点。

---

## 四、自动化测试最低要求

1. 事件写入后可按 `session_id,id` 顺序读出
2. waiting+approval 场景事件顺序断言
3. append 失败时返回错误并记录日志
4. snapshot 中 `last_event_id` 与 `workflow_events` 最大 `id` 对齐

建议命令：

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture
```

---

## 五、必须执行的手工测试场景

### 场景1：Event Store migration 验证

步骤：
1. 启动应用并触发 DB migration
2. 检查 `workflow_events` 表与索引

验收：
- ✅ 表存在，字段齐全
- ✅ 索引 `idx_workflow_events_session_id_id` 存在

### 场景2：普通执行链事件写入

步骤：
1. 创建并启动 workflow
2. 让其正常完成
3. 查询该 `session_id` 事件链

验收：
- ✅ 包含 `WorkflowStarted`
- ✅ 包含多次 `StateChanged`
- ✅ 包含 `WorkflowCompleted`
- ✅ 时间与顺序单调递增

### 场景3：Approval 等待事件链

步骤：
1. 触发需要审批的工具
2. 暂不审批，查询事件
3. 执行一次 approve/reject，查询事件

验收：
- ✅ 出现 `WaitEntered(wait_reason=approval)`
- ✅ 出现 `ApprovalRequested`
- ✅ 操作后出现 `ApprovalResolved`

### 场景4：UserInput 等待事件链

步骤：
1. 触发 `ask_user`
2. 输入回复
3. 查询事件链

验收：
- ✅ 出现 `WaitEntered(wait_reason=user_input)`
- ✅ 出现 `UserInputReceived`
- ✅ 后续状态恢复为 running/thinking

### 场景5：`last_event_id` 对齐验证

步骤：
1. 在 waiting、completed、cancelled 三种状态分别取 snapshot
2. 查询各自事件链尾部 `id`

验收：
- ✅ snapshot 中 `last_event_id` 与事件链尾部一致
- ✅ 不出现逆序或跳号未解释情况

### 场景6：append 失败可见性验证

步骤：
1. 人为制造一次 event append 失败（如 mock DB 错误）
2. 观察执行链和日志

验收：
- ✅ 出现 `workflow.event.append_failed`
- ✅ 错误可见，不 silent fail

---

## 六、完成定义检查

| 检查项 | 状态 |
|---|---|
| `workflow_events` 表与索引存在 | ⬜ |
| 关键事件链可按 session 查询 | ⬜ |
| waiting/approval 事件顺序正确 | ⬜ |
| `last_event_id` 可对齐 | ⬜ |
| append 失败可见 | ⬜ |
| 未引入 replay 逻辑 | ⬜ |

---

## 七、回归范围说明

阶段4不是里程碑回归阶段。  
回归仅限阶段4本身与阶段3等待/审批主链的冒烟校验。

