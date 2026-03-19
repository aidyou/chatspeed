# Workflow 模块架构文档

## 1. 模块概述

Workflow 模块是 ChatSpeed 的核心自主执行引擎，基于 ReAct (Reasoning + Acting) 模式实现。它让 AI 能够自主完成复杂的多步骤任务，包括代码编写、文件操作、网络搜索等。

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              用户界面 (Vue 3)                               │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐   │
│  │  Sidebar   │  │ MessageList │  │ InputArea   │  │  StatusPanel   │   │
│  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────────┘   │
└──────────────────────────────────────┬──────────────────────────────────────┘
                                       │ Tauri Events (workflow://event/*)
┌──────────────────────────────────────┴──────────────────────────────────────┐
│                          后端 (Rust + Tauri)                                │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                        Commands Layer                                 │  │
│  │  workflow_start / workflow_signal / workflow_stop / ...            │  │
│  └────────────────────────────────┬───────────────────────────────────────┘  │
│                                   │                                           │
│  ┌────────────────────────────────┴───────────────────────────────────────┐  │
│  │                    Gateway (TauriGateway)                             │  │
│  │         事件聚合 · 节流 · 前端通信                                    │  │
│  └────────────────────────────────┬───────────────────────────────────────┘  │
│                                   │                                           │
│  ┌────────────────────────────────┴───────────────────────────────────────┐  │
│  │                    ReAct Engine (核心)                               │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────────────┐   │  │
│  │  │ LLM      │  │ Context  │  │  Tool    │  │ Intelligence      │   │  │
│  │  │Processor │  │ Manager  │  │ Manager  │  │ Manager (Grader)   │   │  │
│  │  └──────────┘  └──────────┘  └──────────┘  └────────────────────┘   │  │
│  │                                                                  │   │  │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────────────┐   │  │
│  │  │ Security │  │ Skills   │  │ Memory   │  │ LoopDetector       │   │  │
│  │  │ (PathGurad)│ │ Scanner │  │ Manager  │  │                    │   │  │
│  │  └──────────┘  └──────────┘  └──────────┘  └────────────────────┘   │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
│                                   │                                           │
│  ┌────────────────────────────────┴───────────────────────────────────────┐  │
│  │                    数据层 (MainStore/SQLite)                          │  │
│  │        Workflow / Agent / WorkflowMessage / AgentConfig             │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. 核心组件详解

### 2.1 前端组件 (Vue 3)

#### 2.1.1 视图层

| 组件 | 文件位置 | 职责 |
|------|----------|------|
| `Workflow.vue` | `src/views/Workflow.vue` | 主视图，组装所有子组件和 composables |
| `WorkflowSidebar.vue` | `src/components/workflow/` | 工作流列表管理、侧边栏 |
| `WorkflowMessageList.vue` | `src/components/workflow/` | 消息渲染、流式输出显示 |
| `WorkflowInputArea.vue` | `src/components/workflow/` | 用户输入、技能/文件自动补全 |
| `WorkflowModelSelector.vue` | `src/components/workflow/` | 模型选择对话框 |
| `ApprovalDialog.vue` | `src/components/workflow/` | 工具执行审批对话框 |
| `StatusPanel.vue` | `src/components/workflow/` | 状态栏（通知、待办等） |

#### 2.1.2 Composables (逻辑复用层)

| Composable | 文件位置 | 职责 |
|------------|----------|------|
| `useWorkflowCore` | `src/composables/workflow/` | 核心：CRUD、启动/停止/信号、事件监听 |
| `useWorkflowChat` | `src/composables/workflow/` | 流式解析、chatState 管理、重试状态 |
| `useWorkflowMessages` | `src/composables/workflow/` | 消息增强、展开/收起、diff 解析 |
| `useWorkflowApproval` | `src/composables/workflow/` | 审批状态管理 |
| `useWorkflowSidebar` | `src/composables/workflow/` | 侧边栏折叠、宽度调整 |
| `useWorkflowInput` | `src/composables/workflow/` | 输入处理、技能/文件建议 |
| `useWorkflowPaths` | `src/composables/workflow/` | 路径管理、@mention 处理 |

#### 2.1.3 Store (状态管理)

| Store | 文件位置 | 职责 |
|-------|----------|------|
| `workflow.js` | `src/stores/workflow.js` | 工作流列表、消息、运行状态、通知 |

**workflow.js 关键状态：**
```javascript
workflows: []           // 所有工作流
currentWorkflowId      // 当前工作流 ID
messages: []           // 当前工作流消息
isRunning: false       // 是否运行中
notification: {}       // 通知消息
autoApprovedTools: []  // 自动批准的工具列表
shellPolicy: []        // Shell 策略规则
toolStreams: Map       // 工具流式输出缓存
```

### 2.2 后端组件 (Rust)

#### 2.2.1 Commands 层 (`src-tauri/src/commands/workflow.rs`)

这是前端与后端通信的入口：

| 命令 | 职责 |
|------|------|
| `create_workflow` | 创建新工作流 |
| `list_workflows` | 获取所有工作流 |
| `workflow_start` | 启动工作流执行（创建 ReActExecutor） |
| `workflow_signal` | 发送信号（用户输入、审批、停止等） |
| `workflow_stop` | 停止工作流 |
| `workflow_approve_plan` | 批准计划 |
| `update_workflow_status` | 更新工作流状态 |
| `get_workflow_snapshot` | 获取工作流快照（含消息历史） |

#### 2.2.2 Gateway 层 (`src-tauri/src/workflow/react/gateway.rs`)

**职责：**
- 前后端事件通信
- 消息节流（throttling）- 每 100ms 批量发送
- 代码块状态追踪（避免在代码块内截断）
- 流式内容聚合

**核心 trait：**
```rust
pub trait Gateway: Send + Sync {
    async fn send(&self, session_id: &str, payload: GatewayPayload) -> Result<()>;
    async fn inject_input(&self, session_id: &str, input: String) -> Result<()>;
    async fn register_session_tx(&self, session_id: String, tx: mpsc::Sender<String>);
}
```

#### 2.2.3 ReAct Engine 核心 (`src-tauri/src/workflow/react/engine.rs`)

**WorkflowExecutor 结构：**
```rust
pub struct WorkflowExecutor {
    pub session_id: String,
    pub context: ContextManager,        // 消息上下文管理
    pub tool_manager: Arc<ToolManager>, // 工具注册与执行
    pub gateway: Arc<dyn Gateway>,       // 事件发送
    pub llm_processor: LlmProcessor,    // LLM 调用
    pub intelligence_manager: IntelligenceManager, // 审核/评分
    pub compressor: ContextCompressor,   // 上下文压缩
    pub path_guard: Arc<RwLock<PathGuard>>, // 路径安全检查
    pub skill_scanner: SkillScanner,    // 技能扫描
    pub memory_manager: Arc<MemoryManager>, // 记忆管理
    pub loop_detector: LoopDetector,    // 循环检测
    pub policy: ExecutionPolicy,        // 执行策略
    // ...
}
```

**ReAct 执行循环：**
```
┌─────────────────────────────────────────────────────────────────┐
│                      ReAct Loop                                 │
│                                                                  │
│  ┌──────────────┐     ┌──────────────┐     ┌──────────────┐   │
│  │   THINK     │────▶│    ACT       │────▶│   OBSERVE    │   │
│  │  (LLM Call) │     │ (Tool Call)  │     │ (Get Result) │   │
│  └──────────────┘     └──────────────┘     └──────┬───────┘   │
│         │                                            │           │
│         │              ┌────────────────────────────┘           │
│         │              ▼                                        │
│         │        ┌──────────────┐                               │
│         │        │  Approval?   │                               │
│         │        └──────┬───────┘                               │
│         │               │                                       │
│         │    ┌──────────┴──────────┐                            │
│         │    ▼                     ▼                            │
│         │  Wait User          Auto                              │
│         │  Approval           Approve                           │
│         │    │                     │                            │
│         └────┴─────────────────────┘                            │
│                           │                                      │
│                           ▼                                      │
│                    Check Loop Exit                              │
│                    (Max Steps / Error / Done)                   │
└─────────────────────────────────────────────────────────────────┘
```

#### 2.2.4 子模块职责

| 模块 | 文件 | 职责 |
|------|------|------|
| `context.rs` | 上下文管理 | 消息持久化、上下文压缩、历史加载 |
| `llm.rs` | LLM 调用 | API 调用、流式处理、重试逻辑、Prompt 注入 |
| `intelligence.rs` | 智能审核 | 执行后审核 (Audit)、Grader 评分 |
| `runners.rs` | 执行器 | ExecutionExecutor 实现 |
| `planners.rs` | 规划器 | PlanningExecutor 实现 |
| `orchestrator.rs` | 编排器 | 子 Agent 工厂、Task 工具 |
| `gateway.rs` | 网关 | 前后端通信 |
| `compression.rs` | 压缩 | 上下文压缩 |
| `skills.rs` | 技能 | Skill 扫描和加载 |
| `memory.rs` | 记忆 | 项目记忆文件管理 |
| `security.rs` | 安全 | PathGuard 路径限制 |
| `loop_detector.rs` | 循环检测 | 检测重复工具调用 |
| `policy.rs` | 策略 | 执行策略（Approval/Phase） |
| `prompts.rs` | Prompt | 系统 Prompt 模板 |
| `observation.rs` | 观察 | 观察增强器 |

---

## 3. 数据流向

### 3.1 用户发送消息流程

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   用户输入   │────▶│ InputArea   │────▶│  Workflow   │
│              │     │ (Vue)       │     │  .vue       │
└──────────────┘     └──────────────┘     └──────┬───────┘
                                                  │
                                                  ▼
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│   Tauri      │◀────│ workflow_   │◀────│ useWorkflow  │
│   Events     │     │ signal cmd  │     │ Core         │
│              │     │ (Rust)      │     │              │
└──────┬───────┘     └──────────────┘     └──────────────┘
       │
       ▼
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│  Gateway    │────▶│ ReAct        │────▶│ LLM Processor│
│  inject     │     │ Executor     │     │              │
│  input      │     │ run_loop     │     │ call()       │
└──────────────┘     └──────────────┘     └──────┬───────┘
                                                   │
                                                   ▼
                                            ┌──────────────┐
                                            │  CCProxy     │
                                            │  (AI Backend)│
                                            └──────────────┘
```

### 3.2 事件推送流程 (后端 → 前端)

```
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│ LLM Response│     │  Gateway     │     │ Tauri        │
│ or Tool     │────▶│ send()       │────▶│ emit()       │
│ Result      │     │              │     │              │
└──────────────┘     └──────────────┘     └──────┬───────┘
                                                   │
                                                   ▼
                                            ┌──────────────┐
                                            │ Frontend     │
                                            │ Event        │
                                            │ Listener     │
                                            └──────┬───────┘
                                                   │
                                                   ▼
                                            ┌──────────────┐
                                            │ useWorkflow  │
                                            │ Core         │
                                            │ setupEvents  │
                                            └──────┬───────┘
                                                   │
                              ┌────────────────────┼────────────────────┐
                              ▼                    ▼                    ▼
                    ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
                    │ processChunk │     │ addMessage   │     │ setNotifi-   │
                    │ (流式文本)   │     │ (完整消息)   │     │ cation       │
                    └──────────────┘     └──────────────┘     └──────────────┘
```

### 3.3 GatewayPayload 类型

| 类型 | 用途 | 前端处理 |
|------|------|----------|
| `Chunk` | 流式文本片段 | 追加到 chatState.content |
| `ReasoningChunk` | 思考过程片段 | 追加到 chatState.reasoning |
| `Message` | 完整消息 | 存入 workflowStore.messages |
| `State` | 状态变更 | 更新 workflowStore.isRunning |
| `Confirm` | 需要审批 | 打开审批对话框 |
| `Notification` | 通知 | 显示状态栏通知 |
| `CompressionStatus` | 压缩状态 | 显示压缩提示 |
| `AutoApprovedToolsUpdated` | 自动批准列表更新 | 更新 store |
| `ShellPolicyUpdated` | Shell 策略更新 | 更新 store |
| `RetryStatus` | 重试状态 | 显示重试倒计时 |

---

## 4. 工作流状态机

```
┌─────────┐
│ Pending │  (初始状态)
└────┬────┘
     │ workflow_start
     ▼
┌─────────┐     ┌──────────┐
│Thinking │────▶│Executing │  (LLM 思考中)
└────┬────┘     └────┬─────┘
     │               │ LLM 返回工具调用
     │               ▼
     │        ┌──────────────┐     ┌───────────────┐
     │        │   Paused     │────▶│AwaitingApproval│
     │        │ (等待用户)   │     │ (等待审批)     │
     │        └──────────────┘     └───────┬───────┘
     │                                       │
     │                   ┌───────────────────┼───────────────────┐
     │                   ▼                   ▼                   ▼
     │            ┌────────────┐      ┌───────────┐      ┌───────────┐
     │            │  Approved  │      │Reject/Stop│      │ ApproveAll│
     │            └─────┬──────┘      └───────────┘      └─────┬─────┘
     │                  │                                    │
     │                  └──────────────┬──────────────────────┘
     │                                 │
     │                                 ▼
     │                         ┌────────────┐
     └────────────────────────▶│ Completed │  (正常结束)
                             └────────────┘

错误流程:
  Thinking/Executing ──[错误]──▶ ┌─────────┐
                                │  Error  │
                                └─────────┘

取消流程:
 任意状态 ──[stop信号]──▶ ┌──────────┐
                        │ Cancelled│
                        └──────────┘
```

---

## 5. 快速修改指南

### 5.1 修改消息显示

**问题：** 消息显示不正确、样式问题

**查找位置：**
1. `src/components/workflow/WorkflowMessageList.vue` - 消息渲染模板
2. `src/composables/workflow/useWorkflowMessages.ts` - 消息处理逻辑
3. `src/stores/workflow.js` - 消息存储

**关键函数：**
- `getParsedMessage()` - 解析消息中的工具调用
- `getDiffMarkdown()` - 渲染 diff 内容

### 5.2 修改输入处理

**问题：** 输入框行为、技能/文件自动补全不工作

**查找位置：**
1. `src/components/workflow/WorkflowInputArea.vue` - 输入组件
2. `src/composables/workflow/useWorkflowInput.ts` - 输入逻辑

### 5.3 修改工具执行

**问题：** 工具不执行、工具结果错误

**查找位置：**
1. `src-tauri/src/workflow/react/engine.rs` - `execute_tool_calls()`
2. `src-tauri/src/tools/` - 工具定义
3. `src-tauri/src/workflow/react/security.rs` - 路径检查

### 5.4 修改 LLM 调用

**问题：** 模型不响应、Prompt 不正确

**查找位置：**
1. `src-tauri/src/workflow/react/llm.rs` - LlmProcessor
2. `src-tauri/src/workflow/react/prompts.rs` - 系统 Prompt

### 5.5 修改上下文管理

**问题：** 上下文丢失、压缩时机不对

**查找位置：**
1. `src-tauri/src/workflow/react/context.rs` - ContextManager
2. `src-tauri/src/workflow/react/compression.rs` - 压缩逻辑

### 5.6 修改前端事件处理

**问题：** 事件不触发、状态不同步

**查找位置：**
1. `src/composables/workflow/useWorkflowCore.ts` - `setupWorkflowEvents()`
2. `src-tauri/src/workflow/react/gateway.rs` - 事件发送

---

## 6. 关键数据格式

### 6.1 WorkflowMessage (数据库)

```json
{
  "id": 123,
  "session_id": "workflow_xxx",
  "role": "user|assistant|system|tool",
  "message": "内容",
  "reasoning": "思考过程(可选)",
  "step_type": "Think|Act|Observe",
  "step_index": 1,
  "is_error": false,
  "metadata": {
    "type": "summary|tool_call",
    "tool_call_id": "xxx",
    "tool": { "name": "Read", "arguments": {} }
  }
}
```

### 6.2 GatewayPayload (前后端通信)

```rust
// 状态变更
GatewayPayload::State { state: WorkflowState::Thinking }

// 完整消息
GatewayPayload::Message {
    role: "assistant".into(),
    content: "...".into(),
    reasoning: Some("...".into()),
    step_type: Some(StepType::Think),
    step_index: 1,
    is_error: false,
    metadata: Some(json!({...}))
}

// 需要审批
GatewayPayload::Confirm {
    id: "tool_xxx".into(),
    action: "Read".into(),
    details: "path: /etc/passwd".into()
}
```

---

## 7. 调试技巧

### 7.1 后端日志

```rust
// 在 engine.rs 中启用调试日志
#[cfg(debug_assertions)]
log::debug!("WorkflowExecutor {}: ...", self.session_id, ...);
```

### 7.2 前端日志

```javascript
// 在 useWorkflowCore 中
console.log('Event payload:', payload)
```

### 7.3 常用调试命令

```bash
# 查看工作流列表
curl http://localhost:21912/api/workflow/list

# 获取快照
curl http://localhost:21912/api/workflow/snapshot?session_id=xxx
```

---

## 8. 文件索引

### 前端文件

| 路径 | 说明 |
|------|------|
| `src/views/Workflow.vue` | 主视图 |
| `src/stores/workflow.js` | 状态管理 |
| `src/components/workflow/*.vue` | UI 组件 |
| `src/composables/workflow/*.ts` | 逻辑 composables |

### 后端文件

| 路径 | 说明 |
|------|------|
| `src-tauri/src/commands/workflow.rs` | IPC 命令 |
| `src-tauri/src/workflow/react/engine.rs` | ReAct 引擎 |
| `src-tauri/src/workflow/react/gateway.rs` | 事件网关 |
| `src-tauri/src/workflow/react/context.rs` | 上下文管理 |
| `src-tauri/src/workflow/react/llm.rs` | LLM 调用 |
| `src-tauri/src/workflow/react/orchestrator.rs` | 子 Agent |
| `src-tauri/src/workflow/react/types.rs` | 类型定义 |

---

## 9. 常见问题排查

| 症状 | 可能原因 | 排查位置 |
|------|----------|----------|
| 消息不显示 | 事件未收到/解析错误 | gateway.rs + frontend event listener |
| 工具不执行 | 工具未注册/权限问题 | tool_manager / security.rs |
| 上下文丢失 | 压缩时机/数据库写入 | context.rs |
| 流式卡顿 | Gateway 节流/网络 | gateway.rs throttle_duration |
| 审批弹窗不显示 | 事件类型错误 | confirm event handling |
| Shell 命令失败 | 策略阻止/path guard | security.rs / policy.rs |

---

*文档版本: 2026-03-19*
*基于 ChatSpeed Workflow 模块源码生成*
