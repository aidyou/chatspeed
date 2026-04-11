# 阶段 9 执行计划（可直接分派给其他 AI）

本文档用于替代 `work/plan.md` 中第 16/16B 节的高层描述，目标是让执行者“按单施工”而非二次设计。

---

## 1. 阶段目标（可验收）

1. 建立 `Task Ledger` 前端统一视图模型，不再依赖 transcript 文本推断任务态。
2. 工具态收敛到单一主键：`tool_call_id`，避免 pending / approved / rejected / final 多轨并存。
3. 高级 UI（任务列表/Inspector）只消费结构化状态与事件，不影响执行内核。
4. UI 异常必须可降级，不能阻断 workflow 执行与恢复。

---

## 2. 范围边界（必须遵守）

### 2.1 允许改动

- `src/components/workflow/*`
- `src/composables/workflow/*`
- `src/stores/workflow.js`
- 与 task ledger 数据暴露相关的后端定义（仅结构化输出，不改状态机语义）

### 2.2 禁止改动

- 禁止改动核心执行状态机语义（`engine.rs` 主流程状态迁移逻辑）
- 禁止新增 transcript 文本正则推断任务态
- 禁止 UI 逻辑回写执行层状态（除现有 signal 命令外）

---

## 3. 文件级任务拆分（执行清单）

## 3.1 Task A：定义前端任务账本模型

### 目标

建立统一 `TaskItem` / `ToolViewState` 结构，作为所有工作流消息展示的唯一中间层。

### 主要文件

- `src/stores/workflow.js`
- `src/composables/workflow/useWorkflowMessages.ts`
- `src/composables/workflow/useWorkflowCore.ts`

### 具体动作

1. 在 store 新增/统一 `tool_call_id -> ToolViewState` 索引容器（Map 或普通对象）。
2. 规定 `ToolViewState.status` 枚举：`pending | approved_running | rejected | final_success | final_error`。
3. 约束所有 tool stream、approval、tool final message 更新只走一个 reducer 入口（例如 `upsertToolViewState`）。
4. 保留 `messages` 用于原始记录，但 UI 优先消费视图层数据。

### 交付物

- 可复用的 `upsert` / `finalize` / `clear` API
- `tool_call_id` 对应状态机图（写在代码注释或本文件附录）

---

## 3.2 Task B：消息->任务态映射收敛

### 目标

把 `assistant/tool/user(observe)` 消息统一映射到同一 `ToolViewState`，消除多条冲突展示。

### 主要文件

- `src/composables/workflow/useWorkflowMessages.ts`
- `src/stores/workflow.js`

### 具体动作

1. 将当前多轮 `filter/map` 的工具态推断逻辑下沉为“先归一化后渲染”两阶段：
   - 阶段1：归一化到 `ToolViewState`
   - 阶段2：按 `ToolViewState` 渲染
2. 明确优先级（高 -> 低）：
   - `final_error/final_success`
   - `rejected`
   - `approved_running`
   - `pending`
3. 任何时候同一 `tool_call_id` 只允许一个展示状态。

### 交付物

- 一个纯函数：`deriveToolViewState(messages, toolStreams) -> Map`
- 单测或最小断言：同 `tool_call_id` 不会产生三条冲突态

---

## 3.3 Task C：Task Ledger UI（列表 + 状态 + 跳转）

### 目标

提供可读的任务账本面板，不扫描 transcript 文本。

### 主要文件

- `src/components/workflow/StatusPanel.vue`
- `src/components/workflow/WorkflowMessageList.vue`
- `src/styles/workflow/messages.scss`
- （必要时新增）`src/components/workflow/TaskLedger.vue`

### 具体动作

1. 任务列表项最少字段：
   - `tool_call_id`
   - `tool_name`
   - `status`
   - `summary`
   - `updated_at`
2. 增加状态徽标（pending/running/rejected/final）。
3. 点击账本项可定位到消息列表对应条目（滚动或高亮）。
4. 当无数据时显示空态，不回退为 transcript 解析。

### 交付物

- 可交互账本面板（不阻塞主聊天区域）
- 状态色与图标规范（scss 变量）

---

## 3.4 Task D：Inspector（结构化详情面板）

### 目标

展示结构化详情（arguments、error_type、approval_status、stream 摘要），避免原始 JSON 噪声。

### 主要文件

- `src/components/workflow/WorkflowMessageList.vue`
- `src/components/workflow/StatusPanel.vue`
- （可新增）`src/components/workflow/TaskInspector.vue`

### 具体动作

1. Inspector 输入只能来自 `ToolViewState`。
2. 字段分组：
   - Execution（status、updated_at）
   - Tool（name、arguments 摘要）
   - Approval（pending/approved/rejected）
   - Result（summary/error_type）
3. 长文本默认折叠，避免渲染性能抖动。

### 交付物

- Inspector 组件
- 至少 3 种状态样例（pending、rejected、final_error）

---

## 3.5 Task E：会话切换隔离与容错

### 目标

多会话切换不串台；UI 渲染异常不影响 workflow 执行。

### 主要文件

- `src/stores/workflow.js`
- `src/composables/workflow/useWorkflowCore.ts`
- `src/views/Workflow.vue`

### 具体动作

1. 将 `tool view state` 与 `currentWorkflowId` 强绑定；切换 workflow 时清理/重建本地索引。
2. 对面板渲染加入错误边界（或 try/catch + fallback 组件）。
3. 事件监听异常时只影响展示层，不影响 `workflow_signal` 命令。

### 交付物

- 切换会话后账本数据不串台的手测记录
- UI 故障时的降级日志（warning），且流程继续

---

## 3.6 Task F：后端结构化字段补齐（仅必要最小）

### 目标

若前端所需字段缺失，仅补“结构化暴露”，不改执行语义。

### 主要文件（按需）

- `src-tauri/src/workflow/react/types.rs`
- `src-tauri/src/workflow/react/events.rs`
- `src-tauri/src/commands/workflow.rs`
- `src-tauri/src/db/workflow.rs`

### 具体动作

1. 仅在快照或事件中补齐前端缺字段（如 `updated_at`、`tool_name` 标准化）。
2. 不改 `WorkflowState` 迁移规则，不改 approval 判定语义。
3. 新字段需向后兼容（`Option<T>`）。

### 交付物

- 字段兼容说明（新增字段、默认值、旧数据行为）

---

## 4. 推荐执行顺序（可并行）

1. Task A（模型）
2. Task B（映射收敛）
3. Task C（账本 UI）
4. Task D（Inspector）
5. Task E（会话隔离与容错）
6. Task F（按需后端补字段）

并行建议：

- 一组 AI 负责 A+B（数据层）
- 一组 AI 负责 C+D（UI 层）
- 一组 AI 负责 E（稳定性）+ F（必要后端字段）

---

## 5. 验收标准（DoD）

1. 同一 `tool_call_id` 在任一时刻只显示一个状态。
2. transcript 注入噪声文本不会改变任务账本状态。
3. 高频 `tool_stream` 更新不产生重复 pending/final 冲突条目。
4. 会话切换后账本不串台。
5. 人为制造 UI 渲染异常后，workflow 仍能继续运行并接收 signal。

---

## 6. 测试清单（可直接执行）

### 6.1 自动化最低要求

1. `useWorkflowMessages` 映射单测：同 `tool_call_id` 状态收敛。
2. store 单测：`appendToolStream -> approve -> final` 顺序一致性。
3. 组件测试：TaskLedger 渲染和点击定位。

### 6.2 手工验收

1. 触发一个需审批工具，验证 pending -> approved_running -> final 连续收敛。
2. 触发 reject，验证 rejected 保留且不会被后续无关消息覆盖。
3. 在执行中快速发送多条用户消息，验证队列显示与最终落库一致。
4. 切换到另一 session 再切回，验证账本与 inspector 正确恢复。
5. 人工让 Inspector 渲染报错（mock 无效字段），验证主流程不中断。

---

## 7. 回退策略

1. 保留原始 `messages` 渲染开关（feature flag）：
   - `workflow.ui.taskLedgerEnabled=false` 时回到旧视图。
2. 新增字段全部可选，后端可回滚不影响旧前端启动。
3. 任何 UI 问题优先降级展示，不回滚执行内核。

---

## 8. 给执行 AI 的任务模板（可复制）

你可以把下面模板直接发给其他 AI：

1. 目标：实现阶段9 Task A+B，建立 `tool_call_id` 统一视图模型并完成状态收敛。
2. 允许改动文件：`src/stores/workflow.js`, `src/composables/workflow/useWorkflowMessages.ts`, `src/composables/workflow/useWorkflowCore.ts`。
3. 禁止事项：禁止改动 Rust 执行状态机；禁止 transcript 文本推断任务态。
4. 交付要求：
   - 提供变更文件列表
   - 提供状态收敛规则
   - 提供最少 2 个测试结果（自动化或可复现手测）
5. 验收：同一 `tool_call_id` 不出现冲突状态条目。

