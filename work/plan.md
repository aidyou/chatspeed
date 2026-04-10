# 工作流重构实施计划

本文档是当前工作流模块的直接实施说明，目标是让任何接手的 AI 或开发者在不了解历史讨论的前提下，也能基于仓库现状直接开始重构工作。

本文档不讨论是否要重构。默认结论已经成立：

- 当前工作流实现存在“持久化状态”和“运行时状态”分裂。
- 当前恢复机制过度依赖 `workflow_messages` 中的历史文本和 JSON 反推。
- 当前信号路由和 session 生命周期与 UI 页面状态耦合过深。
- 当前状态机、等待逻辑、审批逻辑分散在多个代码路径中，维护成本高。

本次重构允许破坏性更新，不要求兼容旧版本运行态或旧版未发布数据，但要求：

- 每个开发阶段都可运行。
- 每个开发阶段都可测试。
- 每个开发阶段都可观测。
- 每个开发阶段都可回退到上一阶段稳定点。

---

## 1. 现状锚点

在开始任何编码前，必须先理解当前关键代码位置。以下是本次重构的核心锚点文件。

### 后端入口与命令层

- [src-tauri/src/commands/workflow.rs](src-tauri/src/commands/workflow.rs)
- 当前命令层负责：
  - `create_workflow`
  - `get_workflow_snapshot`
  - `workflow_start`
  - `workflow_signal`
  - workflow 继续执行和 approval 恢复

### 执行器

- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- 当前问题集中在：
  - `signal_rx`
  - `pending_approvals`
  - `run_loop_internal`
  - `init_internal`
  - `update_state`

### 网关

- [src-tauri/src/workflow/react/gateway.rs](src-tauri/src/workflow/react/gateway.rs)
- 当前 `TauriGateway` 既负责 UI 事件发送，也负责输入注入，是后续解耦重点。

### 子任务与后台任务

- [src-tauri/src/workflow/react/orchestrator.rs](src-tauri/src/workflow/react/orchestrator.rs)
- 当前通过 `BACKGROUND_TASKS` 管理后台 executor 和 shell 任务。

### 数据库

- [src-tauri/src/db/workflow.rs](src-tauri/src/db/workflow.rs)
- [src-tauri/src/db/sql/migrations/v5.rs](src-tauri/src/db/sql/migrations/v5.rs)
- 当前数据库只提供：
  - `workflows`
  - `workflow_messages`
- 这两张表不足以承载可靠恢复。

### 前端工作流接入

- [src/composables/workflow/useWorkflowCore.ts](src/composables/workflow/useWorkflowCore.ts)
- 当前前端基于 `workflow://event/{session_id}` 监听事件，并直接依赖旧状态字符串。

---

## 2. 重构目标

本次重构的首要目标不是“增加更多能力”，而是把工作流模块变成一个可靠的执行内核。

### 必达目标

- 后端统一管理 workflow session 生命周期。
- 用户输入、审批、继续执行都通过统一信号入口处理。
- 等待态是结构化状态，不再通过历史消息文本推断。
- 页面刷新后，活跃 workflow 不丢失。
- 程序重启后，waiting 状态可恢复。
- approval 恢复不再解析旧 transcript 中的 JSON。
- 为后续 event replay 提供基础存储能力。

### 非首轮目标

以下内容不作为第一里程碑目标，不应抢占内核工作：

- 多代理复杂 UI
- Inspector Tabs
- Task Tiles
- 任务账本的完整语义层
- 跨任务记忆恢复
- 完整事件溯源回放所有中间 chunk
- 完整 handoff 体系

---

## 3. 设计原则

### 原则 1：Session 生命周期只归后端管理

前端窗口只负责订阅和发信号，不负责维持 session 活着。

### 原则 2：结构化状态优先于 transcript

所有影响恢复的内容都必须进入结构化状态或结构化事件。

### 原则 3：等待点统一建模

等待用户输入、等待审批、等待继续执行、等待子任务完成，必须进入统一等待模型。

### 原则 4：先 snapshot，后 replay

不要一开始就做全量 event sourcing。第一轮先把 snapshot 做正确，再补 replay。

### 原则 5：先做单 session 可靠性，再做多代理控制权

没有稳定的 session、snapshot、waiting 恢复，就不要提前做 handoff 和复杂 UI。

### 原则 6：每一阶段必须可验收

任何阶段都必须定义：

- 代码边界
- 数据结构变化
- 日志与指标
- 手工验证步骤
- 自动化测试范围
- 回退方式

---

## 4. 目标架构总览

### 4.1 第一阶段后的核心架构

第一里程碑完成后，目标架构如下：

1. Tauri 命令层接收创建、启动、信号、查询请求。
2. `WorkflowManager` 成为 workflow session 的唯一注册中心。
3. 每个 session 在后端有明确的运行态记录。
4. `ExecutionContext` 作为最小恢复快照写入 DB。
5. 前端通过 `get_workflow_snapshot` 获取初始态，通过 Tauri event 收实时更新。
6. approval / user input / continue 都由统一 signal 入口处理。

### 4.2 第二阶段后的扩展架构

后续阶段再增加：

1. `workflow_events` 结构化事件表
2. replay reducer
3. 多 sink 分发
4. 子任务 call
5. handoff
6. UI 增强

---

## 5. 统一术语

### Workflow Session

一个完整工作流执行实例，对应一个 `session_id`。

### Executor

执行 workflow 主循环的运行体，当前由 ReAct executor 承担。

### WorkflowManager

后端全局注册中心。负责管理 session 与 executor 的映射、信号路由、状态查询。

### ExecutionContext

可持久化的最小运行态快照，不等于完整 transcript，不等于完整 event history。

### Waiting Point

workflow 暂停自动执行，等待外部信号的状态点。

### Signal

来自用户、UI 或系统的结构化恢复命令。

### Event

影响恢复和审计的结构化业务事件。

### Snapshot

某个 session 在某个时刻的结构化上下文序列化结果。

---

## 6. 第一里程碑范围

第一里程碑只包含阶段 0 到阶段 3。

### 第一里程碑交付后，系统必须满足

- `WorkflowManager` 接管 session 生命周期。
- session 不依赖 UI 页面存活。
- waiting 状态被结构化保存。
- approval 请求和恢复不再依赖消息反推。
- 用户输入和 approval 通过统一 signal 入口流转。
- 前端等待态逻辑可以基于 `state + wait_reason` 工作。

### 第一里程碑明确不做

- replay
- handoff
- tasks ledger
- inspector
- tabs
- 子任务树状上下文恢复

---

## 7. 阶段 0：重构基线与观测补齐

### 7.1 阶段目标

建立当前系统的运行基线，把现有问题变成可复现、可记录、可比较的对象。

### 7.2 需要修改的文件

- [src-tauri/src/commands/workflow.rs](src-tauri/src/commands/workflow.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/workflow/react/gateway.rs](src-tauri/src/workflow/react/gateway.rs)
- 必要时新增一份内部文档，例如：
  - `work/current-workflow-runtime.md`

### 7.3 具体任务

1. 为以下路径增加统一日志前缀：
   - workflow create
   - workflow start
   - workflow resume
   - workflow signal
   - workflow stop
   - approval request
   - approval resume
   - executor crash
2. 增加 `session_id`、`state`、`signal_type`、`tool_call_id` 等结构化字段。
3. 梳理当前 `WorkflowState` 的实际使用点，输出一张状态流转表。
4. 补充一组最小手工测试场景。
5. 若仓库已有测试基础，新增至少一个 workflow 恢复相关测试文件。

### 7.4 建议日志格式

统一使用类似格式：

```text
[Workflow][session={session_id}][phase={phase}] message...
```

关键字段：

- `session`
- `state`
- `wait_reason`
- `signal_type`
- `tool_call_id`
- `source`

### 7.5 必测场景

1. 创建 workflow 并执行普通对话。
2. 进入 `awaiting_approval` 后刷新页面。
3. 进入 `awaiting_user` 后发送补充消息。
4. 在 waiting 时发送 stop。
5. 活跃 workflow 执行时关闭前端窗口并重新打开。

### 7.6 验收标准

- 每个关键路径都能在日志中串起来。
- `signal channel closed` 的出现路径可被完整定位。
- 能明确确认哪些状态是真正需要保留的。

### 7.7 回退策略

- 仅新增日志和测试，不改变功能行为。

---

## 8. 阶段 1：引入 WorkflowManager

### 8.1 阶段目标

把 session 生命周期从命令层和 `BACKGROUND_TASKS` 中抽离，交给一个后端全局管理器。

### 8.2 新增模块建议

建议新增：

- [src-tauri/src/workflow/react/manager.rs](src-tauri/src/workflow/react/manager.rs)

若模块划分较大，可拆成：

- `manager.rs`
- `manager/types.rs`
- `manager/errors.rs`

### 8.3 WorkflowManager 最小职责

必须实现以下职责：

1. 注册 session executor
2. 查询 session 是否活跃
3. 获取 session 当前运行态引用
4. 路由 signal 到指定 session
5. 标记 session 结束
6. 提供 session 的只读状态查询接口

### 8.4 最小数据结构建议

```rust
pub struct WorkflowManager {
    sessions: DashMap<String, ManagedSession>,
}

pub struct ManagedSession {
    session_id: String,
    executor: Arc<tokio::sync::Mutex<dyn ReActExecutor>>,
    created_at: i64,
    updated_at: i64,
    status: ManagedSessionStatus,
}

pub enum ManagedSessionStatus {
    Active,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}
```

### 8.5 现有代码替换点

本阶段必须替换下列逻辑：

- `workflow_start` 中直接注册到 `BACKGROUND_TASKS` 的逻辑
- `workflow_signal` 中直接判断 session 是否存在的逻辑
- 各类 resume 分支中“先尝试注入、失败后再重启”的拼接逻辑

### 8.6 实施步骤

1. 新建 `WorkflowManager`。
2. 在 `src-tauri/src/lib.rs` 注册为全局 `State`。
3. 修改 `workflow_start`：
   - 创建 executor
   - 调用 manager 注册
   - 由 manager 返回是否成功
4. 修改 `workflow_signal`：
   - 先走 manager 查询
   - 活跃则直接路由
   - 不活跃则决定是否进入恢复流程
5. 保留旧 `BACKGROUND_TASKS` 仅作为过渡兼容。

### 8.7 不做事项

- 不改 executor 主循环。
- 不改 approval 内部状态来源。
- 不引入 snapshot 表。

### 8.8 可观测性

必须新增日志：

- `workflow_manager.session_registered`
- `workflow_manager.session_removed`
- `workflow_manager.signal_routed`
- `workflow_manager.signal_rejected`
- `workflow_manager.session_lookup_hit`
- `workflow_manager.session_lookup_miss`

### 8.9 自动化测试建议

至少补以下测试：

1. 注册一个 session 后可查询到。
2. 已结束 session 不可再路由普通 signal。
3. 活跃 session 可以成功注入 signal。
4. session 移除后 manager 查询不到。

### 8.10 手工验收步骤

1. 启动 workflow。
2. 刷新前端页面。
3. 再次发送消息或审批。
4. 验证不依赖页面监听也能找到活跃 session。

### 8.11 完成定义

满足以下条件才算阶段完成：

- `workflow_signal` 已通过 manager 路由。
- session 生命周期不再由命令层零散管理。
- 日志能看出每个 session 的注册、更新、移除。

### 8.12 回退策略

- 保留旧 `BACKGROUND_TASKS` 查询支路，通过 feature flag 或临时 fallback 使用。

---

## 8A. 第一阶段详细执行清单

本节是阶段 1 的直接施工说明。目标是让接手者不需要再次拆任务，可以直接按步骤修改代码。

第一阶段的唯一目标是：

- 把 session 生命周期统一收敛到 `WorkflowManager`
- 不动 executor 主循环核心行为
- 不提前引入 snapshot / replay / reviewer / handoff

如果接手者发现自己准备修改 waiting 逻辑、approval 内部状态、snapshot 表，说明已经越界，必须停下。

### 8A.1 第一阶段边界

本阶段允许修改：

- [src-tauri/src/lib.rs](src-tauri/src/lib.rs)
- [src-tauri/src/commands/workflow.rs](src-tauri/src/commands/workflow.rs)
- [src-tauri/src/workflow/react/orchestrator.rs](src-tauri/src/workflow/react/orchestrator.rs)
- [src-tauri/src/workflow/react/gateway.rs](src-tauri/src/workflow/react/gateway.rs)
- 新增：
  - [src-tauri/src/workflow/react/manager.rs](src-tauri/src/workflow/react/manager.rs)

本阶段禁止修改：

- `engine.rs` 的等待模型重构
- `WorkflowState` 的结构重定义
- DB schema
- 前端状态协议
- approval 恢复机制

### 8A.2 第一阶段交付目标

阶段完成后，必须满足：

1. 所有活跃 workflow session 都注册在 `WorkflowManager`
2. `workflow_signal` 优先通过 `WorkflowManager` 查找和路由
3. session 注册、查询、移除、信号注入都有统一日志
4. 命令层不再自己散落维护 session 生命周期

### 8A.3 推荐实现顺序

必须按下面顺序实现，不建议跳步：

1. 新建 `manager.rs` 和最小类型
2. 在 `lib.rs` 注册 `WorkflowManager` 为全局状态
3. 修改 `workflow_start` 使用 manager 注册 session
4. 修改 workflow 执行结束路径，使用 manager 移除 session
5. 修改 `workflow_signal` 通过 manager 路由 signal
6. 保留 `BACKGROUND_TASKS` 作为兼容 fallback
7. 补自动化测试
8. 做手工验收

### 8A.4 manager.rs 第一版建议结构

建议第一版只做一个文件，不要过早拆太多子模块。

建议结构：

```rust
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct WorkflowManager {
    sessions: DashMap<String, ManagedSession>,
}

pub struct ManagedSession {
    pub session_id: String,
    pub executor: Arc<Mutex<dyn ReActExecutor>>,
    pub status: ManagedSessionStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub enum ManagedSessionStatus {
    Active,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}
```

第一版不要把 manager 设计得太复杂。不要在这一阶段引入：

- snapshot 缓存
- event 广播
- 子任务树
- review gate

### 8A.5 WorkflowManager 第一版最小接口

建议至少实现这些方法：

```rust
impl WorkflowManager {
    pub fn new() -> Self;

    pub fn register_session(
        &self,
        session_id: String,
        executor: Arc<Mutex<dyn ReActExecutor>>,
        status: ManagedSessionStatus,
    ) -> Result<(), WorkflowManagerError>;

    pub fn remove_session(&self, session_id: &str) -> Option<ManagedSession>;

    pub fn has_session(&self, session_id: &str) -> bool;

    pub fn get_session_status(&self, session_id: &str) -> Option<ManagedSessionStatus>;

    pub fn get_executor(
        &self,
        session_id: &str,
    ) -> Option<Arc<Mutex<dyn ReActExecutor>>>;

    pub fn update_session_status(
        &self,
        session_id: &str,
        status: ManagedSessionStatus,
    );
}
```

第一阶段不要试图把 `signal_tx` 直接放进 manager，如果当前信号路由仍然依赖 gateway，也允许先让 manager 只持有 executor 引用和状态。

### 8A.6 lib.rs 修改要求

在 [src-tauri/src/lib.rs](src-tauri/src/lib.rs) 中：

1. 初始化 `WorkflowManager`
2. 将其注册为全局 `State`
3. 传入所有需要的 workflow 命令

建议风格与现有 `TauriGateway`、`TsidGenerator`、`ChatState` 一致。

### 8A.7 workflow_start 改造清单

在 [src-tauri/src/commands/workflow.rs](src-tauri/src/commands/workflow.rs) 中，`workflow_start` 必须做以下调整：

1. 增加 `WorkflowManager` 的 `State` 参数
2. 创建 executor 后，先调用 manager 注册
3. 注册成功后再启动后台任务
4. 保留 `BACKGROUND_TASKS.insert(...)` 作为过渡
5. 后台任务退出时：
   - 记录退出原因
   - 调用 manager 移除 session
   - 再移除 `BACKGROUND_TASKS`

关键要求：

- manager 是主注册表
- `BACKGROUND_TASKS` 只是兼容层

### 8A.8 workflow_signal 改造清单

在 [src-tauri/src/commands/workflow.rs](src/commands/workflow.rs) 不是正确路径，应修改：

- [src-tauri/src/commands/workflow.rs](src-tauri/src/commands/workflow.rs)

`workflow_signal` 改造目标：

1. 优先通过 manager 查询 session
2. 若 session 活跃：
   - 直接走现有 signal 注入路径
3. 若 session 不活跃：
   - 记录 `session_lookup_miss`
   - 再进入现有恢复逻辑

第一阶段不要重写恢复逻辑本身。只需要把“是否存在活跃 session”的判断统一到 manager。

### 8A.9 orchestrator.rs 处理要求

[src-tauri/src/workflow/react/orchestrator.rs](src-tauri/src/workflow/react/orchestrator.rs) 中现有 `BACKGROUND_TASKS` 不能在第一阶段直接删除。

要求如下：

- 保留 `BACKGROUND_TASKS`
- 明确注释其角色已经降级为兼容层
- 后续阶段再考虑是否拆分为：
  - workflow manager registry
  - background utility tasks registry

本阶段不要尝试一次性把 shell task、workflow task、sub-agent task 全部统一掉。

### 8A.10 gateway.rs 处理要求

本阶段 [src-tauri/src/workflow/react/gateway.rs](src-tauri/src/workflow/react/gateway.rs) 只做最小修改：

- 增加更明确的日志
- 保持现有 `inject_input` 语义
- 不改现有 Tauri event 聚合行为

不要在第一阶段就重写 gateway。

### 8A.11 日志清单

第一阶段必须新增以下日志：

- `workflow_manager.session_registered`
- `workflow_manager.session_removed`
- `workflow_manager.session_lookup_hit`
- `workflow_manager.session_lookup_miss`
- `workflow_manager.signal_routed`
- `workflow_manager.signal_rejected`

建议输出字段：

- `session_id`
- `status`
- `source_command`
- `signal_type`
- `reason`

### 8A.12 自动化测试清单

建议新增测试文件：

- `src-tauri/src/workflow/react/manager.rs` 同文件单元测试
- 或单独新增：
  - `src-tauri/src/workflow/react/manager_tests.rs`

最少覆盖：

1. `register_session` 后 `has_session == true`
2. `remove_session` 后 `has_session == false`
3. `get_executor` 能返回正确 executor 引用
4. 重复注册同一个 `session_id` 时的行为明确
5. `update_session_status` 后状态可读

如果当前代码结构不方便直接构造真实 executor，可先用最小 mock executor。

### 8A.13 手工验收脚本

接手者完成第一阶段后，必须按以下步骤手工验证：

1. 启动应用
2. 创建一个 workflow
3. 启动执行
4. 检查日志中是否出现 `session_registered`
5. 刷新 workflow 页面
6. 发送一条普通消息或 approval 信号
7. 检查日志中是否优先命中 `session_lookup_hit`
8. 结束 workflow
9. 检查日志中是否出现 `session_removed`

### 8A.14 常见错误

第一阶段最容易犯的错误有：

1. 试图在 manager 中直接重建所有恢复逻辑
2. 试图删除 `BACKGROUND_TASKS`
3. 试图改动 executor 内部 waiting 逻辑
4. 试图引入新的数据库表
5. 试图同步重写前端逻辑

发现这些倾向时，应立即收缩范围。

### 8A.15 第一阶段完成定义

只有满足以下全部条件，才算第一阶段完成：

1. `WorkflowManager` 已存在并注册为全局状态
2. `workflow_start` 已调用 manager 注册 session
3. `workflow_signal` 已优先通过 manager 判断 session 活跃性
4. workflow 结束时 manager 会移除 session
5. 自动化测试通过
6. 手工验收通过
7. 没有引入 snapshot、review gate、handoff 等超范围改动

### 8A.16 第一阶段后再做什么

第一阶段完成后，下一步只能进入以下两项之一：

1. 阶段 2：Snapshot 与 `ExecutionContext`
2. 补足第一阶段测试与日志遗漏

禁止在第一阶段后直接跳去做：

- reviewer 审核机制
- handoff
- event replay
- 任务账本

---

## 9. 阶段 2：引入最小 Snapshot 与 ExecutionContext

### 9.1 阶段目标

把运行态最小真相从内存抽出来，写入结构化快照，解决 waiting 恢复依赖 transcript 的问题。

### 9.2 数据库变更

新增表：

```sql
CREATE TABLE workflow_snapshots (
    session_id TEXT PRIMARY KEY,
    context_json TEXT NOT NULL,
    version TEXT NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);
```

如需额外字段，可增加：

- `last_event_id INTEGER`
- `state TEXT`
- `wait_reason TEXT`

但第一版优先保证 `context_json` 完整。

### 9.3 ExecutionContext 第一版字段

第一版不要做太大，只做最小恢复集：

```rust
pub struct ExecutionContext {
    pub session_id: String,
    pub state: RuntimeState,
    pub wait_reason: Option<WaitReason>,
    pub current_step: usize,
    pub max_steps: usize,
    pub pending_tools: Vec<PendingTool>,
    pub last_action_summary: Option<String>,
    pub version: String,
}

pub enum RuntimeState {
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

pub enum WaitReason {
    Confirmation,
    UserInput,
    Approval,
}

pub struct PendingTool {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub details: Option<String>,
}
```

### 9.4 新增文件建议

- `src-tauri/src/workflow/react/context_snapshot.rs`
- 或在 `context.rs` 基础上拆出 snapshot 专用模块

### 9.5 实施步骤

1. 增加 migration。
2. 在 DB 层增加：
   - `get_workflow_execution_context`
   - `upsert_workflow_execution_context`
   - `delete_workflow_execution_context`
3. 在 executor 中新增：
   - 从运行态导出 `ExecutionContext`
   - 从 `ExecutionContext` 恢复最小状态
4. 在以下时机写 snapshot：
   - 进入 waiting
   - approval 请求完成后
   - user input 被接收后
   - state 进入 completed
   - state 进入 failed
   - state 进入 cancelled
5. `pending_approvals` 改为从 `ExecutionContext.pending_tools` 构建。

### 9.6 关键设计要求

- snapshot 必须是“事实来源”，不能只是日志副本。
- `pending_tools` 必须是结构化对象，不能存原始消息字符串。
- approval 恢复必须优先使用 snapshot，而不是扫描 `workflow_messages`。

### 9.7 不做事项

- 不做 replay。
- 不做 event store。
- 不做多代理字段。

### 9.8 可观测性

新增日志：

- `workflow.snapshot.write`
- `workflow.snapshot.read`
- `workflow.snapshot.restore`
- `workflow.snapshot.fallback_legacy`

每次写 snapshot 必须带：

- `session_id`
- `state`
- `wait_reason`
- `pending_tool_count`
- `snapshot_version`

### 9.9 自动化测试建议

至少补以下测试：

1. approval 请求生成后，snapshot 中有对应 pending tool。
2. 从 snapshot 恢复时能重建 waiting + approval。
3. 没有历史消息扫描时，仍可恢复审批请求。
4. completed 状态 snapshot 可被正确读取。

### 9.10 手工验收步骤

1. 创建 workflow。
2. 触发需要审批的工具调用。
3. 在 approval 弹窗出现时关闭页面。
4. 重新打开页面。
5. 调用初始化接口和事件重放逻辑。
6. 验证 approval 信息来自 snapshot，不来自消息反推。

### 9.11 完成定义

满足以下条件才算阶段完成：

- approval 恢复不再依赖 `workflow_messages` 解析。
- waiting 状态可在应用重启后恢复。
- `ExecutionContext` 成为恢复的主要结构。

### 9.12 回退策略

- 保留旧消息反推逻辑作为临时 fallback。
- fallback 触发时必须打印废弃日志，便于后续彻底移除。

---

## 9A. 第二阶段详细执行清单

第二阶段只做一件事：把最小运行态变成结构化 snapshot。不要在这一阶段提前做 event store、replay 或 reviewer。

### 9A.1 允许修改的文件

- [src-tauri/src/db/sql/migrations](src-tauri/src/db/sql/migrations)
- [src-tauri/src/db/workflow.rs](src-tauri/src/db/workflow.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/workflow/react/context.rs](src-tauri/src/workflow/react/context.rs)
- 新增：
  - `src-tauri/src/workflow/react/context_snapshot.rs`

### 9A.2 推荐实现顺序

1. 增加 migration
2. 在 DB 层增加 snapshot 读写 API
3. 定义 `ExecutionContext`
4. 增加运行态到 snapshot 的导出逻辑
5. 在 waiting / terminal 关键点写 snapshot
6. 将 approval 恢复切换到 snapshot
7. 保留旧消息反推 fallback
8. 补测试

### 9A.3 本阶段禁止事项

- 不做 event store
- 不做 replay
- 不改前端状态协议
- 不引入多代理字段
- 不引入 review gate

### 9A.4 必须覆盖的恢复点

- 进入 approval 等待后
- approval 响应后
- user input 等待后
- completed
- failed
- cancelled

### 9A.5 自动化测试最低要求

1. snapshot 写入成功
2. snapshot 读取成功
3. pending tools 能正确 round-trip
4. approval 恢复优先走 snapshot
5. snapshot 缺失时仍可 fallback 到旧逻辑

### 9A.6 手工验收

1. 触发 approval
2. 页面刷新
3. 验证 approval 可恢复
4. 重启应用
5. 验证 waiting 态仍可恢复

### 9A.7 完成定义

- 结构化 snapshot 已成为 waiting 恢复主路径
- 旧 transcript 反推仅作为 fallback

---

## 10. 阶段 3：统一等待模型与结构化 Signal

### 10.1 阶段目标

把当前散落在 `Paused`、`AwaitingUser`、`AwaitingApproval`、零散 signal JSON 分支里的逻辑，统一为一个等待模型和一套结构化 signal。

### 10.2 目标状态模型

顶层状态：

- `pending`
- `running`
- `waiting`
- `completed`
- `failed`
- `cancelled`

正交字段：

- `wait_reason`
  - `confirmation`
  - `user_input`
  - `approval`
  - `child_task`
- `resume_policy`
  - `manual`
  - `signal`
  - `child_done`

### 10.3 Signal 第一版模型

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowSignal {
    UserMessage {
        content: String,
    },
    ApprovalDecision {
        tool_call_id: String,
        approved: bool,
        approve_all: bool,
    },
    Continue,
    Stop,
    RebroadcastPending,
    RemoveAutoApprovedTool {
        tool_name: String,
    },
    RemoveShellPolicyItem {
        pattern: String,
    },
}
```

### 10.4 必改代码点

- `workflow_signal` 命令层输入解析
- `engine.rs` 中 waiting 状态下的 signal 分支处理
- `useWorkflowCore.ts` 中前端等待态和 approval 相关判断

### 10.5 实施步骤

1. 定义 `WorkflowSignal` 枚举。
2. 命令层将传入 JSON 统一解析为 `WorkflowSignal`。
3. executor 只接受结构化 signal，而不是任意字符串 JSON。
4. 统一 waiting 入口：
   - 进入 waiting 时写 snapshot
   - 等待 signal
   - 校验 signal 是否匹配当前 `wait_reason`
   - 合法则恢复，不合法则忽略或记录错误
5. 前端更新逻辑改为基于：
   - `state === waiting`
   - `wait_reason === approval | user_input | confirmation`

### 10.6 前端改造要求

前端不要再直接依赖多个旧状态值做业务判断。必须收敛为：

- 是否运行中
- 是否等待中
- 当前等待原因

必要时在后端暂时保留旧状态到新状态的映射层，但前端新逻辑应优先使用新字段。

### 10.7 可观测性

新增日志：

- `workflow.wait.enter`
- `workflow.wait.resume`
- `workflow.wait.signal_received`
- `workflow.wait.signal_rejected`
- `workflow.wait.signal_mismatch`

### 10.8 自动化测试建议

至少补以下测试：

1. `approval` 只接受 `ApprovalDecision`。
2. `user_input` 等待时收到 `Continue` 会被拒绝。
3. `confirmation` 等待时收到 `UserMessage` 会被拒绝或延后处理。
4. `Stop` 在任何等待态都能生效。

### 10.9 手工验收步骤

1. 让 workflow 进入 `approval` waiting。
2. 发送错误类型 signal，验证不会错误恢复。
3. 发送正确 approval，验证恢复成功。
4. 让 workflow 进入 `user_input` waiting。
5. 发送补充消息，验证恢复成功。

### 10.10 完成定义

满足以下条件才算阶段完成：

- waiting 态只走一套统一主逻辑。
- signal 通过结构化类型分发。
- 前端可基于 `state + wait_reason` 判断交互状态。

### 10.11 回退策略

- 命令层保留旧 JSON 输入兼容解析，但内部立即转换为 `WorkflowSignal`。

---

## 10A. 第三阶段详细执行清单

第三阶段的目标不是“简化所有状态”，而是把等待行为收敛到一条主链路。

### 10A.1 允许修改的文件

- [src-tauri/src/workflow/react/types.rs](src-tauri/src/workflow/react/types.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/commands/workflow.rs](src-tauri/src/commands/workflow.rs)
- [src/composables/workflow/useWorkflowCore.ts](src/composables/workflow/useWorkflowCore.ts)

### 10A.2 推荐实现顺序

1. 定义 `WorkflowSignal`
2. 命令层统一解析 signal
3. 在 engine 内集中 waiting 分支
4. 给 waiting 增加 `wait_reason`
5. 前端改为基于 `state + wait_reason`
6. 保留旧状态兼容映射

### 10A.3 本阶段禁止事项

- 不做 snapshot schema 变化
- 不做 event store
- 不做 handoff
- 不做 reviewer

### 10A.4 关键实现要求

- 所有等待信号必须做类型匹配
- 非法 signal 不能 silently continue
- `Stop` 必须在任何等待态都可生效

### 10A.5 自动化测试最低要求

1. waiting + approval 接收正确 signal
2. waiting + approval 拒绝错误 signal
3. waiting + user_input 接收用户消息
4. waiting + confirmation 接收继续/停止

### 10A.6 手工验收

1. 进入 approval
2. 发错 signal
3. 验证 workflow 不错误恢复
4. 发对 signal
5. 验证恢复

### 10A.7 完成定义

- waiting 只走一条统一主链路
- 前端不再硬编码多个旧等待状态

---

## 11. 阶段 4：引入结构化 Event Store

### 11.1 阶段目标

建立结构化事件表，为审计、问题排查和后续 replay 提供基础。

### 11.2 数据库变更

在 src-tauri/src/db/sql/migrations/v5.rs 新增表：

```sql
CREATE TABLE workflow_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    event_version TEXT NOT NULL,
    event_data TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX idx_workflow_events_session_id_id
ON workflow_events(session_id, id);
```

如后续需要，可在第二版增加：

- `task_id`
- `agent_id`

但第一版不要提前引入。

### 11.3 事件原则

- 只记录 domain event。
- 不记录流式 chunk。
- 不记录纯 UI 展示型事件。
- 事件设计必须支持 replay，但本阶段不实现 replay。

### 11.4 首批事件清单

- `WorkflowStarted`
- `StateChanged`
- `WaitEntered`
- `UserInputReceived`
- `ApprovalRequested`
- `ApprovalResolved`
- `ToolStarted`
- `ToolCompleted`
- `ToolFailed`
- `WorkflowCompleted`
- `WorkflowFailed`
- `WorkflowCancelled`

### 11.5 事件结构建议

```rust
pub struct WorkflowEventRecord {
    pub id: i64,
    pub session_id: String,
    pub event_type: String,
    pub event_version: String,
    pub event_data: serde_json::Value,
    pub created_at: String,
}
```

### 11.6 实施步骤

1. 增加 migration。
2. 在 DB 层增加：
   - `append_workflow_event`
   - `list_workflow_events`
3. 在关键状态节点写事件。
4. 在 snapshot 中补 `last_event_id` 字段。
5. 增加调试接口或内部打印，方便按 session 查看事件序列。

### 11.7 可观测性

新增日志：

- `workflow.event.append`
- `workflow.event.append_failed`
- `workflow.event.sequence_gap`

### 11.8 自动化测试建议

1. workflow 执行后能查到事件序列。
2. waiting 和 approval 相关事件顺序正确。
3. event append 失败时不会 silent fail。

### 11.9 完成定义

- 任意 session 都能查到结构化关键事件链。
- snapshot 与 event 序列可以对齐到最近一个 `last_event_id`。

### 11.10 回退策略

- event store 只做追加写，不影响主执行链路。

---

## 11A. 第四阶段详细执行清单

第四阶段的目标是建立结构化事件审计链，不是完整 event sourcing。

### 11A.1 允许修改的文件

- [src-tauri/src/db/sql/migrations](src-tauri/src/db/sql/migrations)
- [src-tauri/src/db/workflow.rs](src-tauri/src/db/workflow.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- 新增：
  - `src-tauri/src/workflow/react/events.rs`

### 11A.2 推荐实现顺序

1. 增加 `workflow_events` migration
2. 定义事件 record 结构
3. 增加 DB append/list API
4. 从关键节点开始写事件
5. 增加 debug 查询方式

### 11A.3 本阶段禁止事项

- 不做 replay
- 不记录 thinking chunk
- 不让 event store 取代 transcript

### 11A.4 第一版必须写入的事件

- `WorkflowStarted`
- `StateChanged`
- `WaitEntered`
- `ApprovalRequested`
- `ApprovalResolved`
- `WorkflowCompleted`
- `WorkflowFailed`

### 11A.5 自动化测试最低要求

1. 事件序列顺序正确
2. event append 失败可见
3. `last_event_id` 可对齐

### 11A.6 完成定义

- 任意 session 可查询到可读的关键事件链

---

## 12. 阶段 5：引入 Snapshot + Event Replay 恢复

### 12.1 阶段目标

在 snapshot 缺失、损坏或版本不匹配时，通过事件流重建最小运行态。

### 12.2 不要做的事情

- 不要回放 thinking chunk。
- 不要试图用 replay 重建完整 UI transcript。
- 不要把 replay 设计成每次都从头扫描所有历史。

### 12.3 核心策略

1. 优先读取 snapshot。
2. snapshot 存在且版本兼容：
   - 从 snapshot 开始恢复
3. snapshot 缺失或不兼容：
   - 从事件流 reducer 重建
4. replay 失败：
   - 进入只读失败态，不允许继续执行

### 12.4 Reducer 责任

Reducer 只负责重建：

- `state`
- `wait_reason`
- `pending_tools`
- `current_step`
- `last_action_summary`
- `last_event_id`

### 12.5 建议新增模块

- `src-tauri/src/workflow/react/replay.rs`
- `src-tauri/src/workflow/react/events.rs`

### 12.6 可观测性

新增日志：

- `workflow.replay.start`
- `workflow.replay.done`
- `workflow.replay.failed`
- `workflow.restore.snapshot_hit`
- `workflow.restore.snapshot_miss`
- `workflow.restore.replay_fallback`

### 12.7 自动化测试建议

1. snapshot 存在时优先从 snapshot 恢复。
2. snapshot 删除后可从 event replay 恢复 waiting。
3. snapshot version 不匹配时自动切换 replay。
4. replay 失败时进入安全失败态。

### 12.8 完成定义

- waiting 和 terminal 状态都可由 replay 重建。
- 恢复链路有明确优先级和失败处理策略。

### 12.9 回退策略

- 可临时关闭 replay，只保留 snapshot 恢复。

---

## 12A. 第五阶段详细执行清单

第五阶段才开始做恢复链真正的“兜底”能力。

### 12A.1 允许修改的文件

- `src-tauri/src/workflow/react/replay.rs`
- `src-tauri/src/workflow/react/events.rs`
- [src-tauri/src/db/workflow.rs](src-tauri/src/db/workflow.rs)
- [src-tauri/src/workflow/react/manager.rs](src-tauri/src/workflow/react/manager.rs)

### 12A.2 推荐实现顺序

1. 先定义 reducer
2. 再定义 snapshot + event 的恢复优先级
3. 实现 replay fallback
4. 加恢复错误分类
5. 补测试

### 12A.3 本阶段禁止事项

- 不做 UI transcript replay
- 不做 chunk replay
- 不做多代理 replay

### 12A.4 自动化测试最低要求

1. snapshot 存在优先命中
2. snapshot 缺失走 replay
3. version mismatch 走 replay
4. replay 失败进入安全失败态

### 12A.5 完成定义

- 恢复链路已明确：snapshot first，replay fallback

---

## 13. 阶段 6：内存分发总线与多 Sink

### 13.1 阶段目标

把 UI 推送、DB 记录、外部网关输出从 executor 中拆出来，形成统一分发层。

### 13.2 关键约束

- DB 写入不能依赖 best-effort 广播语义。
- UI sink 可以容忍部分中间状态丢失，但不能丢失最终状态。
- executor 不能被慢 sink 长时间阻塞。

### 13.3 建议架构

推荐：

1. executor 产生内部事件
2. 进入 dispatcher
3. dispatcher 分发给：
   - DB sink
   - Tauri sink
   - 其他 sink

### 13.4 不推荐做法

- 不要把 DB sink 和 UI sink 完全建立在同一个 `broadcast` 丢包语义上。

### 13.5 可观测性

必须增加：

- 每个 sink 的队列长度
- 每个 sink 的处理延迟
- 丢弃事件计数
- lag 警告

### 13.6 完成定义

- UI 中断不影响 DB 恢复数据。
- DB 延迟不会直接拖死 executor。

### 13.7 防偏离补充（执行约束）

#### 13.7.1 主链接入判据（必须）

以下三项必须同时成立，才算“接入 dispatcher 主链”：

1. 运行时路径中可追踪到 `engine -> dispatcher -> sinks` 的真实调用链（非测试代码）。
2. UI sink 故障时，DB sink 仍持续写入事件/快照。
3. DB sink 慢速时，executor 主循环吞吐不出现级联阻塞。

#### 13.7.2 禁止伪完成

- 只新增 `dispatcher.rs/sinks.rs` 但 `engine` 未真正改为 dispatcher 发射，不算完成。
- 仅有模块单测通过，不算完成；必须有入口级集成测试覆盖 executor 实际分发。
- 仅日志打印 lag/dropped 而无可断言计数读取接口，不算完成。

#### 13.7.3 最小验收证据模板

至少提供：

1. 一组“UI sink 报错 + DB sink 正常写入”的日志样本。
2. 一组“DB sink 注入延迟 + workflow 最终完成”的日志样本。
3. 一条终态事件（completed/failed/cancelled）进入 DB 的断言输出。

---

## 13A. 第六阶段详细执行清单

第六阶段解决的是“输出分发”的工程稳定性，而不是业务状态机。

### 13A.1 允许修改的文件

- [src-tauri/src/workflow/react/gateway.rs](src-tauri/src/workflow/react/gateway.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- 新增：
  - `src-tauri/src/workflow/react/dispatcher.rs`
  - `src-tauri/src/workflow/react/sinks.rs`

### 13A.2 推荐实现顺序

1. 定义 dispatcher 接口
2. 定义 Tauri sink
3. 定义 DB sink
4. 将 executor 输出接入 dispatcher
5. 增加 lag / dropped 日志

### 13A.3 本阶段禁止事项

- 不改变恢复语义
- 不把 DB sink 建在不可靠广播上

### 13A.4 自动化测试最低要求

1. UI sink 失败不影响 DB sink
2. DB sink 变慢不阻断 executor 完成
3. 最终状态事件不丢失

### 13A.5 完成定义

- executor 输出与具体 sink 解耦

### 13A.6 允许修改文件最小闭环（补充）

为防止“模块新增但主链未接入”，阶段6建议最小闭环文件集合为：

- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)（主链接入点，必改）
- [src-tauri/src/workflow/react/gateway.rs](src-tauri/src/workflow/react/gateway.rs)
- `src-tauri/src/workflow/react/dispatcher.rs`
- `src-tauri/src/workflow/react/sinks.rs`
- 与 sink 指标读取相关的状态/命令暴露文件（如 `commands/workflow.rs`，按实际实现补齐）

---

## 14. 阶段 7：单层子任务 Call 模型

### 14.1 阶段目标

先支持父任务等待子任务完成，不先支持 handoff。

### 14.2 原则

- 第一版只做 `Call`
- 父任务仍持有主会话控制权
- 子任务结果以结构化输出返回父任务

### 14.3 新增字段建议

- `waiting_on_task_id`
- `child_sessions`

### 14.4 完成定义

- 父任务可等待子任务
- 子任务完成后父任务自动恢复
- 恢复后仍能知道父任务在等哪个子任务

### 14.6 防偏离补充（执行约束）

#### 14.6.1 主链接入判据（必须）

1. 父任务进入 `waiting + child_task` 必须由运行时状态机真实驱动，而非仅 UI 投影。
2. 子任务完成/失败/取消三条路径都能触发父任务收敛（恢复或失败决策）。
3. 重启恢复后，`waiting_on_task_id` 与 `child_sessions` 可从持久化结构重建。

#### 14.6.2 禁止伪完成

- 仅创建子任务 ID 但父任务未真正进入 waiting 态，不算完成。
- 仅成功路径可恢复、失败/取消路径悬挂，不算完成。
- 仅子任务模块单测通过，不算完成；必须有父子联动集成测试。

#### 14.6.3 最小验收证据模板

至少提供：

1. 父任务进入 waiting 的状态与字段快照证据。
2. 子任务 completed/failed/cancelled 三分支各一条恢复结果证据。
3. 重启后父任务仍定位到同一 `waiting_on_task_id` 的证据。

### 14.5 阶段 7 之后可接入的扩展：审核 Gate

审核机制必须建立在阶段 7 的单层 `Call` 能力之上。

原因：

- 审核本质上不是单独的前端功能，而是“主流程等待 reviewer 子任务结果”的标准工作流节点。
- 如果没有稳定的 `Call + Waiting + Snapshot`，审核机制会重新把复杂度带回命令层和 transcript 解析。
- 因此，审核机制不能放进第一里程碑，也不能早于阶段 7 落地。

推荐顺序：

1. 先完成阶段 0-3，打稳内核
2. 完成阶段 4-5，具备基本 snapshot / event 能力
3. 完成阶段 7，具备稳定的 reviewer 子任务等待模型
4. 再接入计划审核和完成审核

第一版审核机制不要做 handoff，只做 reviewer 子任务 `Call`。

---

## 14A. 第七阶段详细执行清单

第七阶段只做 `Call`，不要做 `Handoff`。

### 14A.1 允许修改的文件

- [src-tauri/src/workflow/react/orchestrator.rs](src-tauri/src/workflow/react/orchestrator.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/db/workflow.rs](src-tauri/src/db/workflow.rs)

### 14A.2 推荐实现顺序

1. 增加父子任务关联字段
2. 主任务进入 `Waiting + child_task`
3. 子任务完成后结构化回传
4. 父任务恢复
5. 增加恢复测试

### 14A.3 本阶段禁止事项

- 不做用户输入重新路由
- 不做焦点代理
- 不做 handoff stack

### 14A.4 自动化测试最低要求

1. 父任务等待子任务
2. 子任务完成后父任务恢复
3. 应用重启后父任务仍知道自己在等待子任务

### 14A.5 完成定义

- `Call` 模型稳定可恢复

### 14A.6 允许修改文件最小闭环（补充）

为防止“只改 orchestrator，未打通恢复链”，阶段7建议最小闭环文件集合为：

- [src-tauri/src/workflow/react/orchestrator.rs](src-tauri/src/workflow/react/orchestrator.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/db/workflow.rs](src-tauri/src/db/workflow.rs)
- [src-tauri/src/workflow/react/manager.rs](src-tauri/src/workflow/react/manager.rs)（若涉及恢复与会话映射）
- 相关命令入口文件（按实际实现补齐）

---

## 15. 阶段 8：Handoff 与焦点代理模型

> 执行顺序说明（2026-04-07 更新）：为提升可观测性与前端体感验证质量，阶段8改为在阶段9之后执行。  
> 即：先完成任务账本与高级UI（原阶段9），再实现Handoff与焦点代理（原阶段8）。

### 15.1 阶段目标

在 call 稳定后，支持真正的控制权转移。

### 15.2 必备字段

- `focused_agent_id`
- `running_agent_id`
- `parent_session_id`
- `handoff_stack`

### 15.3 关键要求

- 用户输入根据 `focused_agent_id` 路由
- 专家完成后可自动归还主控
- 异常时允许强制回主控

### 15.4 完成定义

- 焦点代理切换后用户输入路由正确
- 恢复后焦点代理不丢失

### 15.5 防偏离补充（执行约束）

#### 15.5.1 主链接入判据（必须）

1. 输入路由由后端运行时按 `focused_agent_id` 决策，前端仅展示，不参与裁决。
2. 自动归还与强制回主控都经过同一状态机出口，避免双路径行为分叉。
3. 恢复后 `focused_agent_id`、`running_agent_id`、`handoff_stack` 三者一致。

#### 15.5.2 禁止伪完成

- 只做 UI 焦点显示切换，不算 handoff 完成。
- 仅正常归还可用，异常强制回主控不可用，不算完成。
- 仅内存态可用、重启丢失焦点，不算完成。

#### 15.5.3 最小验收证据模板

至少提供：

1. handoff enter/route/return/force_return 各一条日志样本。
2. 一组“handoff 活跃时输入路由到焦点代理”的断言输出。
3. 一组重启恢复后焦点与路由一致的证据。

---

## 15A. 第八阶段详细执行清单

第八阶段是高风险阶段，必须在前面所有阶段稳定后再做。

### 15A.1 允许修改的文件

- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/workflow/react/manager.rs](src-tauri/src/workflow/react/manager.rs)
- [src/composables/workflow/useWorkflowCore.ts](src/composables/workflow/useWorkflowCore.ts)

### 15A.2 推荐实现顺序

1. 增加 `focused_agent_id`
2. 增加 handoff 事件
3. 实现输入路由
4. 实现异常回主控
5. 增加恢复测试

### 15A.3 本阶段禁止事项

- 不同时改 reviewer gate
- 不同时改任务账本 UI

### 15A.4 自动化测试最低要求

1. handoff 后用户输入路由正确
2. 自动归还主控正确
3. 强制回主控可用

### 15A.5 完成定义

- 控制权转移稳定且可恢复

### 15A.6 允许修改文件最小闭环（补充）

为防止“只改前端路由判断”，阶段8建议最小闭环文件集合为：

- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- [src-tauri/src/workflow/react/manager.rs](src-tauri/src/workflow/react/manager.rs)
- [src/composables/workflow/useWorkflowCore.ts](src/composables/workflow/useWorkflowCore.ts)
- 存储 `focused_agent_id/handoff_stack` 的持久化读写文件（按实际实现补齐）
- 命令层输入路由入口文件（按实际实现补齐）

---

## 16. 阶段 9：任务账本与高级 UI

> 执行顺序说明（2026-04-07 更新）：阶段9前置到阶段8之前执行，用于降低后续Handoff调试成本并改善联调体感。

### 16.1 阶段目标

在内核全部稳定后，再引入任务账本、任务磁贴、Inspector 等 UI 增强。

### 16.2 原则

- 所有 UI 展示数据必须来自结构化状态或事件
- 不允许从 transcript 文本推断任务层信息
- 工具消息 / 观察消息 / 流式输出的前端展示应优先基于增量视图模型，而不是每次从原始消息数组全量现推

### 16.3 完成定义

- UI 增强不影响内核正确性
- 即使 UI 展示异常，workflow 仍可继续运行和恢复

### 16.4 防偏离补充（执行约束）

#### 16.4.1 主链接入判据（必须）

1. 任务账本展示字段全部来源于结构化状态/事件（可追踪字段来源）。
2. UI 异常路径必须被错误边界或降级逻辑捕获，不向执行内核回写错误状态。
3. 多会话切换时，任务面板数据与 session 强绑定，不串台。
4. 工具调用的 pending / approved-running / rejected / final 状态在前端使用统一 `tool_call_id` 视图模型收敛，不允许 assistant/tool 双轨长期并存。

#### 16.4.2 禁止伪完成

- 任何 transcript 文本推断任务态的逻辑进入主路径，视为未完成。
- 仅视觉展示可用、数据来源不可追溯，不算完成。
- UI 异常导致 workflow 控制命令异常/中断，不算完成。

#### 16.4.3 最小验收证据模板

至少提供：

1. TaskItem/Inspector 字段映射测试输出。
2. 一条“注入 transcript 噪声但任务态不变”的测试证据。
3. 一条“UI 渲染异常但后端执行持续推进”的证据。

---

## 16B. 第九阶段详细执行清单

第九阶段只做展示层增强，不应再反向污染执行内核。

### 16B.1 允许修改的文件

- `src/components/workflow/*`
- `src/composables/workflow/*`
- 与 task ledger 相关的后端结构定义

### 16B.2 推荐实现顺序

1. 定义 `TaskItem`
2. 后端暴露结构化任务状态
3. 前端建立 `tool_call_id -> ToolViewState` 的增量视图模型
4. 前端接入任务列表
5. 前端接入焦点显示
6. 前端接入高级面板

### 16B.3 本阶段禁止事项

- 不从 transcript 文本推断任务状态
- 不让 UI 逻辑决定执行层状态
- 不继续扩大“原始 messages 数组全量扫描 + 多轮 filter/map”作为工具态收敛主路径

### 16B.4 自动化测试最低要求

1. UI 状态展示与后端结构化数据一致
2. UI 异常不影响 workflow 恢复
3. tool stream 高频更新时，不会重复产生 pending / waiting approval / approved result 三条冲突展示

### 16B.5 完成定义

- UI 增强建立在稳定内核之上

### 16B.6 允许修改文件最小闭环（补充）

为防止“UI 改造反向污染内核”，阶段9建议最小闭环文件集合为：

- `src/components/workflow/*`
- `src/composables/workflow/*`
- `src/stores/workflow.js`
- 与 task ledger 数据结构暴露相关的后端定义文件
- 明确禁止改动核心执行状态机语义文件（除非单独立项）

---

## 16A. 可选审核机制设计（Plan Review / Final Review）

本节是一个后置扩展设计，只允许在阶段 7 完成后实现。目的不是简单地“拦截流程”，而是引入一个受约束的批判性改进环，提高计划质量和结果质量。

### 16A.1 目标

审核机制必须满足：

- 由前端开关决定是否启用
- 可分别控制“计划审核”和“完成审核”
- reviewer 输出结构化问题、严重度和修订建议
- 审核失败后可以驱动修订，不是只有通过/拒绝
- 审核轮次可控，避免无限对抗循环
- 刷新页面和应用重启后，审核状态可恢复

### 16A.2 核心定位

审核机制不是 UI 弹窗，不是 transcript 后处理，也不是简单布尔裁决。

审核机制应建模为：

- 一个可选 Gate
- 一个 reviewer 子任务
- 一个结构化 `ReviewResult`
- 一个受控的修订闭环

### 16A.3 接入时机

计划审核接入点：

- planner 生成计划之后
- executor 或主控代理进入执行之前

完成审核接入点：

- executor 生成最终结果之后
- workflow 真正标记 `Completed` 之前

第一版不要在执行过程中每一步都审核。

### 16A.4 review policy 配置

建议在 workflow 的配置中新增：

```ts
review_policy = {
  plan_review_enabled: boolean,
  final_review_enabled: boolean,
  reviewer_agent_id: string | null,
  max_review_rounds: number,
  strict_mode: boolean
}
```

字段解释：

- `plan_review_enabled`
  - 是否启用计划审核
- `final_review_enabled`
  - 是否启用完成审核
- `reviewer_agent_id`
  - 审核代理 ID
- `max_review_rounds`
  - 最大审核轮次，建议默认 `2`
- `strict_mode`
  - 严格模式。若存在阻断问题则不允许继续

### 16A.5 前端职责

前端只负责配置和展示，不负责裁决逻辑。

建议前端提供开关：

- `Enable Plan Review`
- `Enable Final Review`

可选配置：

- `Strict Review Mode`
- `Max Review Rounds`
- `Reviewer Agent`

前端必须能展示：

- 当前是否处于 review gate
- 当前审核轮次
- reviewer 的结构化结论
- 主要问题
- 修订建议

### 16A.6 审核结果模型

审核结果不能只有通过或拒绝。

建议如下：

```rust
pub struct ReviewResult {
    pub verdict: ReviewVerdict,
    pub summary: String,
    pub strengths: Vec<String>,
    pub issues: Vec<ReviewIssue>,
    pub revision_instructions: Vec<String>,
    pub confidence: ReviewConfidence,
}

pub enum ReviewVerdict {
    Approved,
    NeedsRevision,
    Blocked,
}

pub enum ReviewConfidence {
    Low,
    Medium,
    High,
}

pub struct ReviewIssue {
    pub severity: ReviewSeverity,
    pub category: ReviewCategory,
    pub title: String,
    pub detail: String,
    pub suggested_fix: String,
    pub blocking: bool,
}

pub enum ReviewSeverity {
    Critical,
    Major,
    Minor,
}

pub enum ReviewCategory {
    Logic,
    Coverage,
    Risk,
    Architecture,
    Testing,
    Quality,
    Security,
}
```

### 16A.7 verdict 语义

- `Approved`
  - 允许继续
  - 允许保留非阻断建议
- `NeedsRevision`
  - 不允许直接继续
  - 必须按 reviewer 建议修订后再次提交
- `Blocked`
  - 存在关键问题
  - 不允许自动继续
  - 通常需要主控代理或用户介入

### 16A.8 Review Gate 运行态

建议后续在 `ExecutionContext` 中增加：

```rust
pub struct ReviewGateState {
    pub gate_type: ReviewGateType,
    pub reviewer_agent_id: String,
    pub current_round: u32,
    pub max_rounds: u32,
    pub strict_mode: bool,
    pub last_result: Option<ReviewResult>,
}

pub enum ReviewGateType {
    PlanReview,
    FinalReview,
}
```

若当前版本不希望一次性扩充太多，也至少要持久化：

- 当前 review gate 类型
- 当前轮次
- reviewer 子任务 ID
- 最后一次 review 结果

### 16A.9 Snapshot 要求

审核机制启用后，以下内容必须进入 snapshot：

- `review_policy`
- `active_review_gate`
- `current_review_round`
- `last_review_result`
- `waiting_on_task_id`

禁止只把 reviewer 输出写进 `workflow_messages`。

### 16A.10 Event Store 事件建议

在 event store 稳定后，应增加：

- `ReviewRequested`
- `ReviewCompleted`
- `ReviewApproved`
- `ReviewNeedsRevision`
- `ReviewBlocked`
- `ReviewRoundExceeded`

事件至少包含：

- `session_id`
- `gate_type`
- `reviewer_agent_id`
- `current_round`
- `verdict`
- `blocking_issue_count`

### 16A.11 第一版接入方式

第一版不要让 reviewer 接管主会话。

推荐做法：

1. 主流程创建 reviewer 子任务
2. 主流程进入 `Waiting + child_task`
3. reviewer 产出结构化 `ReviewResult`
4. 父流程恢复并分支处理

这样前端只负责显示，不参与审核状态机。

### 16A.12 计划审核流程

1. planner 生成计划
2. 如果 `plan_review_enabled = false`
   - 直接进入执行
3. 如果 `plan_review_enabled = true`
   - 启动 reviewer 子任务
   - 主流程进入 `Waiting`
   - `review_gate = plan_review`
4. reviewer 返回 `ReviewResult`
5. 主流程恢复：
   - `Approved` -> 进入执行
   - `NeedsRevision` -> 把 `issues + revision_instructions` 回给 planner 修订
   - `Blocked` -> 进入阻断态或等待用户裁决

### 16A.13 完成审核流程

1. executor 生成最终结果
2. 如果 `final_review_enabled = false`
   - 直接完成
3. 如果 `final_review_enabled = true`
   - 启动 reviewer 子任务
   - 主流程进入 `Waiting`
   - `review_gate = final_review`
4. reviewer 返回 `ReviewResult`
5. 主流程恢复：
   - `Approved` -> 真正标记 `Completed`
   - `NeedsRevision` -> 把 reviewer 问题交回 executor 或主控代理继续修复
   - `Blocked` -> 不允许自动完成

### 16A.14 最大轮次控制

审核机制必须防止无限循环。

建议：

- 默认最大轮次 `2`
- 超过轮次后：
  - 若 `strict_mode = true`
    - 进入等待用户决策
  - 若 `strict_mode = false`
    - 可允许用户带风险继续

必须新增：

- `ReviewRoundExceeded` 事件
- `workflow.review.round_exceeded` 日志

### 16A.15 Reviewer 职责约束

Reviewer 必须是“批判性质量代理”，不是单纯的否决代理。

必须要求 reviewer：

- 给出明确问题
- 给出严重度
- 给出可执行修订建议
- 区分阻断问题和非阻断问题
- 不允许只给空泛结论
- 不允许为了对抗而制造无意义问题

推荐 reviewer 职责：

- 优先检查逻辑漏洞、覆盖缺口、风险遗漏、测试不足
- 其次指出高价值改进点
- 对轻微风格问题不过度阻断

### 16A.16 第一版实现边界

第一版只做：

- plan review
- final review
- 结构化 `ReviewResult`
- 最大轮次控制
- snapshot / event / log 可恢复

第一版不做：

- 中途每一步审核
- 多 reviewer 投票
- reviewer handoff
- reviewer 接管对话
- 多 reviewer 交叉辩论

### 16A.17 可观测性要求

必须新增日志：

- `workflow.review.requested`
- `workflow.review.completed`
- `workflow.review.approved`
- `workflow.review.needs_revision`
- `workflow.review.blocked`
- `workflow.review.round_exceeded`

每条日志至少包含：

- `session_id`
- `gate_type`
- `reviewer_agent_id`
- `round`
- `verdict`
- `blocking_issue_count`

### 16A.18 自动化测试建议

至少补以下测试：

1. review 开关关闭时，主流程不会进入 review gate。
2. plan review 开启时，planner 完成后会创建 reviewer 子任务。
3. reviewer 返回 `Approved` 时，主流程进入下一阶段。
4. reviewer 返回 `NeedsRevision` 时，主流程进入修订分支。
5. reviewer 返回 `Blocked` 时，主流程进入阻断分支。
6. review 超过最大轮次时，不会自动无限重试。
7. 应用重启后，处于 review gate 的 workflow 可以恢复。

### 16A.19 手工验收步骤

1. 打开计划审核开关
2. 启动需要 planner 的 workflow
3. 验证 planner 完成后进入 review gate
4. 让 reviewer 返回修改建议
5. 验证 planner 能根据建议修订
6. 打开完成审核开关
7. 让 executor 产出一个故意有缺陷的结果
8. 验证 reviewer 可返回 `Blocked`
9. 验证主流程不会错误完成

### 16A.20 完成定义

满足以下条件才算审核机制第一版完成：

- 可通过前端开关控制 plan review / final review
- reviewer 输出结构化问题与建议
- `NeedsRevision` 能驱动修订闭环
- `Blocked` 能阻止错误完成
- 超过最大轮次时能安全停下
- 刷新页面或重启应用后，review gate 状态可恢复

### 16A.21 实现顺序要求

审核机制必须按以下顺序接入：

1. 阶段 0-3 完成
2. 阶段 4-5 至少具备基础 snapshot / event 能力
3. 阶段 7 完成单层 `Call`
4. 再实现审核 Gate

禁止跳过阶段 7 直接做审核机制。

---

## 16C. 审核机制详细执行清单

审核机制是阶段 7 之后的扩展实施清单。

### 16C.1 允许修改的文件

- review policy 所在的 workflow config 读写代码
- [src-tauri/src/workflow/react/orchestrator.rs](src-tauri/src/workflow/react/orchestrator.rs)
- [src-tauri/src/workflow/react/engine.rs](src-tauri/src/workflow/react/engine.rs)
- 前端 workflow 设置与展示组件

### 16C.2 推荐实现顺序

1. 增加 `review_policy`
2. 增加 `ReviewResult` 结构
3. 接入 `plan_review` gate
4. 接入 `final_review` gate
5. 增加最大轮次控制
6. 增加 snapshot / event / log 支持
7. 前端增加开关与 review 结果展示

### 16C.3 本阶段禁止事项

- 不做 reviewer handoff
- 不做多 reviewer
- 不做执行过程中的逐步 review

### 16C.4 自动化测试最低要求

1. 关闭开关时不进入 review gate
2. 开启开关时能进入 reviewer 子任务
3. `Approved` 能继续
4. `NeedsRevision` 能修订
5. `Blocked` 能阻断
6. 超轮次能停下

### 16C.5 完成定义

- review gate 可选开启
- reviewer 输出结构化问题与修订建议
- 具备受控修订闭环

---

## 17. 推荐开发顺序

严格按以下顺序推进，不要跳阶段：

1. 阶段 0
2. 阶段 1
3. 阶段 2
4. 阶段 3
5. 阶段 4
6. 阶段 5
7. 阶段 6
8. 阶段 7
9. 阶段 7 扩展：可选审核 Gate
10. 阶段 9（前置执行）
11. 阶段 8（后置执行）

如果资源有限，第一阶段只做：

1. 阶段 0
2. 阶段 1
3. 阶段 2
4. 阶段 3

---

## 18. 每阶段统一交付模板

接手的 AI 或开发者在完成任意阶段时，必须输出以下内容：

### 18.1 代码变更说明

- 改了哪些文件
- 每个文件承担什么职责

### 18.2 数据结构变化

- 新增了哪些 struct / enum / table
- 删除了哪些旧入口

### 18.3 兼容与迁移说明

- 是否保留 fallback
- fallback 何时触发

### 18.4 可观测性说明

- 新增了哪些日志
- 新增了哪些指标

### 18.5 测试说明

- 自动化测试覆盖了什么
- 手工测试如何执行

### 18.6 风险说明

- 当前阶段已知风险
- 下一阶段前置条件

---

## 19. 第一里程碑详细任务单

以下任务单可以直接派给 AI 执行。

### 19.1 任务 A：补日志与测试基线

目标：

- 完成阶段 0

要做的事：

1. 在 `commands/workflow.rs`、`engine.rs`、`gateway.rs` 中补统一日志。
2. 梳理现有状态使用点，输出文档。
3. 补充至少一组 waiting / approval 测试。

完成标准：

- 可复现问题并通过日志定位。

### 19.2 任务 B：实现 WorkflowManager

目标：

- 完成阶段 1

要做的事：

1. 新增 `manager.rs`
2. 在 `lib.rs` 注入全局 manager state
3. 修改 `workflow_start`
4. 修改 `workflow_signal`
5. 将 session 查询与路由迁移到 manager

完成标准：

- session 生命周期集中管理

### 19.3 任务 C：实现 Snapshot 表和 ExecutionContext

目标：

- 完成阶段 2

要做的事：

1. 增加 migration
2. 增加 DB API
3. 增加 `ExecutionContext`
4. 在 waiting / terminal 关键点写 snapshot
5. approval 恢复改为依赖 snapshot

完成标准：

- approval 恢复不再依赖消息反推

### 19.4 任务 D：统一 Waiting 模型与 Signal 模型

目标：

- 完成阶段 3

要做的事：

1. 新增 `WorkflowSignal`
2. waiting 统一主逻辑
3. UI 等待态改造
4. 保留旧输入兼容解析层

完成标准：

- waiting 逻辑统一
- 前端可基于 `state + wait_reason` 工作

### 19.5 任务 E：实现单层子任务 Call

目标：

- 完成阶段 7

要做的事：

1. 引入父任务等待子任务模型
2. 持久化 `waiting_on_task_id`
3. 子任务完成后恢复父任务
4. 增加相关日志和测试

完成标准：

- 父任务可稳定等待并恢复

### 19.6 任务 F：实现可选审核 Gate 第一版

目标：

- 完成 `16A. 可选审核机制设计`

要做的事：

1. 增加 `review_policy`
2. 接入 reviewer 子任务
3. 实现 `plan_review` gate
4. 实现 `final_review` gate
5. 引入结构化 `ReviewResult`
6. 实现最大轮次控制
7. 增加 snapshot / event / 日志支持
8. 增加前端开关和展示

完成标准：

- 开关可控
- reviewer 可输出结构化问题与修订建议
- 主流程可根据审核结果修订或阻断

---

## 20. 禁止事项

在第一里程碑完成前，禁止以下行为：

- 一边做 snapshot 一边提前做 handoff
- 一边做 waiting 重构一边提前做任务账本
- 用更多消息 metadata 继续增强“历史消息反推”
- 在没有 reducer 设计前直接引入复杂 event replay
- 在没有 manager 的情况下继续扩展 `workflow_signal` 命令层分支
- 在没有阶段 7 单层 `Call` 的前提下提前做 reviewer 审核机制

---

## 21. 结束语

本次重构的核心不是“做一个更炫的工作流系统”，而是先把工作流系统做成一个可靠的后端执行内核。

只有当以下四件事成立之后，后续多代理和复杂 UI 才值得继续投入：

1. session 生命周期稳定
2. waiting 状态可恢复
3. approval 恢复可靠
4. 结构化状态不再依赖 transcript 反推

如果接手者只能做一件事，优先顺序永远是：

1. `WorkflowManager`
2. `ExecutionContext Snapshot`
3. `Waiting + Signal Unified Model`
4. `Event Store`
