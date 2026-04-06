# Phase4 场景2/4 事件链整改方案

> 目标：修复 `work/phase4-test-log.md` 中场景2/4不满足验收定义的问题，并给出可直接执行的整改步骤。  
> 范围：仅阶段4 Event Store，不引入阶段5 replay 语义。

---

## 1. 现状与问题定位

对照 [phase4-test-scenarios.md](./phase4-test-scenarios.md) 与 [phase4-test-log.md](./phase4-test-log.md)：

### 1.1 场景2与场景4可以共用一条日志链

这是合理的。  
如果同一个 `session_id` 先发生普通执行，再进入 `ask_user` 并收到用户输入，这一条链路可以同时覆盖：

- 场景2（普通执行链）
- 场景4（UserInput 等待链）

### 1.2 当前日志不满足场景4关键验收

你的样例链路中有：

- `wait_entered(wait_reason=user_input)`
- `state_changed(awaiting_user -> thinking)`

但缺失：

- `user_input_received`

这会导致场景4“输入事件可审计”不成立。

### 1.3 当前 event_type 存储格式错误

日志中 `event_type` 为：

- `"\"workflow_started\""`
- `"\"state_changed\""`

说明 DB 中把枚举序列化成了 JSON 字符串文本（多了一层引号），应为裸字符串：

- `workflow_started`
- `state_changed`

### 1.4 存在重复 terminal 事件

样例中出现两条 `workflow_completed`（id 13、14）。  
`update_state()` 当前实现会在 `new_state == Completed` 时无条件追加 terminal 事件，若重复调用 `update_state(Completed)` 会重复写入。

---

## 2. 根因分析（代码级）

### 根因A：`UserMessage` 分支未写 `UserInputReceived`

文件：`src-tauri/src/workflow/react/engine.rs`  
位置：`WorkflowSignal::UserMessage { content }` 两处分支（约第 1391 行、1737 行）  
现状：仅 `add_message_and_notify_internal` + `update_state(Thinking)`，未 `append_event(WorkflowEvent::user_input_received(...))`。

### 根因B：event_type 写库方式错误

文件：`src-tauri/src/db/workflow.rs`  
函数：`append_workflow_event`  
现状：

- `let event_type_str = serde_json::to_string(&event.event_type)?;`

这会生成带引号 JSON 字面量字符串。

### 根因C：terminal 事件写入未做幂等保护

文件：`src-tauri/src/workflow/react/engine.rs`  
函数：`update_state`  
现状：`Completed/Cancelled/Error` 事件在 `new_state` 命中时总是 append，不检查 `old_state == new_state`。

---

## 3. 详细整改方案

## 3.1 修复 `user_input_received` 事件缺失（阻塞）

### 修改点

文件：`src-tauri/src/workflow/react/engine.rs`

在两处 `WorkflowSignal::UserMessage { content }` 分支里，`add_message_and_notify_internal` 之后、`update_state(Thinking)` 之前，追加：

```rust
let event = WorkflowEvent::user_input_received(self.session_id.clone(), content.clone());
if let Err(e) = self.append_event(&event) {
    log::error!(
        "[Workflow][session={}] event.append_failed - error={}",
        self.session_id,
        e
    );
}
```

### 预期效果

- 场景4链路中，`wait_entered(user_input)` 与 `state_changed(awaiting_user->thinking)` 之间出现 `user_input_received`。

---

## 3.2 修复 `event_type` 双引号污染（阻塞）

### 修改点

文件：`src-tauri/src/db/workflow.rs`  
函数：`append_workflow_event`

将：

```rust
let event_type_str = serde_json::to_string(&event.event_type)?;
```

改为：

```rust
let event_type_str = format!("{}", event.event_type);
```

前提：`WorkflowEventType` 需实现 `Display`（当前未实现则补 `strum::Display` 或手写 `impl Display`）。

### 数据回填（已污染历史数据）

先备份，再执行一次性 SQL 清洗（SQLite）：

```sql
UPDATE workflow_events
SET event_type = TRIM(event_type, '\"')
WHERE event_type LIKE '\"%\"';
```

说明：仅清除首尾双引号，避免改坏正常值。

---

## 3.3 修复重复 `workflow_completed`（高优先）

### 修改点

文件：`src-tauri/src/workflow/react/engine.rs`  
函数：`update_state`

将 terminal 事件写入放入 `if old_state != new_state` 分支中，或在 terminal match 前增加 guard：

```rust
if old_state == new_state {
    // no terminal event append
} else {
    // append terminal events
}
```

### 预期效果

- 单 session 的单次终态不再出现重复 `workflow_completed`/`workflow_cancelled`/`workflow_failed`。

---

## 4. 验收查询与判定标准

### 4.1 场景2+4合并链路查询

```sql
SELECT id, session_id, event_type, event_data, created_at
FROM workflow_events
WHERE session_id = :sid
ORDER BY id ASC;
```

### 4.2 场景4关键断言（必须）

按顺序至少出现：

1. `state_changed` (`executing` -> `awaiting_user`)
2. `wait_entered` (`wait_reason=user_input`)
3. `user_input_received`（包含用户输入内容）
4. `state_changed` (`awaiting_user` -> `thinking`)

### 4.3 终态去重断言（必须）

```sql
SELECT event_type, COUNT(*) as cnt
FROM workflow_events
WHERE session_id = :sid
  AND event_type IN ('workflow_completed', 'workflow_cancelled', 'workflow_failed')
GROUP BY event_type;
```

对单次执行终态，`cnt` 应为 `1`。

### 4.4 event_type格式断言（必须）

不得再出现：

- `"\"workflow_started\""`

应为：

- `workflow_started`

---

## 5. 测试日志文档整改建议（避免再次误判）

文件：`work/phase4-test-log.md`

建议结构：

1. 明确标注“场景2与场景4共用 session_id”
2. 在同一 JSON 事件链后附“覆盖映射表”

示例：

```markdown
本链路 session_id=xxx 同时覆盖场景2与场景4：
- 场景2：WorkflowStarted + StateChanged + WorkflowCompleted
- 场景4：WaitEntered(user_input) + UserInputReceived + AwaitingUser->Thinking
```

---

## 6. 执行顺序（给执行AI）

1. 修 `engine.rs`：补 `UserInputReceived` 两处写入  
2. 修 `db/workflow.rs`：event_type 存储改裸字符串  
3. 修 `engine.rs`：terminal 事件幂等  
4. 跑后端测试 + 手工复测场景2/4  
5. 执行历史数据清洗 SQL（可选但建议）  
6. 更新 `work/phase4-test-log.md`，给出“同链覆盖场景2/4”说明

---

## 7. 完成定义（本整改）

- [ ] 场景2/4共链路成立且场景4含 `user_input_received`
- [ ] `event_type` 不再带双引号
- [ ] 终态事件无重复写入
- [ ] `phase4-test-log.md` 明确覆盖映射

