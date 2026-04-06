# 阶段5测试验收文档

> 测试目的：验证 `Snapshot First + Replay Fallback` 恢复链路  
> 对应计划：`work/plan.md` 第 12 节和 12A 节
> 自动化任务清单：`work/phase5-9-automation-tasklist.md`

---

## 一、阶段测试边界

本阶段聚焦恢复链路，不测试 Dispatcher、多 Sink、子任务、Handoff。

- 必测：snapshot 命中、snapshot 缺失 fallback、version mismatch fallback、replay 失败安全态
- 不测：UI transcript 重放（明确禁止）

---

## 二、核心验证点

1. 恢复优先级固定为 `snapshot -> replay`
2. snapshot 缺失或版本不兼容时自动 replay
3. replay 能重建最小运行态字段
4. replay 失败进入安全失败态（只读，不继续执行）
5. waiting 与 terminal 都可被重建

---

## 三、日志关键字对照

```log
workflow.restore.snapshot_hit
workflow.restore.snapshot_miss
workflow.restore.replay_fallback
workflow.replay.start
workflow.replay.done
workflow.replay.failed
```

---

## 四、自动化测试最低要求

1. snapshot 命中路径断言
2. snapshot 缺失走 replay 断言
3. snapshot version mismatch 走 replay 断言
4. replay 失败进入安全态断言
5. reducer 重建字段完整性断言

建议命令：

```bash
cargo test --manifest-path src-tauri/Cargo.toml workflow::react -- --nocapture
```

---

## 五、测试场景（自动化优先）

### 场景1：snapshot 命中优先恢复（自动化）

建议实现：
1. 单测构造 session + snapshot + events
2. 触发 restore 入口
3. 断言命中 snapshot 分支（不进入 replay）

验收：
- ✅ 日志出现 `workflow.restore.snapshot_hit`
- ✅ 不进入 replay
- ✅ waiting 恢复正确

### 场景2：snapshot 缺失 fallback 到 replay（自动化）

建议实现：
1. 构造 waiting 事件链但不写 snapshot（或删除 snapshot）
2. 调用 restore
3. 断言 `snapshot_miss -> replay_fallback -> replay.done`

验收：
- ✅ 日志出现 `snapshot_miss` 与 `replay_fallback`
- ✅ `replay.start -> replay.done`
- ✅ waiting 被正确重建

### 场景3：snapshot 版本不匹配 fallback 到 replay（自动化）

建议实现：
1. 写入不兼容 version 的 snapshot
2. 调用 restore
3. 断言自动切换 replay 并成功恢复

验收：
- ✅ 不兼容被识别
- ✅ 自动切换 replay
- ✅ 恢复成功

### 场景4：replay 失败进入安全失败态（自动化）

建议实现：
1. 构造损坏事件（非法/缺关键字段）
2. 调用 restore
3. 断言进入安全失败态，且 destructive 操作被拒绝

验收：
- ✅ `workflow.replay.failed` 可见
- ✅ session 进入安全失败态
- ✅ 不允许继续执行 destructive 操作

### 场景5：terminal 状态重建（自动化）

建议实现：
1. 分别构造 completed/cancelled 终态事件链
2. 删除 snapshot 后 restore
3. 断言终态准确重建

验收：
- ✅ terminal 状态准确重建
- ✅ 无误判为 running/waiting

### 场景6：reducer 字段完整性（自动化）

建议实现：
1. 构造含 waiting、approval、user_input 的复合事件链
2. 执行 replay reducer
3. 断言关键字段完整重建

验收：
- ✅ `state`、`wait_reason`、`pending_tools`、`current_step`、`last_action_summary`、`last_event_id` 全部重建正确

### 保留手工项（必要）

1. approval 弹窗交互体验与用户操作路径（同意/拒绝/重试）
2. 跨进程重启后的真实 UI 提示一致性（非仅后端状态）

---

## 六、里程碑回归范围（阶段5）

阶段5作为“恢复链路里程碑”，需要做增量回归：

1. 阶段2：snapshot 写读与 waiting 恢复
2. 阶段3：结构化 signal waiting 匹配
3. 阶段4：关键事件链写入与顺序
4. 阶段5：snapshot/replay 优先级与失败安全态

---

## 七、完成定义检查

| 检查项 | 状态 |
|---|---|
| 恢复优先级为 snapshot first | ⬜ |
| snapshot 缺失走 replay | ⬜ |
| version mismatch 走 replay | ⬜ |
| replay 失败进入安全态 | ⬜ |
| waiting/terminal 均可重建 | ⬜ |
| 未引入 transcript/chunk replay | ⬜ |
