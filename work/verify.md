# 工作流引擎架构重构规划 (V3 - 全量技术白皮书)

## 一、 问题根源深度分析 (Root Cause Analysis)

### 1.1 核心矛盾：状态分裂 (State Splitting)
目前的系统存在 **持久化状态 (SQLite)** 与 **运行时状态 (内存)** 的严重脱节。
*   **持久化状态**：仅存储了 `workflows.status` 和 `workflow_messages`。
*   **运行时状态**：包括 `signal_rx` 通道、`pending_approvals` (DashMap)、`auto_approve` 策略、`loop_detector` 等仅在内存中。
*   **恢复灾难**：刷新页面或程序重启后，内存状态丢失。系统被迫通过解析历史消息的 JSON 文本来“猜测”并重建 `pending_approvals`。一旦 LLM 输出稍有偏差，就会触发 "Signal channel closed" 或恢复失败。

### 1.2 具体问题清单
| 问题类型 | 具体表现 | 根本原因 | 影响 |
|:---|:---|:---|:---|
| **状态过多且重叠** | 11 个状态（Paused, AwaitingUser 等）逻辑重复 | 缺乏统一的等待机制抽象 | 代码冗余，难以维护 |
| **信号通道脆弱** | 刷新页面导致 signal_rx 关闭 | 通道生命周期绑定在 UI 窗口而非后端 Session | 审批信号无法提交 |
| **恢复路径分散** | 重建逻辑散落在 init_internal 等多处 | 状态非自包含 | 难以追踪完整的恢复流 |
| **时序依赖严重** | 前端必须先就绪，否则 Backend 发出的事件丢失 | 缺乏订阅握手机制 | 页面刷新后弹窗不显示 |

---

## 二、 核心架构：事件总线驱动 (Event-Bus Driven)

不再将数据库作为“同步中转站”，而是将其视为“事件归档者”和“状态快照点”。

### 2.1 架构拓扑
```
┌──────────────┐      (1) 产生事件      ┌─────────────────────┐
│   Executor   │ ───────────────────→ │   Event Broadcaster │
│ (状态机引擎)  │                      │   (内存事件总线)      │
└──────────────┘                      └──────────┬──────────┘
                                                 │
      ┌──────────────────┬───────────────────────┼──────────────────────┬──────────────────┐
      │                  │                       │                      │                  │
      ▼                  ▼                       ▼                      ▼                  ▼
┌────────────┐    ┌────────────┐          ┌────────────┐         ┌────────────────────────┐
│  DB Sink   │    │ Tauri Sink │          │  IM Sink   │         │      Other Sink        │
│ (持久化存储)│    │ (窗口通知)  │          │ (外部机器人)│         │      (后续扩展)         │
└────────────┘    └────────────┘          └────────────┘         └────────────────────────┘
```
*   **Broadcaster (总线)**：Rust 端的内存广播通道（如 `tokio::sync::broadcast`）。
*   **Sinks (订阅者)**：所有网关（Gateway）并行订阅。DB Sink 仅负责存盘，不参与业务分发。彻底移除低效的数据库轮询机制。

### 2.2 事件分发可靠性与背压处理 (Backpressure Handling)
*   **非阻塞分发原则**：`Event Broadcaster` 必须是非阻塞的。Executor 产生的事件应立即推入总线，不能因为 Sink（如 DB 写入）的延迟而卡住执行引擎。
*   **消费者缓冲机制 (Sink Buffering)**：
    *   **DB Sink (Critical)**：由于其决定了 L3 恢复的正确性，DB Sink 内部必须维护一个足够大的异步缓冲队列。若发生 `Lagged` 错误，系统应记录严重告警并尝试从内存快照修复。
    *   **UI/IM Sink (Best-effort)**：对于实时性要求高但允许少量丢失的 Sink，在缓冲区溢出时允许丢弃中间状态事件（如 Thinking Token），仅保证最终状态事件（如 TaskCompleted）的必达。
*   **通道容量配置**：根据典型工作流负载（每秒 50-100 事件）配置合理的总线 `capacity`，并结合 `tokio::sync::Notify` 机制在消息量激增时通知 Executor 适当降频。

---

## 三、 状态模型与上下文 (State & Context)

### 3.1 简化后的状态机 (5 个核心状态)
| 原状态 | 新状态 + WaitReason | 合并原因 |
|:---|:---|:---|
| Paused | Waiting + Confirmation | 都是等待用户决定是否继续 |
| AwaitingUser | Waiting + UserInput | 都是等待用户回答问题 |
| AwaitingApproval | Waiting + Approval | 都是等待工具审批 |
| Thinking/Executing | Running | 执行循环的组成部分 |

### 3.2 ExecutionContext 字段定义 (含任务账本支持)
```rust
struct ExecutionContext {
    session_id: String,
    current_state: WorkflowState,
    wait_reason: Option<WaitReason>,
    
    // 控制权与输入路由管理 (借鉴 OpenAI Swarm)
    active_agent_id: String,           // 当前焦点代理。系统根据此 ID 路由用户输入流。
    
    // 任务账本 (Task Ledger - 替代/扩展现有的 todo 字段)
    // 记录任务层级、状态及语义摘要，用于 UI 渲染与跨任务记忆恢复。
    tasks: Vec<TaskItem>,             
    
    // 执行进度与中间态
    current_step: usize,
    max_steps: usize,
    pending_tools: HashMap<String, PendingTool>, // 替代旧的 pending_approvals
    last_action_summary: String,                // 供 UI 展示的“一句话动态”
    
    // 多级代理关联 (限定三级深度)
    parent_session_id: Option<String>,
    child_sessions: Vec<String>,
    
    // 恢复与校验
    version: String,           // 代码版本，用于兼容性检查
    last_event_id: i64,        // 最后一个已处理事件 ID，用于 L3 对齐
}

struct TaskItem {
    id: String,                        // 任务 ID
    title: String,                     // 任务标题 (UI 展示)
    status: TaskStatus,                // Pending, Running, Completed, Failed
    summary: Option<String>,           // 任务完成后的语义摘要 (由大脑生成，用于恢复)
    agent_id: String,                  // 执行该任务的代理 ID
    task_id: Option<String>,           // 关联的消息/事件 TaskID (用于分片过滤)
}
```

---

## 四、 代理协作与输入路由策略

支持 **“大脑 -> 专家 Agent -> Sub-Agent”** 的递归嵌套结构，深度限三级。

### 4.1 任务移交 (Handoff - 异步接力模式)
*   **非阻塞切换机制**：
    *   当大脑调用 `transfer_to_agent` 时，系统执行 **“快照保存 -> 变更 active_agent_id -> 大脑协程休眠”** 的原子操作。
    *   **不占用资源**：大脑不会在工具内部同步等待，而是直接释放计算资源，其状态仅保留在内存快照中。
*   **输入穿透路由 (Input Piercing)**：
    *   由于 `active_agent_id` 已拨至专家，后续 `UserInputProvided` 事件将绕过大脑，直接由 `WorkflowManager` 路由至活跃专家。
*   **控制权归还与激活**：
    *   专家任务完成（或主动归还）时，系统重新将 `active_agent_id` 拨回大脑，并根据 L1/L2 恢复机制 **唤醒（Resume）** 大脑继续执行。

### 4.2 任务调用 (Call - 树状指挥官模式)
*   **原理**：父代理将子代理视为“异步长期运行工具”。
*   **状态表现**：父代理保持 `active_agent_id`，但进入 `Waiting` 状态（同样不占用活跃 CPU），监听子代理产生的事件流。子代理完成后，结果作为工具输出返回给父级。

### 4.3 协作模式：移交 (Handoff) vs 调用 (Call)
*   **Handoff (扁平化接力赛)**：大脑将阶段性任务控制权转移给专业代理。修改 `active_agent_id`，后续追问直接响应。
    *   **控制权归还机制**：
        1.  **主动归还**：专家在处理追问时，若发现意图超出专业范畴（如在编程中突然问天气），需调用 `transfer_back_to_brain` 工具主动交还。
        2.  **自动归还**：当专家完成预定任务并发送 `TaskCompleted` 事件后，`WorkflowManager` 监听到该事件，将 `active_agent_id` 自动重置为 "Brain"，确保后续对话焦点回到总控。
*   **Call (树状指挥官)**：父代理将子代理视为“异步长期运行工具”。父代理保持 `active_agent_id` 并挂起监听。子代理完成后结果返回父级。

### 3.3 标识符规范 (Naming Convention)
*   **全局唯一性**：统一采用 13 位 TSID 生成器。
*   **前缀标识**：SessionID 使用 `ses-` 前缀，TaskID 使用 `tsk-` 前缀，确保日志与数据库的可读性与唯一性。

### 4.2 任务账本分级缓存 (Ledger Tiered Storage)
*   **内存轻量化**：`ExecutionContext` 中的 `tasks` 列表仅保留最近 10 个已完成任务的 `summary`（语义摘要）。
*   **按需加载**：超过 10 个的任务摘要将从内存中移除（仅保留元数据）。若大脑需要唤醒旧任务，则通过 `task_id` 从数据库事件流中异步重新生成摘要，平衡内存占用与恢复能力。

### 4.3 协作模式：移交 (Handoff) vs 调用 (Call)
*   **Handoff (扁平化接力赛)**：大脑将阶段性任务控制权转移给专业代理。修改 `active_agent_id`，后续追问直接响应。
    *   **控制权归还机制**：
        1.  **主动归还**：专家在处理追问时，若发现意图超出专业范畴（如在编程中突然问天气），需调用 `transfer_back_to_brain` 工具主动交还。
        2.  **自动归还**：当专家完成预定任务并发送 `TaskCompleted` 事件后，`WorkflowManager` 监听到该事件，将 `active_agent_id` 自动重置为 "Brain"，确保后续对话焦点回到总控。
*   **Call (树状指挥官)**：父代理将子代理视为“异步长期运行工具”。父代理保持 `active_agent_id` 并挂起监听。子代理完成后结果返回父级。

### 4.4 异常管理与故障升级 (Fault Escalation)
*   **专家崩溃恢复**：若执行专家（如 Coder）意外中断，其状态仍保留在 `active_agent_id` 中。用户点击“继续”将触发 L2/L3 恢复逻辑，尝试原地复活专家。
*   **自动升级机制**：
    *   **触发条件**：若子代理在同一任务中连续崩溃或陷入死循环（Loop Detection 触发）。
    *   **动作**：`WorkflowManager` 强制将 `active_agent_id` 拨回 "Brain"，大脑被唤醒并接收 `AgentCrashed` 事件。
    *   **职责**：大脑接管对话，负责向用户报告错误，并决定是“重试”还是“更换专家”。
*   **交互锁定与强制重置**：
    *   在任务未发送 `TaskCompleted` 事件前，系统默认锁定焦点代理。
    *   **强行中断**：UI 提供“返回主控”按钮，允许用户手动中断卡死的专家，强制归还控制权给大脑。

### 4.5 上下文解耦与注入 (Context Decoupling)
*   **物理隔离**：大脑与专家不共享全量上下文。
    *   **大脑视图**：用户原始需求 + 各专家的阶段性任务摘要 (Task Summary)。
    *   **专家视图**：大脑下达的子任务描述 + 专家内部的思考过程与工具调用历史。
*   **动态注入**：Handoff 移交控制权时，大脑仅将“任务相关背景”注入专家的 Prompt。这种模式极大地节省了 Token 开销，并避免了无关对话对专业代理的干扰。

---

## 五、 三级恢复策略 (Recovery Hierarchy)

### 5.1 恢复流水线
1.  **L1: 内存恢复 (Hot Recovery)**：
    *   **机制**：后端全局 `WorkflowManager` 查找 SessionID。只要实例在，前端刷新后直接重连到活跃通道。
    *   **优势**：状态绝对实时，延迟为零。
2.  **L2: 快照恢复 (Cold Recovery)**：
    *   **场景**：程序重启。
    *   **校验**：检查版本号(Version)兼容性、Schema 完整性、`last_event_id` 逻辑对齐。
3.  **L3: 事件溯源回放 (Event Replay Recovery)**：
    *   **原理**：`State = Σ Events`。
    *   **机制**：从 DB 读取结构化事件流（JSON），在内存中重演，精确还原 `ExecutionContext`（包含当前的控制权状态）。

---

## 六、 持久化与存储细节

### 6.1 存储层瘦身与精简策略
*   **MetaData 字段精简**：移除冗余的请求配置信息（如 `proxyServer`, `customHeaders`, `chatParam` 等）。持久化仅保留业务核心字段：`tool_calls`, `usage`, `active_agent_id`, `task_id`。
*   **引入 Task ID (任务分片)**：
    *   在 `workflow_events` 和 `workflow_messages` 表中增加 `task_id` 字段。
    *   **作用**：用于标识消息/事件所属的具体子任务。在“跨任务记忆恢复”时，系统根据 `task_id` 快速索引并精准剪裁上下文，避免无关任务的 Token 干扰。

### 6.2 数据库 Schema 建议 (优化版)
```sql
-- 事件存储 (Event Store)
CREATE TABLE workflow_events (
    id INTEGER PRIMARY KEY,
    session_id TEXT NOT NULL,
    task_id TEXT,             -- 可选：关联具体的子任务 ID
    active_agent_id TEXT,     -- 产生事件的代理标识
    event_type TEXT NOT NULL, -- ToolRequested, Thinking, UserInput, etc.
    event_data TEXT NOT NULL, -- 精简后的业务数据 JSON
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- 快照存储 (Snapshot Store)
CREATE TABLE workflow_snapshots (
    session_id TEXT PRIMARY KEY,
    context_json TEXT NOT NULL, -- 包含 finished_tasks 索引的快照
    updated_at DATETIME
);
```

---

## 七、 UI/UX 演进：多级感知与焦点管理

### 7.1 悬浮状态面板 (Inspector)
*   **标签化 (Tabs)**：面板顶部增加横向滚动标签。切换 Tab 自动刷新对应代理的数据。
*   **焦点同步**：前端订阅 `AgentSwitched` 事件。当控制权转移时，自动切换到对应代理的 Tab。
*   **红点角标**：后台代理处于 `Waiting` 状态时，标题显示闪烁红点。

### 7.2 主窗口任务磁贴 (Task Tiles)
*   **呼吸灯卡片**：顶部展示活跃子任务磁贴。展示 `代理名` + `状态颜色` + `一句话动态`（如：正在读取 main.rs...）。
*   **交互联动**：点击磁贴，状态面板自动切到该 Tab。支持 `resume_task` 信号一键跳转回旧任务。

---

## 八、 网关模式 (Gateway Pattern)

### 8.1 统一 Trait 定义
```rust
#[async_trait]
pub trait WorkflowGateway: Send + Sync {
    /// 提交控制信号
    async fn submit_signal(&self, session_id: &str, signal: WorkflowSignal) -> Result<()>;
    /// 获取当前快照 (用于前端初始化)
    async fn get_snapshot(&self, session_id: &str) -> Result<ExecutionContext>;
    /// 订阅实时事件流 (Push 模式)
    async fn subscribe_events(&self, session_id: &str) -> Result<Box<dyn EventStream>>;
}
```

---

## 九、 实施路径与估算

1.  **第一阶段 (4天)**：建立全局 `WorkflowManager` 注册表，实现 L1/L2 恢复及基于 `active_agent_id` 的输入路由基础。
2.  **第二阶段 (4天)**：实现内存广播总线与多 Sink 分发，支持多 Session 状态面板标签切换与焦点同步。
3.  **第三阶段 (5天)**：实现 L3 事件回放、子代理递归调用/移交逻辑及跨任务记忆索引。

---

## 十、 总结
本方案融合了 LangGraph 的可靠性、OpenAI Swarm 的灵活性以及高性能的事件驱动架构。通过内存快照保证极速响应，通过事件溯源保证数据永不丢失，通过穿透路由和项目记忆解决了多级代理下的交互直感问题。
