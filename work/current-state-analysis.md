# 工作流模块现状分析报告

> 本文档是阶段0的基线分析，记录重构前的代码库现状。
> 生成时间：2025-04-03
> 目的：为后续阶段提供参考锚点

---

## 1. 模块整体结构

```
src-tauri/src/workflow/
├── mod.rs                    # 模块入口 (dag 已注释，仅导出 react)
├── error.rs                  # 错误类型定义
│
├── react/                    # ReAct 自主执行引擎 (26 个文件)
│   ├── mod.rs                # 导出 21 个子模块
│   ├── ARCHITECTURE.md       # 架构文档 (499 行)
│   │
│   ├── engine.rs             # 【核心】ReAct 引擎主循环 (117KB, 2693 行)
│   ├── gateway.rs            # 【通信】Tauri 事件网关 (7KB, 180 行)
│   ├── orchestrator.rs       # 【编排】子代理工厂和后台任务管理 (19KB, 493 行)
│   ├── types.rs              # 类型定义 (WorkflowState, GatewayPayload 等)
│   │
│   ├── context.rs            # 上下文管理
│   ├── memory.rs             # 内存管理
│   ├── llm.rs                # LLM 处理
│   ├── tools/interceptors.rs # 拦截器
│   ├── security.rs           # 安全守卫 (PathGuard)
│   ├── skills.rs             # 技能扫描和管理
│   ├── intelligence.rs       # 智能评估器
│   ├── observation.rs        # 观察强化
│   ├── planners.rs           # 规划器
│   ├── runners.rs            # 执行器
│   ├── prompts.rs            # 提示词模板
│   ├── loop_detector.rs      # 循环检测
│   ├── memory_analyzer.rs    # 内存分析
│   ├── compression.rs        # 上下文压缩
│   ├── policy.rs             # 执行策略
│   ├── agents_md.rs          # 代理元数据
│   └── error.rs              # ReAct 错误类型
│
└── dag/                      # DAG 工作流引擎 (当前未启用)
    ├── mod.rs
    ├── engine/
    ├── executor/
    ├── config.rs
    ├── context.rs
    ├── graph.rs
    ├── parser.rs
    └── types.rs
```

---

## 2. 核心文件职责

### 2.1 engine.rs (核心引擎)

**位置**: `src-tauri/src/workflow/react/engine.rs`
**规模**: 2693 行, 117KB

**关键结构**:
```rust
pub struct WorkflowExecutor {
    pub session_id: String,
    pub context: ContextManager,
    pub tool_manager: Arc<ToolManager>,
    pub chat_state: Arc<ChatState>,
    pub gateway: Arc<dyn Gateway>,
    pub state: WorkflowState,
    pub current_step: usize,
    pub max_steps: usize,
    pub signal_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    pub pending_approvals: Arc<dashmap::DashMap<String, serde_json::Value>>,
    // ... 更多字段
}
```

**关键 Trait**:
```rust
#[async_trait]
pub trait ReActExecutor: Send + Sync {
    async fn init(&mut self) -> Result<(), WorkflowEngineError>;
    async fn run_loop(&mut self) -> Result<(), WorkflowEngineError>;
    async fn add_message_and_notify(...) -> Result<bool, WorkflowEngineError>;
    fn session_id(&self) -> String;
    fn state(&self) -> WorkflowState;
    fn set_state(&mut self, state: WorkflowState);
    fn messages(&self) -> Vec<WorkflowMessage>;
}
```

**核心问题点** (根据 plan.md):
- `signal_rx` 信号接收
- `pending_approvals` 审批管理
- `run_loop_internal` 主循环
- `init_internal` 初始化
- `update_state` 状态更新

---

### 2.2 gateway.rs (通信网关)

**位置**: `src-tauri/src/workflow/react/gateway.rs`
**规模**: 180 行, 7KB

**关键结构**:
```rust
pub struct TauriGateway {
    app_handle: AppHandle,
    input_senders: Mutex<HashMap<String, mpsc::Sender<String>>>,
    event_senders: Mutex<HashMap<String, mpsc::Sender<GatewayPayload>>>,
}
```

**关键方法**:
- `register_session_tx`: 注册 session 的输入通道
- `inject_input`: 向指定 session 注入信号 (行 148-167)
- `get_or_create_event_channel`: 创建聚合事件通道，带节流批处理 (行 49-129)

**当前职责**:
1. UI 事件发送 (Tauri event)
2. 输入注入 (inject_input)
3. 会话通道管理 (register_session_tx)

**问题**: 既负责 UI 事件发送，也负责输入注入，是后续解耦重点。

---

### 2.3 orchestrator.rs (编排器)

**位置**: `src-tauri/src/workflow/react/orchestrator.rs`
**规模**: 493 行, 19KB

**全局任务注册表**:
```rust
lazy_static::lazy_static! {
    /// Global registry for all background tasks (Sub-agents and Shell commands)
    pub static ref BACKGROUND_TASKS: Arc<DashMap<String, BackgroundTask>> = Arc::new(DashMap::new());
}
```

**BackgroundTask 枚举**:
```rust
pub enum BackgroundTask {
    /// An autonomous sub-agent running its own ReAct loop
    SubAgent(Arc<Mutex<dyn ReActExecutor>>),
    /// A raw shell command running in the background
    ShellCommand {
        command: String,
        stdout: Arc<Mutex<String>>,
        stderr: Arc<Mutex<String>>,
        status: Arc<Mutex<String>>,
    },
}
```

**SubAgentFactory Trait**:
```rust
#[async_trait]
pub trait SubAgentFactory: Send + Sync {
    async fn create_executor(
        &self,
        agent_id: &str,
        session_id: &str,
        task: &str,
        subagent_type: &str,
    ) -> Result<Arc<Mutex<dyn ReActExecutor>>, WorkflowEngineError>;
}
```

**问题**: `BACKGROUND_TASKS` 混合管理 workflow executor 和 shell 任务，生命周期分散。

---

### 2.4 types.rs (类型定义)

**WorkflowState 枚举** (当前状态):
```rust
pub enum WorkflowState {
    Pending,
    Running,
    Paused,          // 等待用户输入
    AwaitingApproval,
    AwaitingUser,    // 等待补充信息
    Completed,
    Failed,
    Cancelled,
}
```

**GatewayPayload 枚举**:
```rust
pub enum GatewayPayload {
    // 各种事件类型
}
```

**StepType 枚举**:
```rust
pub enum StepType {
    // 步骤类型
}
```

---

## 3. 命令层分析

**位置**: `src-tauri/src/commands/workflow.rs`

### 3.1 create_workflow (行 236-377)

```rust
pub async fn create_workflow(
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    request: CreateWorkflowRequest,
) -> Result<String, String>
```

**流程**:
1. 使用 TSID 生成唯一 session_id
2. 从 agent 配置构建 AgentConfig
3. 调用 `store.create_workflow()` 持久化
4. 生成 session_key 用于 proxy 认证

---

### 3.2 workflow_start (行 437-728)

```rust
pub async fn workflow_start(
    app: tauri::AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    session_id: String,
    agent_id: String,
    initial_prompt: Option<String>,
    planning_mode: Option<bool>,
) -> Result<String, String>
```

**核心流程**:
```
create_workflow → workflow_start
                      ↓
              [加载 workflow 快照和 agent 配置] (行 454-532)
                      ↓
              [Session 信号通道注册] (行 534-536)
                      │
                      │  let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
                      │  gateway_arc.register_session_tx(session_id.clone(), signal_tx).await;
                      ↓
              [构建执行器] (行 593-638)
                      │
                      │  根据 planning_mode 选择 PlanningExecutor 或 ExecutionExecutor
                      ↓
              [初始化执行器] (行 640-668)
                      │
                      │  executor.init()
                      │  add_message_and_notify()
                      ↓
              [注册到 BACKGROUND_TASKS] (行 670)
                      │
                      │  BACKGROUND_TASKS.insert(session_id, BackgroundTask::SubAgent(executor))
                      ↓
              [Spawn 后台运行循环] (行 674-724)
                      │
                      │  executor.run_loop()
                      ↓
              [错误处理后从 BACKGROUND_TASKS 移除]
```

**注册点**:
- `workflow_start` (行 670): 执行器启动时插入
- `workflow_approve_plan` (行 899): 计划批准后切换执行器

**移除点**:
- 执行器 run_loop 正常结束或取消时 (行 685, 724, 914, 953)

---

### 3.3 workflow_signal (行 960-1117)

```rust
pub async fn workflow_signal(
    app: tauri::AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    session_id: String,
    signal: String,
) -> Result<String, String>
```

**信号路由逻辑**:
```
workflow_signal
      ↓
[gateway.inject_input(&session_id, signal)] (行 973)
      ↓
  ┌─────────────────────────────────────┐
  │        注入结果判断                   │
  └─────────────────────────────────────┘
      ↓ 成功                    ↓ 失败
   [返回]              [进入恢复策略]
                              ↓
                    ┌─────────┴─────────┐
                    │                   │
              [user_input]        [approval]
                    │                   │
                    ↓                   ↓
           [调用 workflow_start]  [重试注入 5 次]
                    │                   │
                    ↓                   ↓
           [依赖消息反推恢复]    [依赖消息反推恢复]
```

**问题**:
1. 注入失败后拼接恢复逻辑
2. 恢复依赖消息反推 (transcript 解析)
3. 缺少统一的生命周期管理

---

## 4. 全局状态注册 (lib.rs)

**位置**: `src-tauri/src/lib.rs`

**当前 8 个全局状态，按依赖顺序注册**:

```rust
// === STATE REGISTRATION SECTION ===
// 位置: setup() 函数内 (第 501-755 行)

// State 1: MainStore (第 578 行) - 必须最先注册
app.manage(main_store.clone());

// State 2: ChatState (第 608-615 行)
// Depends on: MainStore, WindowChannels, AppHandle
let chat_state = ChatState::new(
    Arc::new(WindowChannels::new()),
    Some(app_handle_for_chat_state),
    main_store.clone(),
);
app.manage(chat_state.clone());

// State 3: FilterManager (第 618-625 行)
let filter_manager = crate::sensitive::manager::FilterManager::new();
app.manage(filter_manager);

// State 4: ScraperPool (第 628-630 行)
let scraper_pool = ScraperPool::new(app.handle().clone());
app.manage(scraper_pool);

// State 5: UpdateManager (第 633-635 行)
let update_manager = Arc::new(UpdateManager::new(app.handle().clone()));
app.manage(update_manager.clone());

// State 6: TsidGenerator (第 638-640 行)
let tsid_generator = Arc::new(crate::libs::tsid::TsidGenerator::new(1)
    .expect("Failed to init TSID generator"));
app.manage(tsid_generator.clone());

// State 7: TauriGateway (第 643-645 行)
let gateway = Arc::new(crate::workflow::react::gateway::TauriGateway::new(app.handle().clone()));
app.manage(gateway.clone());

// State 8: SubAgentFactory (第 648-655 行)
// Depends on: MainStore, ChatState, Gateway, TsidGenerator
let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
    main_store: main_store.clone(),
    chat_state: chat_state.clone(),
    gateway: gateway.clone(),
    app_data_dir: app.path().app_data_dir().unwrap_or_default(),
    tsid_generator: tsid_generator.clone(),
});
app.manage(factory);
```

**模式总结**:
| 模式 | 说明 |
|------|------|
| **依赖顺序** | 先注册被依赖的状态 |
| **Arc 包装** | 共享状态用 `Arc::new()` 包装 |
| **AppHandle 获取** | 使用 `app.handle().clone()` |
| **注释规范** | 每个状态标注依赖项 |

---

## 5. 当前问题汇总

根据 plan.md 描述，当前系统存在以下问题：

### 5.1 持久化状态与运行时状态分裂

| 层级 | 存储位置 | 问题 |
|------|----------|------|
| 持久化 | `workflows` + `workflow_messages` 表 | 不足以承载可靠恢复 |
| 运行时 | `BACKGROUND_TASKS` + `gateway.input_senders` | 分散在多处，生命周期不明确 |

### 5.2 恢复机制过度依赖消息反推

- `workflow_signal` 注入失败后，通过解析历史消息恢复
- approval 恢复依赖 `workflow_messages` 中的 JSON 反推
- 缺少结构化的 ExecutionContext

### 5.3 信号路由与 UI 页面状态耦合

- `signal channel closed` 问题
- 页面刷新后活跃 workflow 丢失
- 前端依赖旧状态字符串判断

### 5.4 状态机分散

- `WorkflowState` 枚举值散落在多个代码路径
- waiting 逻辑、approval 逻辑、user input 逻辑分散
- 缺少统一的等待模型

---

## 6. 阶段0前置信息

### 6.1 需要补日志的关键路径

根据 plan.md 第 7.3 节：

1. workflow create
2. workflow start
3. workflow resume
4. workflow signal
5. workflow stop
6. approval request
7. approval resume
8. executor crash

### 6.2 建议日志格式

```text
[Workflow][session={session_id}][phase={phase}] message...
```

关键字段:
- `session`
- `state`
- `wait_reason`
- `signal_type`
- `tool_call_id`
- `source`

### 6.3 必测场景 (plan.md 第 7.5 节)

1. 创建 workflow 并执行普通对话
2. 进入 `awaiting_approval` 后刷新页面
3. 进入 `awaiting_user` 后发送补充消息
4. 在 waiting 时发送 stop
5. 活跃 workflow 执行时关闭前端窗口并重新打开

---

## 7. 阶段1实施锚点

### 7.1 新增文件

- `src-tauri/src/workflow/react/manager.rs`

### 7.2 修改文件

- `src-tauri/src/lib.rs` - 注册 WorkflowManager
- `src-tauri/src/commands/workflow.rs` - 使用 manager 替代分散逻辑
- `src-tauri/src/workflow/react/orchestrator.rs` - 保留 BACKGROUND_TASKS 作为兼容层

### 7.3 WorkflowManager 最小接口

```rust
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

impl WorkflowManager {
    pub fn new() -> Self;
    pub fn register_session(...) -> Result<(), WorkflowManagerError>;
    pub fn remove_session(&self, session_id: &str) -> Option<ManagedSession>;
    pub fn has_session(&self, session_id: &str) -> bool;
    pub fn get_session_status(&self, session_id: &str) -> Option<ManagedSessionStatus>;
    pub fn get_executor(&self, session_id: &str) -> Option<Arc<Mutex<dyn ReActExecutor>>>;
    pub fn update_session_status(&self, session_id: &str, status: ManagedSessionStatus);
}
```

---

## 8. Worktree 信息

**创建命令**:
```bash
git worktree add ../chatspeed-workflow-refactor -b refactor/workflow-core Workflow
```

**路径**: `/Volumes/dev/personal/dev/ai/chatspeed-workflow-refactor`

**分支**: `refactor/workflow-core` (基于 `Workflow` 分支)

---

## 9. 下一步行动

1. 确认从 **阶段0** 开始
2. 在 worktree 目录执行阶段0任务：
   - 补统一日志
   - 梳理状态流转表
   - 补充测试场景
3. 完成后进行手工验证
4. 用户确认后进入阶段1
