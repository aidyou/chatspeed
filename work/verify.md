# workflow模块逻辑校验

## 工作流的启动的两个方式

### 1. 用户创建工作流后在工作流聊天窗口发送信息

在这个模式下，用户会先创建一个没有任何信息的空的工作流。当用户在这个工作流当中输入信息，点击发送以后，会触发工作流的进一步动作。

创建工作流，工作流处于pending状态 -> 用户发送信息 -> 工作流切换为thinking状态开始正式启动工作这个时候实际上工作流在后台运行，无论用户是关闭窗口（会被拦截并转为隐藏状态）还是切换到其他的工作流，这个工作流应当始终处于后台工作状态。如果是关闭窗口（最小化），当前工作流仍然会自动输出并更新工作流窗口；如果用户切换到别的工作流，由于当前工作流id已经变更，那么前台的消息只会展示切换后的工作流（工作流信息匹配）

这一步需要注意的是，在刚创建工作流的时候，由于用户尚未发布任何消息，因此工作流是处于pending状态，所以文本输入框下面的"停止"或者"继续"按钮是不可见的，发送按钮处于灰色（用户尚未输入）

### 2. 状态恢复

工作流的可恢复状态：
- 当一个工作流处于工作中，程序退出时该工作流如果处于未结束状态
- 工作流出错停止
- 工作流被用户取消（用户按了停止按钮）
- 工作流已结束（被标记为完成状态）

#### 工作流的状态恢复有3个触发机制

- **用户发送信息**：
  - 如果工作流已完成，用户发送信息后，当前的工作流应该立即切换为thinking状态，步数重置为0，用户的问题可视为是对当前已完成工作流的进一步要求
  - 如果工作流未完成，用户发送消息后，在程序的流程上同上，切换状态为思考，重置步数为0，对ai来说意味着新上下文的补充或者对问题的回复等

- **工具恢复**：
  - **ask_user**：如果在工作流输出要求回答问题，但是用户没回答关闭了程序或者切换到别的工作流。当重新打开或者切换到当前工作流时，用户回答问题后需要恢复工作流
  - **等待审批**：当ai输出工作要求审批的时候，如果用户尚未审批就关闭了程序或者切换到别的工作流，当重新打开程序或者切换回当前工作流时，审批弹窗需要弹出，当用户审批或者拒绝后工作流继续

- **用户点击"继续"按钮**：当一个工作流未结束时，无论是重启了程序还是从别的工作流切换过来，前端都需要展示"继续"按钮。当用户点击后，工作流恢复为thinking状态续接上一步继续工作。

#### 工作流的恢复机制

无论用户发送消息（ask_user的回复实际也是发送消息）还是工具审批，实际上都是发送信号给后端。Rust端收到信号以后，应该根据工作流的状态决定如何操作。如果工作流是处于停止并销毁的状态，那么应该创建一个新的工作流，加载历史消息，恢复为可工作状态。如果工作流是处于工作的某个状态，那么这个时候直接把信号通过网关发送进去并切换为合适的状态就可以了。

这里有一点需要注意的是，如果工作流处于销毁状态，信息有可能无法直接发送进去，所以应该是在恢复工作流以后，才将信息发送进去，这跟网关信息的传递机制有关，需要确认一下。

---

## 补充：工作流状态定义（基于代码分析）

### ReAct 工作流引擎状态（AI 智能体工作流）

**定义文件**: `src-tauri/src/workflow/react/types.rs`

| 状态 | 含义 | 触发条件 |
|------|------|----------|
| pending | 初始/空工作流 | 用户创建工作流，还未发送消息 |
| thinking | AI 思考中 | 用户发送消息后，或恢复工作流 |
| executing | 工具执行中 | AI 调用工具 |
| auditing | 工具输出审计中 | 工具返回结果后，发送 LLM 前进行审计 |
| paused | 暂停/等待用户输入 | AI 输出 ask_user 工具，或用户点击停止 |
| awaiting_approval | 等待用户审批 | AI 输出需要用户审批的内容（submit_plan 工具） |
| awaiting_auto_approval | 自动审批过渡 | 内部状态，审批通过后自动进入执行阶段 |
| completed | 已完成 | 任务完成 |
| error | 出错 | 执行出错 |
| cancelled | 已取消 | 用户点击停止按钮 |

---

## 补充：资源生命周期与恢复机制

### 资源管理

```
启动 → 注册到 BACKGROUND_TASKS + gateway sender
    ↓
运行中 → 状态变化 + pending_approvals 缓存更新
    ↓
结束(Completed/Error/Cancelled) → BACKGROUND_TASKS.remove + gateway sender 销毁
```

**关键点**：
- `pending_approvals`: DashMap 缓存，运行时存储待审批的工具调用详情
- `workflows.status`: SQLite 数据库持久化工作流状态
- `workflow_messages.metadata`: JSON 格式存储消息级别元数据（如 approval_status）

### 恢复流程

**Step 1：前端发送信号**

前端通过 `workflow_signal` 或 `workflow_start` 发送信号到后端。

**Step 2：后端处理**

```
情况A：gateway sender 存在（工作流正在运行）
    ↓
直接通过 sender 发送信号，状态切换

情况B：gateway sender 不存在（工作流已停止/程序重启）
    ↓
检查工作流状态：
    - completed/error/cancelled → 重新创建执行器，加载历史消息，恢复运行
    - paused/awaiting_approval → 重新创建执行器，恢复到暂停前的状态
```

**Step 3：ask_user/审批的恢复**

这两个场景特殊之处在于：
1. 前端需要展示 UI（展示问答到聊天窗上方 / 审批弹窗）
2. 用户操作后，信号传给后端
3. 后端需要恢复工作流到暂停前的状态，然后注入用户的响应

---

## 补充：功能实现确认（基于代码分析）

| 功能 | 实现状态 | 关键文件 |
|------|---------|---------|
| **ask_user 工具** | ✅ 已完整实现 | `src-tauri/src/tools/interaction.rs` |
| **ask_user 状态持久化** | ✅ 已实现 | 状态存储在 `workflows.status = 'paused'`，消息存 `workflow_messages` 表 |
| **审批功能** | ✅ 已完整实现 | `src-tauri/src/workflow/react/interceptors.rs` (handle_approval_interception) |
| **审批状态持久化** | ✅ 已实现 | 状态存储在 `workflows.status = 'awaiting_approval'`，审批详情存 `pending_approvals` 缓存 + 消息 metadata |
| **审批策略** | ✅ 已实现 | `src-tauri/src/workflow/react/policy.rs` (ApprovalLevel: Default/Smart/Full) |
| **状态恢复机制** | ✅ 已实现 | `src-tauri/src/commands/workflow.rs` (workflow_signal 命令) |
| **程序重启后恢复** | ✅ 已实现 | 重启后发送消息自动触发工作流重建 |

---

## 补充：关键代码路径

### 后端核心文件

| 功能 | 文件路径 |
|------|---------|
| 状态枚举定义 | `src-tauri/src/workflow/react/types.rs` |
| 引擎主循环 | `src-tauri/src/workflow/react/engine.rs` |
| 状态更新 | `src-tauri/src/workflow/react/engine.rs` (update_state 函数，约 L2164) |
| ask_user 拦截 | `src-tauri/src/workflow/react/interceptors.rs` (handle_ask_user_intercept) |
| 审批拦截 | `src-tauri/src/workflow/react/interceptors.rs` (handle_approval_interception) |
| 审批策略 | `src-tauri/src/workflow/react/policy.rs` |
| 数据库操作 | `src-tauri/src/db/workflow.rs` |
| 命令处理 | `src-tauri/src/commands/workflow.rs` (workflow_signal, workflow_start) |

### 前端核心文件

| 功能 | 文件路径 |
|------|---------|
| 工作流状态存储 | `src/stores/workflow.js` |
| 核心逻辑（状态检测） | `src/composables/workflow/useWorkflowCore.ts` |
| 审批逻辑 | `src/composables/workflow/useWorkflowApproval.ts` |
| 审批弹窗 | `src/components/workflow/ApprovalDialog.vue` |
| 输入区域（按钮显示） | `src/components/workflow/WorkflowInputArea.vue` |
| 工作流视图 | `src/views/Workflow.vue` |

---

## 补充：前端状态检测与 UI 显示

### 状态存储与检测

**状态存储** (`src/stores/workflow.js`):
```javascript
const isRunning = ref(false)  // 运行状态标志

// selectWorkflow 时根据 status 判断
isRunning.value = [
  'thinking', 'executing', 'auditing',
  'awaiting_approval', 'awaiting_auto_approval',
  'paused', 'running'
].includes(status)
```

**状态计算属性** (`src/composables/workflow/useWorkflowCore.ts`):
```typescript
const isAwaitingApproval = computed(() => 
  currentWorkflow.value?.status === 'awaiting_approval'
)
```

### 按钮显示逻辑

**WorkflowInputArea.vue 第 183-192 行**:
```vue
<!-- 审批按钮 -->
<el-button v-if="isAwaitingApproval" size="small" round type="success" @click="$emit('approve-plan')">
  {{ $t('workflow.approvePlan') }}
</el-button>

<!-- 继续按钮 -->
<el-button
  v-if="!isRuning && !isAwaitingApproval && currentWorkflowId && 
        currentWorkflow?.status !== 'pending' && 
        currentWorkflow?.status !== 'completed' && 
        currentWorkflow?.status !== 'error'"
  size="small" round type="primary" @click="$emit('continue')">
  {{ $t('workflow.continue') }}
</el-button>

<!-- 停止按钮 -->
<cs name="stop" @click="$emit('stop')" v-if="isRunning" />
```

### ask_user 前端处理

**检测 ask_user 状态** (`src/views/Workflow.vue` 第 362-374 行):
```javascript
const activeAskUser = computed(() => {
  if (currentWorkflow.value?.status !== 'paused') return null
  const lastMsg = enhancedMessages.value[enhancedMessages.value.length - 1]
  if (
    lastMsg?.role === 'tool' &&
    lastMsg.toolDisplay?.action === 'Ask User' &&
    lastMsg.toolDisplay?.displayType === 'choice'
  ) {
    return parseChoiceContent(removeSystemReminder(lastMsg.message))
  }
  return null
})
```

**消息格式**: 后端发送 JSON 格式
```json
{
  "question": "请问您想继续还是停止？",
  "options": ["继续", "停止", "修改计划"]
}
```

**发送用户回答** (`src/composables/workflow/useWorkflowCore.ts` 第 424-452 行):
```typescript
const signal = JSON.stringify({
  type: 'user_input',
  content: message
})
// 乐观更新状态
if (isPaused) {
  workflowStore.updateWorkflowStatus(currentWorkflowId.value, 'thinking')
}
await invokeWrapper('workflow_signal', {
  sessionId: currentWorkflowId.value,
  signal: signal
})
```

### 审批弹窗前端处理

**弹窗触发** (`src/composables/workflow/useWorkflowCore.ts` 第 223-227 行):
```typescript
} else if (payload.type === 'confirm') {
    approvalRequestId.value = payload.id
    approvalAction.value = payload.action
    approvalDetails.value = payload.details
    approvalVisible.value = true
}
```

**审批操作** (`src/composables/workflow/useWorkflowApproval.ts`):
```typescript
// Approve
const signal = JSON.stringify({
  type: 'approval',
  approved: true,
  id: approvalRequestId.value
})

// Reject
const signal = JSON.stringify({
  type: 'approval',
  approved: false,
  id: approvalRequestId.value
})

await invokeWrapper('workflow_signal', { sessionId, signal })
```

**重启后恢复** (`src/views/Workflow.vue` 第 456-470 行):
```typescript
nextTick(() => {
  if (
    currentWorkflow.value?.status === 'awaiting_approval' &&
    lastMsg?.role === 'tool'
  ) {
    approvalRequestId.value = lastMsg.metadata?.tool_call_id || 'restored'
    approvalAction.value = lastMsg.toolDisplay.action
    approvalDetails.value = removeSystemReminder(lastMsg.message)
    approvalVisible.value = true
  }
})
```

---

## 补充：前后端通信机制

### 事件通道

**后端 → 前端**：通过 Tauri 事件 `workflow://event/{sessionId}` 发送状态变更

```rust
// 后端发送示例 (engine.rs)
self.gateway.send(&self.session_id, GatewayPayload::State { state: new_state }).await?;
self.gateway.send(&self.session_id, GatewayPayload::Confirm { ... }).await?;
```

**前端监听** (`useWorkflowCore.ts`):
```typescript
const unlisten = await listen(`workflow://event/${sessionId}`, (event) => {
  const payload = event.payload as GatewayPayload
  // 处理 state/confirm/message 等不同类型
})
```

### 命令调用

**前端 → 后端**：通过 Tauri invoke 调用命令

| 命令 | 用途 |
|------|------|
| `workflow_start` | 启动/恢复工作流 |
| `workflow_signal` | 发送用户输入/审批信号 |
| `get_workflow_snapshot` | 获取工作流快照（状态+消息） |

---

## 补充：Gateway Sender 说明

**gateway sender** 是工作流执行器的消息通道，每个运行中的工作流实例持有一个：

```
┌─────────────────┐     gateway      ┌─────────────────┐
│  WorkflowEngine │ ──发送事件──→ │ Frontend        │
│  (Rust)         │                 │ (Vue)           │
│                 │ ←─接收信号── │                 │
└─────────────────┘   workflow_    └─────────────────┘
                      signal
```

- 程序重启后，gateway sender 随执行器销毁而丢失
- 恢复时通过 `workflow_start` 重新创建执行器和 sender
- `workflow_signal` 会先尝试注入信号，失败则重建执行器

---

## 补充：已识别的问题点（需要修复）

### 一、ask_user 恢复问题

| # | 问题 | 位置 | 影响 | 优先级 | 状态 |
|---|------|------|------|--------|------|
| 1 | **workflow_start 重建 channel 时存在竞态** | `workflow.rs:632-636` | 前端信号可能发送到不存在的 channel | 高 | 待进一步调查 |
| 2 | **空 content 仍会切换状态（核心 bug）** | `engine.rs:1143-1167` | ask_user 恢复时收到空消息，AI 继续工作但不记得要用户回答 | **高** | ✅ 已修复 |
| 3 | **没有 AwaitingUser 状态** | `types.rs` | ask_user 和其他暂停原因共用 Paused，无法区分 | 中 | ✅ 已修复 |
| 4 | **恢复时可能重复添加消息** | `workflow.rs:647-656` | 从 DB 加载历史 + 恢复时添加 = 重复 | 中 | ✅ 已修复 |
| 5 | **前端 activeAskUser 未检查 awaiting_user 状态** | `Workflow.vue:362-373` | ask_user UI 选项不会显示，用户无法选择回答 | **高** | ❌ 待修复 |

**核心 bug 详情**：

用户报告的现象：
- ask_user 恢复时（程序重启后），页面加载后工作流"自动进行"
- AI 收到一条空消息
- AI 说"用户发送了一条空消息，让我继续"
- 导致 ask_user 的提问没有被回答，工作流继续往下走

**用户补充**：这条空消息不是用户主动发送的，而是页面加载后自动触发的。

问题代码 (`engine.rs:1143-1167`):
```rust
let user_input = signal_json["content"].as_str().unwrap_or("").to_string();
if !user_input.is_empty() {
    self.add_message_and_notify_internal(...).await?;
} else {
    log::debug!("Received signal with empty content, ignoring");
}
// ❌ 问题：即使 content 为空，仍然切换状态！
self.update_state(WorkflowState::Thinking).await?;
continue;
```

**修复建议**：当 content 为空时，**不应该**切换到 `Thinking` 状态，而应该**继续等待**用户输入。

**可能的触发场景**（待进一步确认）：
1. 用户可能不小心点击了"继续"按钮（按钮在 paused 状态下可见）
2. 或者有其他自动触发 workflow_start 的逻辑

#### 问题 5：前端 activeAskUser 未检查 awaiting_user 状态

**位置**: `Workflow.vue:362-373`

**问题代码**:
```javascript
const activeAskUser = computed(() => {
  if (currentWorkflow.value?.status !== 'paused') return null  // ❌ 只检查 'paused'
  const lastMsg = enhancedMessages.value[enhancedMessages.value.length - 1]
  if (
    lastMsg?.role === 'tool' &&
    lastMsg.toolDisplay?.action === 'Ask User' &&
    lastMsg.toolDisplay?.displayType === 'choice'
  ) {
    return parseChoiceContent(removeSystemReminder(lastMsg.message))
  }
  return null
})
```

**问题原因**：
- 后端在提交 `c6e17e3` 中将 `ask_user` 的状态从 `Paused` 改为 `AwaitingUser`
- 但前端的 `activeAskUser` 计算属性仍然只检查 `status === 'paused'`
- 导致当工作流因 `ask_user` 暂停时，前端不会显示问答选项 UI

**影响**：
- 用户无法看到 ask_user 提供的选项按钮
- 只能手动在输入框中输入回答，用户体验受损

**修复建议**：
```javascript
const activeAskUser = computed(() => {
  const status = currentWorkflow.value?.status
  if (status !== 'paused' && status !== 'awaiting_user') return null  // ✅ 同时检查两种状态
  // ... 其余逻辑不变
})
```

### 二、审批恢复问题

| # | 问题 | 位置 | 影响 | 优先级 | 状态 |
|---|------|------|------|--------|------|
| 1 | ~~前端检测条件过于严格~~ | ~~Workflow.vue:462-468~~ | ~~用户已确认正确，不需要修改~~ | - | - |
| 2 | **后端 tool_info 查找可能失败** | `engine.rs:387-410` | metadata 格式不匹配时恢复失败 | 高 | ✅ 已修复 |
| 3 | **200ms 固定等待可能不足** | `workflow.rs:1034` | 系统负载高时恢复不完整 | 中 | ✅ 已修复 |
| 4 | **前端事件监听器可能未就绪** | `useWorkflowCore.ts:169-220` | Confirm 信号丢失 | 高 | ✅ 已修复 |
| 5 | **status 状态同步延迟** | `workflow.js:174-183` | UI 显示与实际状态不一致 | 低 | ✅ 已优化 |

**核心问题**：前后端恢复流程的时序配合不当，缺少"就绪确认"机制。

### 三、详细问题说明

#### 问题 1：workflow_start 重建 channel 竞态

**位置**: `workflow.rs:632-636`

```rust
let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
gateway_arc.register_session_tx(session_id.clone(), signal_tx).await;
```

**问题**：恢复时传入 `initial_prompt`，但会创建**新的** `signal_rx`。如果前端在恢复完成前发送信号，可能注入到旧的/不存在的 channel。

#### 问题 2：200ms 固定等待不足

**位置**: `workflow.rs:1034`

```rust
tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
```

**问题**：固定 200ms 可能在系统负载高时不足以完成 `init_internal()` 和 `pending_approvals` 恢复。

#### 问题 3：前端事件监听器未就绪

**位置**: `useWorkflowCore.ts:169-220`

**问题**：`setupWorkflowEvents` 在 `selectWorkflow` 中调用，但如果在事件监听器设置完成前，后端的 `GatewayPayload::Confirm` 信号已发送，前端会丢失该信号。

**修复方案**：
1. 后端已有 `request_confirm_broadcast` 信号处理器 (engine.rs:906-928)
2. 前端在 `setupWorkflowEvents` 完成后发送 `request_confirm_broadcast` 信号
3. 后端重新广播待处理的 `GatewayPayload::Confirm` 事件
4. `pending_approvals` 现在存储 `details` 字段，确保重新广播时内容完整
5. 在 `workflow_signal` 中添加 `request_confirm_broadcast` 处理，当引擎未运行时自动恢复工作流

#### 问题 4：后端 tool_info 查找失败

**位置**: `engine.rs:387-410`

```rust
let tool_info = self.context.messages.iter().rev().find_map(|m| {
    if m.role == "assistant" {
        // 查找匹配的 tool_call_id
    }
});
```

**问题**：如果 assistant 消息的 metadata 格式不匹配（`tool_calls` vs `tool` 字段），可能导致找不到原始参数。

---

## 补充：待验证/待优化的问题

| 问题 | 现状 | 验证方式 |
|------|------|---------|
| 窗口最小化时消息展示 | Rust 拦截转为最小化 | 实际测试：最小化后工作流是否继续输出 |
| ask_user 恢复完整性 | 状态+消息已持久化，代码有恢复逻辑 | 实际测试：paused 状态重启后回答问题 |
| 审批恢复完整性 | 代码有 `awaiting_approval` 恢复逻辑 | 实际测试：审批弹窗状态重启后恢复 |
| 连续消息处理 | gateway sender 存在时直接投递 | 实际测试：快速连续发送多条消息 |
| 并发信号投递 | 依赖后端 channel | 需确认 channel 容量和阻塞策略 |

---

## 补充：下一步计划建议

### 测试验证（必须）

1. **测试程序重启场景**：重启后发送消息，验证工作流能否正确恢复
2. **测试 ask_user 恢复**：在 ask_user 状态下关闭程序，重新打开后回答问题
3. **测试审批恢复**：在等待审批状态下关闭程序，重新打开后审批弹窗是否弹出

### 边界测试

4. **验证连续消息**：测试快速连续发送多条消息时的工作流行为
5. **验证最小化场景**：窗口最小化后工作流是否继续运行并输出结果
6. **验证切换工作流**：从工作流 A 切换到工作流 B，A 是否继续在后台运行