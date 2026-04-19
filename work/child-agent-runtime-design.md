# 主代理与子代理运行关系设计说明

本文档是 `work/plan.md` 的补充说明，记录当前主代理与子代理关系的审查结论、设计边界和后续修正方向。

本文档不引入多代理复杂 UI、Inspector Tabs、Task Tiles 或完整任务账本。所有建议仍遵循当前重构原则：

- Session 生命周期由后端管理。
- 等待态必须结构化。
- 影响恢复的状态必须进入 snapshot 或结构化事件。
- 第一阶段先保证 snapshot 与 waiting 恢复正确，不提前做完整 replay/handoff。

---

## 1. 当前目标关系

主代理和子代理都应被视为独立的 `Workflow Session`。

主代理负责：

- 根据任务决定是否调用 `task` 工具。
- 在 `call` 模式下等待子代理结果。
- 在收到子代理完成结果后继续推理或收敛。

子代理负责：

- 使用自己的 agent 配置、模型、工具权限和 prompt 执行 delegated task。
- 将运行状态、上下文占用、工具调用数和最终结果以结构化形式反馈给父会话。
- 在完成、失败、取消或 interrupted 时进入明确终态。

父子关系不应通过普通 transcript 文本反推。最小可靠模型应包含：

- 父会话 `ExecutionContext.waiting_on_task_id`
- 父会话 `ExecutionContext.child_sessions`
- 子会话自身 `ExecutionContext`
- 子任务完成/失败/interrupted 的结构化 completion
- 前端只消费轻量 projection，不作为事实来源

---

## 2. `call` 与 `background` 的语义边界

### 2.1 `call` 模式

`call` 表示父代理必须等待子代理结果才能继续。

正确流程：

1. 父代理调用 `task(execution_mode = "call")`。
2. runtime 创建子代理 session。
3. 父代理写入 `waiting_on_task_id` 和 `child_sessions`。
4. 父代理进入 `AwaitingChildTask`。
5. 父代理停止工具循环，不继续进入普通 thinking。
6. 子代理完成后，runtime 注入 `child_task_complete`。
7. 父代理消费 completion，清理 waiting 状态，写入子任务总结，然后恢复 thinking。

关键原则：

- 等待不是工具调用。
- 等待是 runtime state。
- 父代理不能靠反复调用 `task_output` 轮询 `call` 模式子任务。

### 2.2 `background` 模式

`background` 表示父代理不阻塞，子代理在后台运行。

正确流程：

1. 父代理调用 `task(execution_mode = "background")`。
2. runtime 创建后台子代理 session。
3. 父代理继续执行当前工作。
4. 子代理完成后，runtime 记录结构化完成事件和轻量 projection。
5. 父代理只有在自然需要时才读取结果。

关键原则：

- `background` 不是父代理等待点。
- `task_output` 不应成为高频轮询机制。
- 后台任务完成应由 runtime event-driven 通知，而不是由模型不断猜测和查询。

---

## 3. 工具驱动 Agent 的冲突点

当前主代理是工具驱动的：每一步推理通常都需要继续产生工具调用。

这会带来一个典型冲突：

- 父代理启动异步子代理后回到 thinking。
- 由于它必须调用工具，它可能反复调用 `task_output`。
- 如果子代理运行时间很长，`task_output` 会不断返回未完成状态。
- 这会造成 token、模型调用、工具调用和 UI 状态的浪费。

业界常规处理方式是 event-driven completion，而不是 polling：

- `call` 模式下直接挂起父代理。
- `background` 模式下父代理继续做其他事情。
- 子代理完成后由系统产生 completion event 或 inbox item。
- 父代理需要时再消费结果，或由 runtime 在合适时机注入上下文。

因此，本项目应明确禁止用主代理循环调用 `task_output` 来实现等待。

---

## 4. 当前已发现的问题

### 4.1 子任务 completion 可能丢失

当前子代理完成后通过运行时 signal 通道通知父会话。如果父会话不在 manager 中、signal tx 已丢失或通道关闭，completion 可能静默丢失。

风险：

- 父会话保持 `AwaitingChildTask`。
- 子会话已经结束。
- 页面刷新或重启后只能看到卡住的父会话。

建议：

- 子任务 completion 必须 durable。
- completion 应写入结构化状态或事件。
- 父会话恢复时必须能从 snapshot/event 中发现未消费 completion。

### 4.2 interrupted 只写消息，不足以恢复父状态

当前重启清理会把未完成子代理标记为 `cancelled/interrupted`，并给父会话追加错误消息。

这还不够可靠，因为父会话的结构化等待状态可能仍然保留：

- `wait_reason = ChildTask`
- `waiting_on_task_id = child_id`
- `child_sessions` 仍包含该 child

风险：

- 父会话仍被视为等待子任务。
- UI 和恢复逻辑依赖结构化状态时仍会卡住。

建议：

- interrupted 应作为一种 `child_task_complete` 结构化结果被父状态机消费。
- 父会话必须清理 `waiting_on_task_id` 和对应 `child_sessions`。
- 如果父会话不在运行中，应在 snapshot 中保存 pending child completion，恢复后继续处理。

### 4.3 background 子代理缺少终态清理

后台子代理启动后，如果 run loop 结束，当前需要确保：

- 从 `BACKGROUND_TASKS` 移除。
- 写入 completed task snapshot。
- 写入子会话 terminal snapshot。
- 广播或记录 background completion projection。

否则会出现：

- stale executor 留在内存。
- `task_output` 行为不可靠。
- interrupted reconcile 因 `BACKGROUND_TASKS.contains_key` 误判为仍活跃。

### 4.4 子代理进度投影不应高频查 DB

`child_task_progress` 是正确方向，但进度投影最好不要在每次消息或状态变更时查询 DB 来找 parent。

建议：

- executor 创建时把 `parent_session_id` 放入运行态字段。
- progress event 从内存字段读取 parent。
- snapshot 仍作为恢复事实来源。

### 4.5 不应使用 `#[allow(dead_code)]` 掩盖无用字段

如果 `ParentSessionInfo.child_task_id` 无实际用途，应移除或改为真实使用。

原则：

- 无用代码直接删除。
- 不用 `#[allow(dead_code)]` 掩盖设计不清。

---

## 5. 建议的最小可靠模型

### 5.1 新增结构化 child completion

建议增加一个持久化结构，第一阶段可以先放进 snapshot，不必做完整事件表。

概念字段：

```text
ChildTaskCompletion {
  child_task_id,
  parent_session_id,
  status,              // completed | failed | cancelled | interrupted
  summary,
  error,
  tool_calls_count,
  completed_at_ms,
  consumed
}
```

父会话恢复时：

1. 读取 `ExecutionContext`。
2. 如果 `wait_reason = ChildTask`，检查是否存在未消费 completion。
3. 如果存在，恢复 executor 后注入 `child_task_complete`。
4. 父状态机消费 completion。
5. 标记 completion consumed。

### 5.2 `call` 模式状态机

推荐状态流：

```text
Thinking
  -> tool(task, call)
  -> AwaitingChildTask(waiting_on_task_id = child_id)
  -> child_task_complete(status = completed | failed | cancelled | interrupted)
  -> Thinking
```

如果子任务失败或 interrupted，父代理仍应恢复 thinking，但收到的是错误 observation。

父代理不应退出，除非用户停止父会话。

### 5.3 `background` 模式状态机

推荐状态流：

```text
Thinking
  -> tool(task, background)
  -> Thinking

Child session:
  Running
  -> Completed | Failed | Cancelled | Interrupted
```

父代理只在需要时消费后台结果。

`task_output` 应添加节流：

- 同一未完成任务在短时间内重复查询，返回 cached status。
- 不触发昂贵重建。
- 可以提示模型不要轮询，等待系统完成事件或继续其他工作。

---

## 6. 前端展示原则

状态面板只展示轻量 projection，不承载完整子会话。

推荐显示：

- 最新 5 个子代理。
- 当前状态。
- 最近摘要。
- 工具调用总数。
- 上下文占用比例。

点击行为：

- 如果最终总结显示在父会话消息流，点击跳转到对应父消息。
- 如果未来有子代理详情页或 inspector，再跳转到子代理详情。
- 当前阶段不做 inspector。

刷新恢复：

- 父会话 snapshot 提供 child id。
- 前端读取子会话 snapshot 重建状态面板。
- 前端事件只用于实时更新，不作为唯一事实来源。

---

## 7. 推荐修正顺序

1. 先修 completion 可靠性：子代理完成必须 durable，不能只依赖 live signal。
2. 修 interrupted 父状态清理：重启后 pending child 应转为结构化 interrupted completion。
3. 修 background 终态清理：确保后台子代理退出后从内存 registry 清理并保存结果。
4. 去掉无用 `#[allow(dead_code)]`。
5. 优化 `child_task_progress` parent 查找，避免每次事件查 DB。
6. 给 `task_output` 增加未完成状态节流，避免模型轮询浪费。

以上顺序符合 `work/plan.md` 的优先级：先保证 session/snapshot/waiting 可靠，再做多代理 UI 和更完整 replay。
