# 第一里程碑收尾执行文档：重启后状态投影修正与前端状态-元素映射梳理

> 目的：解决“应用重启后，孤儿运行态 / 孤儿等待态在前端被错误投影”的问题，并一次性梳理 workflow 页面所有核心 UI 元素与状态之间的映射  
> 适用范围：仍属于第一里程碑收尾，不进入阶段 4  
> 使用方式：让执行 AI 先阅读本文件，再实施修改

---

## 1. 问题定义

当前你已经验证：

- workflow 的 waiting / signal / terminal resume 主链路基本可用
- 重启后结构化状态大体可恢复

但重启测试中又暴露了一个新的前端投影问题：

- 某个 workflow 在退出前处于 `thinking` 或其他非终态
- 应用退出后，内存里的 manager / executor 实际已经丢失
- 应用重启后，前端仍然根据 DB 中保留的旧 status 把它显示成“正在运行”
- 因此会出现：
  - Stop 按钮可点
  - 批准计划按钮状态不对
  - 某些 waiting / running 控件误显示

这个问题的本质不是“恢复失败”，而是：

**前端把“持久化状态”误当成了“活跃运行态”。**

也就是：

- DB 中的 `thinking / executing / auditing / awaiting_approval / awaiting_user / paused`
- 不等于应用重启后真的还有 live executor 在跑

这类问题必须在第一里程碑内修掉，不能带入阶段 4。

---

## 2. 为什么这仍属于第一里程碑问题

这不是 event store / replay 问题，也不是后续审计链问题。

它直接落在第一里程碑的三个核心目标上：

1. **阶段 1：session 生命周期只归后端管理**
   - 前端不能只靠旧 status 猜“session 还活着”
2. **阶段 2：snapshot 负责恢复最小真相**
   - snapshot 只说明“上次停在什么状态”，不说明“当前还有没有 live executor”
3. **阶段 3：前端基于结构化状态驱动**
   - 结构化状态里必须区分“持久化状态”和“实时活跃性”

因此，这个问题必须作为第一里程碑的最终 UI 收尾处理。

---

## 3. 本次整改的核心目标

整改后必须满足：

1. 应用重启后，前端不会把没有 live session 的 workflow 错误显示成“正在运行”
2. 应用重启后，前端不会错误显示等待态专属控件
3. “批准计划”按钮只能在**真实 approval waiting 且可恢复**时显示
4. Stop / Continue / 输入提示 / approval dialog / confirmation dialog 都必须同时考虑：
   - 持久化状态
   - `wait_reason`
   - 当前 session 是否真的 live

---

## 4. 必须先建立的原则

### 原则 1：持久化状态不等于 live 状态

重启后看到：

- `thinking`
- `executing`
- `auditing`
- `awaiting_user`
- `awaiting_approval`
- `paused`

只能说明：

- 上一次程序退出时，workflow 落在这些状态

不能直接说明：

- 当前还有 live executor
- 当前按钮仍应显示为可 Stop / 可批准 / 可继续

### 原则 2：前端按钮必须基于“投影态”，不是原始 status

前端真正应该驱动 UI 的，不应只是原始 `workflow.status`，而应是“投影后的 UI 状态”。

至少要综合：

- 持久化 `status`
- `wait_reason`
- 是否存在 live session / active executor
- 是否有可恢复的 waiting 业务上下文

### 原则 3：重启后的 orphan 状态必须被显式定义

应用重启后，最麻烦的是“孤儿状态”：

- orphan running-like state
- orphan waiting-like state

如果不定义清楚，前端就只能继续误判。

---

## 5. 建议采用的正式语义模型

## 5.1 区分两类状态

### A. Persistent State（持久化状态）

来自 DB / snapshot：

- `pending`
- `thinking`
- `executing`
- `auditing`
- `paused`
- `awaiting_user`
- `awaiting_approval`
- `awaiting_auto_approval`
- `completed`
- `error`
- `cancelled`

### B. Live Session State（实时活跃性）

这是本次要补齐的关键字段。

最少需要一个布尔值：

- `has_live_session`

含义：

- `true`：当前后端 manager 中仍存在 live session
- `false`：当前只是恢复到一份持久化记录，没有 live executor

如果可以更细，也可以扩展成：

- `session_presence = live | orphan`

但第一里程碑收尾阶段，不需要复杂建模，`has_live_session` 足够。

---

## 5.2 重启后的状态投影规则

### 情况 A：live session 存在

如果：

- `has_live_session = true`

那么前端可以按原有 waiting / running 语义展示。

即：

- `thinking / executing / auditing` -> 正在运行
- `awaiting_user` -> 等待用户输入
- `awaiting_approval` -> 等待审批
- `paused + confirmation` -> 等待继续/停止

### 情况 B：live session 不存在

如果：

- `has_live_session = false`

则前端必须进入“orphan 投影规则”。

#### orphan running-like state

包括：

- `thinking`
- `executing`
- `auditing`

这些状态在没有 live session 时，不应显示为“正在运行”。

正确投影应为：

- 不显示 Stop
- 不显示“正在运行中的交互入口”
- 允许用户通过发消息重新启动新一轮执行

也就是说，它们在 UI 语义上应被当成：

- “可恢复但当前未运行”

#### orphan approval waiting

包括：

- `awaiting_approval`
- `wait_reason = approval`
- `has_live_session = false`

这类状态不能直接显示“批准计划”按钮，也不能直接显示 approval dialog 已可操作。

正确行为应为：

- 如果后端支持通过 `workflow_start + rebroadcast_pending` 恢复 approval waiting，则先恢复 live session，再显示审批 UI
- 在未恢复 live session 前，不要显示 approval 操作按钮

#### orphan user_input waiting

包括：

- `awaiting_user`
- `wait_reason = user_input`
- `has_live_session = false`

正确行为应为：

- 可显示“上一次在等你的输入”的提示
- 但不要把它当成 live waiting 的实时状态
- 用户发消息时应触发恢复，而不是假定 loop 还在

#### orphan confirmation waiting

包括：

- `paused`
- `wait_reason = confirmation`
- `has_live_session = false`

正确行为应为：

- 不能仅凭旧状态自动弹 Continue/Stop dialog
- 应在用户进入该 workflow 时，先通过后端恢复该 waiting 上下文
- 确认恢复成功后，再展示 confirmation dialog

否则会出现：

- UI 直接弹出继续/停止
- 但实际上后端没有 live session

---

## 6. 最小实现建议

## 6.1 后端补充 `has_live_session`

### 推荐做法

在 `get_workflow_snapshot` 的返回里，补一个字段：

- `hasLiveSession`

其值来自：

- `WorkflowManager::has_session(session_id)`

这是本次最推荐的做法，因为：

- 不需要改 event store
- 不需要改 snapshot schema
- 不需要做复杂 replay
- 直接解决前端“当前有没有 live executor”这个判断盲区

### 为什么推荐放在 snapshot 返回层

因为前端现在很多判断都建立在：

- `selectWorkflow() -> get_workflow_snapshot()`

这正是应用重启后的关键入口。

只要这里补上 `hasLiveSession`，前端就能在恢复第一帧就投影正确。

---

## 6.2 前端 store 增加 live session 概念

在 workflow store 中新增一个面向 UI 的字段：

- `hasLiveSession`

并在 `selectWorkflow()` 时同步赋值。

不要再把 `isRunning` 仅仅建立在 status 上，而应至少改成：

- `isActivelyRunning = hasLiveSession && status in [thinking, executing, auditing, running]`
- `isWaiting = hasLiveSession && status in [paused, awaiting_user, awaiting_approval, awaiting_auto_approval]`

同时保留一个额外计算字段：

- `isRecoverableWaiting`

例如：

- `!hasLiveSession && status in [paused, awaiting_user, awaiting_approval]`

它表示：

- 这是上次停在 waiting，但当前不是 live waiting

这样前端就可以区分：

- live waiting
- orphan waiting

---

## 6.3 前端元素投影必须统一改成基于 live 状态

下面是建议的标准映射。

---

## 7. 前端元素与状态映射总表

以下表格是本次收尾阶段最重要的交付之一。执行 AI 应按此表逐项核对。

| UI 元素 | 当前语义 | 正确驱动条件 |
|--------|---------|-------------|
| Stop 按钮 | 停止当前活跃或等待中的 workflow | `hasLiveSession && (running-like or waiting-like)` |
| Continue 按钮 | 仅用于 confirmation waiting | `hasLiveSession && status=paused && waitReason=confirmation` |
| 批准计划按钮 | 仅用于 approval waiting | `hasLiveSession && waitReason=approval && 存在 pending approval` |
| approval dialog | 实时审批弹窗 | 只在 live approval waiting 且收到 confirm payload 后显示 |
| confirmation dialog | 继续 / 停止确认框 | 只在 live confirmation waiting 下显示 |
| “AI is waiting for your response” 输入提示 | user_input waiting 提示 | `hasLiveSession && waitReason=user_input` |
| Ask User 选项按钮 | Ask User 场景快捷输入 | `hasLiveSession && activeAskUser != null` |
| 发送消息 | 运行中 / waiting 中发 signal；终态 / orphan 状态走恢复或 restart | 按“live / orphan / terminal”分流 |
| 新建 workflow 按钮禁用态 | 当前有活跃 workflow 时禁用 | `hasLiveSession && (running-like or waiting-like)` |
| WorkflowMessageList 的 is-running | 流式运行动画 / 运行中视觉态 | `hasLiveSession && running-like` |
| isAwaitingApproval | approval 专属 UI 判定 | `hasLiveSession && waitReason=approval` |

---

## 8. 当前容易出错的映射点

下面这些点是本次重点要修的。

## 8.1 Stop 按钮

### 问题

如果只看持久化状态：

- orphan `thinking`
- orphan `awaiting_approval`

都可能被错误显示为可 Stop。

### 正确规则

只有在：

- `hasLiveSession = true`

时，Stop 才应显示。

---

## 8.2 Continue 按钮

### 问题

当前 `canContinue` 如果只看：

- `status === paused`

是不够的。

因为 orphan paused 也可能存在。

### 正确规则

必须同时满足：

- `hasLiveSession = true`
- `status === paused`
- `waitReason === confirmation`

才显示 Continue。

---

## 8.3 批准计划按钮

### 问题

当前如果只看：

- `waitReason === approval`
- 或 `status === awaiting_approval`

那么 orphan approval waiting 也可能被误认为“当前可审批”。

### 正确规则

必须同时满足：

- `hasLiveSession = true`
- `waitReason === approval`
- 存在 pending approval

否则不显示批准计划按钮。

---

## 8.4 approval dialog

### 问题

应用重启后，旧 workflow 仍可能带着等待审批的持久化状态，但实际上没有 live session。

这时如果直接显示审批 UI，会出现“看起来可审批，实际上后端 session 不在”的错觉。

### 正确规则

approval dialog 只应在以下流程后出现：

1. 选择 workflow
2. 判断为 orphan approval waiting
3. 先调用恢复逻辑，恢复 live session
4. 恢复成功后，通过 `rebroadcast_pending` 重新拿到 confirm payload
5. 再显示 approval dialog

也就是说：

- **显示 approval dialog 的前提是 live approval waiting，不是持久化 approval 状态。**

---

## 8.5 confirmation dialog

### 问题

当前如果前端在 `status=paused && waitReason=confirmation` 时直接弹 confirmation dialog，而没有区分 live / orphan，就会有错误。

### 正确规则

只有：

- `hasLiveSession = true`
- `status=paused`
- `waitReason=confirmation`

时才能直接弹。

如果是 orphan confirmation waiting，则必须：

1. 先恢复 live session
2. 再弹 dialog

---

## 8.6 user_input 提示条

### 问题

orphan `awaiting_user` 虽然不该被当成 live waiting，但它又确实表示“上一次停在等用户输入”。

### 推荐做法

可区分两层 UI：

- live user_input waiting：
  - 显示标准输入提示
  - 发送消息走 signal
- orphan user_input waiting：
  - 可以显示“上次停在等待你的回复”
  - 但发送消息应触发恢复逻辑，而不是直接假定 loop 活着

如果本次不想做两套文案，至少要保证：

- orphan user_input 不会误显示 live waiting 专属控件

---

## 9. 建议新增的前端语义字段

为了让页面逻辑可维护，建议在 store 中显式提供以下字段。

### 最小必需

- `hasLiveSession`
- `isActivelyRunning`
- `isLiveWaiting`
- `isOrphanWaiting`
- `canStop`
- `canContinue`
- `canApprovePending`

### 推荐定义

#### `isActivelyRunning`

```text
hasLiveSession && status in [thinking, executing, auditing, running]
```

#### `isLiveWaiting`

```text
hasLiveSession && status in [paused, awaiting_user, awaiting_approval, awaiting_auto_approval]
```

#### `isOrphanWaiting`

```text
!hasLiveSession && status in [paused, awaiting_user, awaiting_approval, awaiting_auto_approval]
```

#### `canStop`

```text
hasLiveSession && (isActivelyRunning || isLiveWaiting)
```

#### `canContinue`

```text
hasLiveSession && status === paused && waitReason === confirmation
```

#### `canApprovePending`

```text
hasLiveSession && waitReason === approval && hasPendingApproval
```

---

## 10. 推荐的恢复策略

## 10.1 orphan running-like state

### 建议行为

- 不显示 Stop
- 不显示“正在运行”
- 用户发消息时直接按 terminal/resumable 逻辑重新启动

## 10.2 orphan approval waiting

### 建议行为

- 进入 workflow 时识别为 orphan approval waiting
- 自动尝试恢复 live session
- 恢复成功后再发 `rebroadcast_pending`
- 只有拿到 confirm payload 后才显示 approval dialog / 批准计划按钮

## 10.3 orphan confirmation waiting

### 建议行为

- 进入 workflow 时不要立刻弹 Continue/Stop
- 先恢复 live session
- 恢复成功后再显示 confirmation dialog

## 10.4 orphan user_input waiting

### 建议行为

- 可以保留“等待你的输入”视觉提示
- 但不要把它算作 live waiting
- 用户发消息时应走恢复逻辑，而不是直接 signal 注入到一个不存在的 session

---

## 11. 建议修改文件

本次整改建议聚焦以下文件：

- `src-tauri/src/commands/workflow.rs`
- `src-tauri/src/commands/workflow.rs` 所依赖的 snapshot 返回结构
- `src/stores/workflow.js`
- `src/composables/workflow/useWorkflowCore.ts`
- `src/components/workflow/WorkflowInputArea.vue`
- 如有必要：
  - `src/views/Workflow.vue`
  - `src/components/workflow/ApprovalDialog.vue`

---

## 12. 推荐执行顺序

### 步骤 1：后端补 `hasLiveSession`

在 snapshot 返回里加：

- `hasLiveSession`

值来自 manager 是否持有当前 session。

### 步骤 2：前端 store 引入 live/orphan 投影字段

新增：

- `hasLiveSession`
- `isLiveWaiting`
- `isOrphanWaiting`
- `canApprovePending`

并统一 `isRunning` 语义。

### 步骤 3：修 WorkflowInputArea 的按钮条件

把：

- Stop
- Continue
- 新建 workflow 禁用
- 输入提示条

全部改成基于新语义字段，而不是直接看 status。

### 步骤 4：修 approval / confirmation 的显示入口

确保：

- orphan approval 不直接显示审批操作
- orphan confirmation 不直接弹 Continue/Stop dialog

### 步骤 5：补手工验证

重点验证“重启后旧状态投影”是否正确。

---

## 13. 手工测试清单

本次整改后，至少验证以下场景。

### 场景 A：重启后 orphan thinking

步骤：

1. 让 workflow 停在 `thinking`
2. 退出应用
3. 重启应用并进入该 workflow

验收：

- 不显示 Stop
- 不显示“正在运行”的错误视觉态
- 用户发送新消息可恢复执行

### 场景 B：重启后 orphan approval waiting

步骤：

1. 让 workflow 停在 `awaiting_approval`
2. 退出应用
3. 重启应用并进入该 workflow

验收：

- 初始不误显示“批准计划”按钮
- 恢复 live session 后，approval UI 才重新出现
- 审批可继续进行

### 场景 C：重启后 orphan user_input waiting

步骤：

1. 让 workflow 停在 `awaiting_user`
2. 退出应用
3. 重启应用并进入该 workflow

验收：

- 不误显示 live running / stop 语义
- 用户输入后可恢复执行

### 场景 D：重启后 orphan confirmation waiting

步骤：

1. 让 workflow 停在 `paused + confirmation`
2. 退出应用
3. 重启应用并进入该 workflow

验收：

- 初始不直接误弹 confirmation dialog
- 恢复后再显示 Continue/Stop
- Continue / Stop 可正常处理

---

## 14. 给执行 AI 的直接指令

如果由其他 AI 直接执行本整改，可按下面要求操作：

1. 在 workflow snapshot 返回中补 `hasLiveSession`
2. 前端 store 新增 live/orphan 语义字段，不再只看持久化 status
3. 修正所有核心 UI 元素的显示条件：
   - Stop
   - Continue
   - 批准计划
   - approval dialog
   - confirmation dialog
   - user_input 提示
   - 新建 workflow 按钮禁用
4. 确保 orphan running-like / orphan waiting-like 状态不会被误投影成 live 状态
5. 补齐手工测试，重点覆盖应用重启后的状态投影

执行时必须注意：

- 这仍属于第一里程碑收尾
- 不要引入 event store
- 不要引入 replay
- 不要扩展新的 waiting 模型
- 只修“持久化状态”和“live session 状态”的前端投影问题

