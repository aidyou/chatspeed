# 工作流工具同步与 MCP 独立配置实施计划

## 1. Problem Statement

当前工作流工具配置存在三个相互影响的问题：

1. 工作流任务段的工具快照与 Agent 最新配置之间缺少统一、可观测的边界同步。新工作流、完成后续发、手动清空上下文虽然已有部分同步代码，但运行时重建、工作流快照、前端显示和自动审批列表没有形成一个完整的实时一致链路；前端还可能从较新的 Agent 定义回退取值，导致显示集合与当前任务实际集合不一致。
2. `WorkflowInputArea.vue` 的工具弹层需要严格体现当前任务的权威配置：当前任务可用工具必须是 Agent 配置的子集，自动审批必须是当前任务可用工具的子集；当前实现已有部分过滤，但可用列表仍直接依赖 Agent 的通用工具列表，不能清晰表达 MCP 独立配置和边界同步结果。
3. `Agent.vue` 将 MCP 工具和普通工具放在同一个 `availableTools` 下拉框，并用 `mcpToolExposure` 另行控制“直接展开”。用户安装 MCP 后必须手动在混合下拉框中启用，配置路径长；子代理也没有独立的 MCP 配置展示。

目标是保持现有工作流生命周期、审批、Shell policy、Skills 和子代理权限约束不变，同时把 MCP 配置独立成每个 Agent（包括 child Agent）自己的三项配置：可用、自动审批、自动展开。现有持久化字段应向后兼容，不引入不必要的数据库迁移。

范围包括 Rust workflow/Agent 配置归一化与任务边界同步、前端 Agent 配置页、工作流工具弹层、相关 i18n 和聚焦测试。非目标包括重写 MCP 服务器管理、改变 MCP 协议/工具命名、改变子代理的既有禁止工具规则、改变 Shell 审批模型、压缩/上下文算法或引入新的依赖。

## 2. Target Outcome and Acceptance Contract

目标行为：

- Agent 页面中普通工具列表只配置非 MCP 工具；MCP 工具在 Skills 配置下方独立展示，每项都有“可用 / 自动审批 / 自动展开”三个独立开关。
- MCP 工具只要已安装并由 `get_available_tools` 发现，就出现在独立列表；用户可直接勾选“可用”，不需要再修改普通工具列表。
- 每个新任务段开始时，后端从当前 Agent 配置重新计算工作流有效工具配置，并过滤旧工作流选择；有效工作流工具集合不得超出 Agent 能力集合，自动审批不得超出有效工作流可用集合。当前运行中的任务段不因 Agent 编辑而静默变更。
- 首次启动、完成后续发、手动清空上下文后开始的新任务段，都使用同一个边界同步路径，并向前端发送结构化的当前工作流配置，使 UI 与运行时一致。
- 子代理使用其自身 Agent 的 MCP 三项配置；仍保留后端对子代理禁用 `bash`、子代理委派/输出/停止、计划工具和 `ask_user` 等既有规则。
- 旧数据兼容：原 `mcp_tool_exposure` 数组继续作为“自动展开”集合；原 `available_tools` 中的 MCP 项迁移为 MCP“可用”；原 `auto_approve` 中的 MCP 项迁移为 MCP“自动审批”。归一化后 MCP 不再依赖普通工具 UI。

验收标准：

- **AC-1**：普通 Agent 工具配置 UI 不再把 `category === 'MCP'` 工具放入普通可用工具/自动审批下拉；已发现 MCP 工具在 Skills 下方独立列表展示“可用、自动审批、自动展开”三个可操作状态。
- **AC-2**：主 Agent 和 child Agent 均可保存并重新加载独立 MCP 三项配置；child Agent 不再要求通过普通 `availableTools` 配置 MCP，且原有 child-only/禁止工具规则仍生效。
- **AC-3**：兼容旧 Agent/Workflow 配置：旧 MCP 数组映射到自动展开，旧通用列表中的 MCP 映射到 MCP 可用，旧自动审批列表中的 MCP 映射到 MCP 自动审批；去重、过滤不存在/禁用 MCP 工具，并保持非 MCP 配置不丢失。
- **AC-4**：新工作流创建时，运行时有效可用工具是当前 Agent 工具能力的子集，MCP 有效工具来自独立 MCP 可用配置，自动审批是有效可用工具的子集，自动展开只作用于有效 MCP 工具。
- **AC-5**：完成任务后续发和手动清空上下文后的新任务段都调用同一个 Agent 工具边界同步逻辑，重新加载最新 Agent 配置、过滤旧工作流选择、重建运行时工具注册，并向前端发送同步后的结构化配置。
- **AC-6**：运行中任务段的工具集合不会因为 Agent 配置编辑而静默变化；现有显式工作流工具配置更新仍可通过当前的结构化工作流配置路径生效，并且所有运行时更新继续过滤自动审批集合。
- **AC-7**：`WorkflowInputArea.vue` 的“可用工具”展示只来自当前工作流有效配置与当前已发现工具元数据的交集；自动审批展示只来自当前工作流可用集合；不会从较新的未同步 Agent 定义扩大当前任务显示集合。
- **AC-8**：前端监听边界同步后的结构化配置后，工作流对象、自动审批状态、MCP 配置/工具显示即时更新；手动切换可用工具时自动审批同步移除，MCP 三个开关分别持久化。
- **AC-9**：相关 Rust 单元测试、前端结构合约测试和现有 workflow focused test 命令通过；验证首次启动、完成后续发、手动清空上下文、MCP 兼容归一化、子代理过滤和前端集合包含关系。

受保护不变量：

- **INV-1**：后端仍是 workflow 生命周期、等待、审批、恢复和当前任务工具运行状态的唯一权威；前端不从 transcript 文本猜测工具状态。
- **INV-2**：自动审批工具永远是当前任务 AI-visible 可用工具的子集；Shell auto policy 仍与普通工具自动审批分离。
- **INV-3**：已有 MCP 工具组合名、`ToolManager` 注册/调用机制、MCP loader 折叠/直接展开机制和禁用 MCP 工具安全检查保持不变。
- **INV-4**：Agent 工具变更只在新 workflow、完成任务续发、手动 clear-context 等规范任务边界同步；运行中的任务段不被后台 Agent 编辑改变。
- **INV-5**：子代理已有的工具过滤、不能使用 Shell/交互/委派控制工具的规则保持不变。
- **INV-6**：现有 Skills、Shell policy、sandbox、审批级别、上下文和任务生命周期行为不因本次配置拆分改变。

## 3. Current State and Evidence

适用规则：`src-tauri/src/workflow/react/CONSTITUTION.md` 的 7.5 要求 auto-approval 是当前可见工具子集，并规定 Agent 工具变更只在新 workflow、完成任务续发、手动 clear-context 等任务边界同步；7.6 要求 Shell policy 独立且允许规则累积；3.1/10.4 要求后端权威和结构化 UI 状态。`AGENTS.md` 和 `src-tauri/AGENTS.md` 要求使用现有 Vue 3/Pinia/i18n、Rust `Result`/`?`、小范围变更、不得未经请求引入数据库迁移或依赖。

确认的当前实现：

- `src-tauri/src/db/agent.rs:37-65,120-160`：持久化 `Agent` 使用 JSON 字符串字段 `available_tools`、`auto_approve`、`mcp_tool_exposure`；workflow snapshot 使用 `AgentConfig` 的 typed vectors。
- `src-tauri/src/commands/agent.rs:1-95,283-306`：`get_available_tools` 返回 native/MCP 工具元数据，MCP 由 `category` 区分；child Agent 保存时已有 Shell、角色和工具过滤。
- `src-tauri/src/commands/workflow.rs:1098-1148`：创建 workflow 从 Agent 构建 `AgentConfig`；`1288-1389` 的继承合并保留当前 Agent 能力与工作流已选工具的交集，并过滤 auto-approve；`1481-1515` 的 `sync_workflow_agent_config_at_tool_boundary` 在边界同步 Agent 配置；`2717-2859` 的 clear-context 路径已有边界同步；`3835` 附近的 `workflow_start` 会在 completed 续发前同步。
- `src-tauri/src/workflow/react/engine.rs:586-615,7159-7290,7667-7695,8142-8152`：MCP 当前由 `available_tools` 过滤可用性、`mcp_tool_exposure` 控制完整 schema/折叠，`UpdateAvailableTools` 会重建 foundation tools；completed resume 会从 snapshot 刷新运行时工具配置。
- `src-tauri/src/workflow/react/orchestrator.rs:510-525,720-805`：child executor 读取独立 child Agent 的 `available_tools`/`auto_approve` 并过滤子代理禁止工具；当前 MCP 若存在于 `available_tools`，因此仍混在 child Agent 普通配置。
- `src/components/setting/Agent.vue:458-495`：Skills 为可搜索 checkbox 列表；`500-560`：普通工具、MCP exposure、auto-approve 是三个下拉框，但 MCP exposure 依赖普通 availableTools；`1099-1128,1572-1625,2193-2200`：工具排序、保存归一化和 MCP exposure 清理均与 `availableTools` 耦合；`2150-2180`：child 会禁用 skills/shell，但普通工具配置仍独立存在。
- `src/stores/agent.js:16-29,84-118,145-183,220-230`：前端 Agent 字段在 snake_case JSON 字符串和 camelCase 数组之间转换，MCP 目前只有 `mcpToolExposure`。
- `src/components/workflow/WorkflowInputArea.vue:232-256,1052-1128,1260-1300`：可用工具 tab 由 `currentWorkflow.agentConfig.availableTools` 优先、selected Agent 回退；auto-approve 已过滤为工作流可用集合；切换可用工具会同步移除 auto-approve。
- `src/composables/workflow/useWorkflowCore.ts:367-404,480` 附近：继承配置包含 `availableTools`，并通过 `update_workflow_agent_config` 持久化；`workflowConstitution.test.js:290-337` 已保护当前 workflow 配置优先和 auto-approval 子集。
- `src-tauri/src/tools/tool_manager.rs:365-380,425-465`：工具元数据和调用规范统一来自 ToolManager；MCP 组合名和注册机制不需要改变。
- 现有测试：`src-tauri/src/commands/workflow.rs` 已有继承/边界同步测试；`src-tauri/src/workflow/react/engine.rs:10225` 已测试 MCP exposure 受 available_tools 授权；前端 `workflowConstitution.test.js`、`workflowUiContract.test.js`、`Agent.performance.test.js` 和 `package.json:test:workflow` 可复用。

当前缺口：clear-context 会调用同步函数，但 engine 的 `begin_manual_clear_context_segment` 本身只清理运行时状态；必须确保后续新任务段使用同步后的 AgentConfig 重建工具并向 UI 发出配置。同步函数现有合并语义保留工作流勾选的 Agent 能力交集，符合“workflow <= Agent”，但 MCP 独立后需把 MCP 集合纳入同一交集/归一化逻辑。`AgentConfigUpdated`/工作流本地状态更新路径需显式覆盖 `availableTools` 与新的 MCP 配置，避免 UI 仍显示旧值。

## 4. Recommended Solution and Architecture

### 4.1 配置模型与兼容边界

采用“复用现有 `mcp_tool_exposure` 存储列、改变其 JSON 载荷为兼容对象”的方案，避免数据库 schema migration。新增结构化 `McpToolConfig`（字段语义为 `available`, `auto_approve`, `auto_expand`），并在 Rust 边界提供兼容反序列化：旧数组读取为 `auto_expand`；归一化时从旧 `available_tools`/`auto_approve` 中提取 MCP 项补入对应 MCP 集合。保存新数据时写入对象载荷。前端以 `mcpTools`（或同等明确命名）承载对象，旧 `mcp_tool_exposure` 在 store adapter 处映射为 `autoExpand`。

普通 `available_tools`/`auto_approve` 在新 canonical config 中只存非 MCP 工具；有效运行时工具由普通集合和独立 MCP `available` 集合组成，再统一交给既有 ToolManager 注册过滤。若为了兼容旧 snapshot 暂时保留 MCP 项，必须在 `AgentConfig` normalize adapter 立即拆出，不允许在 UI 继续把它当普通工具。

### 4.2 任务边界同步

保留 `sync_workflow_agent_config_at_tool_boundary` 作为唯一边界同步入口：

1. 读取当前 workflow 与 Agent。
2. 从 Agent 构造最新 canonical config。
3. 将旧 workflow 的普通工具偏好与 MCP 三项偏好分别与 Agent 当前能力取交集；不恢复已从 Agent 删除的工具，不自动选择新安装工具之外的非用户偏好普通工具。MCP 的“可用/自动审批/自动展开”分别取交集，且 auto-approve ⊆ available、auto-expand ⊆ available MCP。
4. 应用 child Agent 的既有过滤规则到 MCP 三项配置。
5. 保留既有 Shell policy/sandbox/技能和 workflow preference 合并语义。
6. 写回 workflow snapshot；在新任务段实际开始前，使用同一个 canonical config 更新 executor 运行时字段并重建 foundation tools。
7. 通过 `AgentConfigUpdated` 携带完整 canonical config 发送给前端；前端按 session id reconcile，不用 Agent 编辑器的全局缓存取代当前 workflow authority。

创建新 workflow、完成后续发和手动 clear-context 后的下一段都调用该路径。运行中显式更新当前 workflow 配置仍走现有 signal/update command，但 payload 需使用 canonical MCP config 并在后端再次 enforce 子集关系。

### 4.3 MCP 运行时

将 engine 的 MCP 过滤从 `mcp_tool_exposure` 改为 `McpToolConfig.available` 控制授权，`auto_expand` 继续控制直接 schema/折叠；已有 `refresh_workflow_mcp_runtime_capabilities`、`register_foundation_tools`、`mcp_tool_load`、MCP disabled 检查和 ToolManager API 保持职责不变。自动审批集合统一由 canonical `auto_approve`（普通工具）与 MCP `auto_approve` 合并到 runtime approval set，但前端和持久化仍分别展示/保存。Shell `bash` 不进入 MCP auto-approve。

### 4.4 子代理

不创建新的子代理继承机制。Agent.vue 的 child 编辑表单同样展示 MCP 独立列表并保存 child 自己的三项配置；`DefaultSubAgentFactory` 将 child Agent canonical MCP config 写入 child workflow config；`filter_sub_agent_tool_ids` 扩展为 MCP 配置的逐集合过滤，继续删除现有 child 禁止工具。这样满足“子代理独立配置”决定，也保持后端权限为最终约束。

### 4.5 前端

- Agent.vue 在 Skills 区域后增加 MCP checkbox/三开关列表（可搜索、空列表提示、按现有样式和 i18n）；Tools tab 的普通下拉排除 MCP，普通 auto-approve 也排除 MCP。
- WorkflowInputArea 的当前任务工具弹层将普通工具和 MCP 工具按当前 workflow canonical config 分开或统一展示但明确使用当前任务集合；MCP 的三个状态只允许写当前 workflow config，自动审批选项来源于当前可用集合。
- `agent.js` 增加 `mcpTools` adapter；`useWorkflowCore.ts` 的继承/更新 payload 增加 MCP config；工作流事件 reconcile 更新 `agentConfig.availableTools`、`agentConfig.mcpTools`、`autoApprove` 等字段。
- 所有新用户可见文案补齐 `en.json`、`zh-Hans.json`、`zh-Hant.json`，保持 locale 结构和排序。

## 5. Decision and Uncertainty Ledger

### Confirmed decisions

- **D-1**：子代理拥有独立 MCP 配置 UI 和持久化；不继承主代理当前 MCP 配置作为唯一来源。
- **D-2**：保留旧 MCP 可用/自动审批状态；旧 `mcpToolExposure` 数组映射为自动展开。
- **D-3**：不新增数据库列，复用现有 `mcp_tool_exposure` 列并以兼容 JSON 对象保存新三项配置，避免 schema migration。
- **D-4**：MCP 工具组合名、ToolManager/MCP loader 机制和既有子代理禁用规则不变。
- **D-5**：Agent 工具配置只在规范任务边界同步到已有 workflow；活动任务段不因 Agent 编辑静默改变。

### Assumptions

- **A-1**：`get_available_tools` 在 MCP server 安装/刷新后能够提供当前可发现 MCP 工具；如果运行时工具列表刷新存在异步延迟，UI 只展示命令返回的当前列表，不自行制造工具。
- **A-2**：现有 `mcp_tool_exposure` 数据列可安全保存 JSON object；所有读取入口会经过同一兼容 adapter，不允许直接把其内容反序列化为 `Vec<String>`。
- **A-3**：当前 workflow agent config 更新命令可扩展为 canonical MCP 字段而不改变 Tauri 命令名；若代码中存在未发现的直接 JSON 字段读写，实施单元 U-1 必须先修正为 adapter。

### Open questions and blockers

无。用户已确认子代理采用独立配置，并确认旧 MCP 可用/自动审批状态保留、旧 exposure 映射为自动展开；其余细节可由现有代码约定安全确定。

## 6. Execution Map

### U-1: 建立 MCP canonical config 与兼容归一化

- **Purpose**：定义 Rust/前端共同认可的 MCP 三项配置载荷，兼容旧数组和旧混合 Agent/Workflow 数据，并保证规范化后 MCP 不再依赖普通列表。
- **Covers**：AC-2, AC-3, AC-4, INV-2, INV-3, INV-5, INV-6
- **Confirmed Targets**：`src-tauri/src/db/agent.rs` 的 `AgentConfig`/`Agent`；`src-tauri/src/commands/agent.rs` 的保存 sanitize；`src-tauri/src/commands/workflow.rs` 的 `build_agent_config_from_agent`、`merge_inherited_workflow_config`、`enforce_auto_approve_tool_visibility`；`src-tauri/src/workflow/react/orchestrator.rs` 的 child filter；`src/stores/agent.js` 的前后端 adapter。
- **Candidate Targets**：`src-tauri/src/db/config_transfer.rs` 及任何直接读取 `mcp_tool_exposure` 的兼容/导入测试；实施时通过文本搜索确认后再改。
- **Preconditions**：D-1 至 D-5；A-2。
- **Implementation Path**：新增 typed `McpToolConfig` 和单一兼容解析/normalize helper；旧数组转 `auto_expand`，从旧普通数组提取 MCP 项，过滤不存在/disabled MCP；在 AgentConfig 创建、workflow merge、保存 sanitize、snapshot 读取和 sub-agent 创建处统一调用；将 child filter 应用到三项；保持 `mcp_tool_exposure` 列名和 Tauri command 名称兼容。
- **Expected Result**：所有 Rust 运行时/持久化入口都能得到 `available`, `auto_approve`, `auto_expand` 三个明确集合，且每个子集关系可被统一验证。
- **Verification**：V-1, V-2, V-3。
- **Allowed Local Decisions**：字段内部命名、serde 自定义反序列化实现方式、去重排序方式可按现有 Rust 风格选择，但不得新增 schema migration 或改变旧配置映射。
- **Stop Conditions**：发现必须改变数据库 schema、旧数据无法区分且会导致放权、或某个外部导入格式要求保留旧字段语义时停止并请求用户确认。
- **Risks / Edge Cases**：旧 JSON malformed、disabled/卸载 MCP、空数组与 `None` 的语义、MCP 工具名大小写/组合名、重复 ID、child Agent 继承 snapshot。

### U-2: 让创建与任务边界同步成为完整运行时路径

- **Purpose**：让首次启动、完成后续发、手动清空上下文后的新任务段共享同一配置同步、运行时重建和前端通知路径。
- **Covers**：AC-4, AC-5, AC-6, AC-8, INV-1, INV-2, INV-4, INV-6
- **Confirmed Targets**：`src-tauri/src/commands/workflow.rs` 的 `workflow_start`、`sync_workflow_agent_config_at_tool_boundary`、`begin_new_context_frame`/`clear_context` 相关路径及 runtime config command；`src-tauri/src/workflow/react/engine.rs` 的 `init_internal`、`prepare_completed_resume_internal`、`begin_manual_clear_context_segment`、`refresh_runtime_config_from_snapshot`、`register_foundation_tools`、`refresh_workflow_mcp_runtime_capabilities`、runtime signal handlers；`src/composables/workflow/useWorkflowCore.ts` 的 event reconcile。
- **Candidate Targets**：`src-tauri/src/workflow/react/types.rs`/gateway dispatch wiring，如完整 `AgentConfigUpdated` payload 尚未覆盖 MCP 字段；`src/stores/workflow.js`，如当前 workflow config 更新需要补充字段。
- **Preconditions**：U-1 完成；A-1。
- **Implementation Path**：使边界同步返回 canonical config 并在 executor 任务段启动前应用；completed resume 先 sync 再 prepare；clear-context 的 cold/live 两条路径先 sync 并确保下一次 user message 使用该 config；engine 的 MCP runtime filter 改读 MCP available/auto-expand；runtime update 同时更新普通与 MCP approval 状态并 enforce 子集；发送完整 `AgentConfigUpdated`，前端按 session 更新 currentWorkflow 和 stores。
- **Expected Result**：新任务段看到的工具、MCP schema 展开状态、auto-approval 和前端显示来自同一个 canonical snapshot；活动任务段不被 Agent 编辑器刷新。
- **Verification**：V-2, V-3, V-4, V-5。
- **Allowed Local Decisions**：可复用现有 `rebuild_foundation_tools_for_runtime_update` 或增加一个薄的 canonical apply helper；不得新增第二套 lifecycle/approval 状态机。
- **Stop Conditions**：需要在运行中的非边界任务段强制刷新工具、需要修改 wait/approval 语义、或发现 clear-context 在等待态仍可能执行时停止。
- **Risks / Edge Cases**：completed grace resume、cold recovery、clear-context live/cold 两条路径、MCP server 刷新时工具消失、配置更新失败后的 rollback、前端 stale event。

### U-3: 重构 Agent.vue 的普通工具/MCP 独立配置

- **Purpose**：为 primary 和 child Agent 提供独立 MCP 三开关，并让普通工具下拉不再包含 MCP。
- **Covers**：AC-1, AC-2, AC-3, AC-8, INV-5, INV-6
- **Confirmed Targets**：`src/components/setting/Agent.vue` 的 Skills/Tools tabs、`defaultFormData`、sorted/auto-approve/MCP computed、normalize/save/load/copy watchers；`src/stores/agent.js`。
- **Candidate Targets**：`src/components/setting/Agent.performance.test.js` 或新增 focused `Agent.toolConfig.test.js`。
- **Preconditions**：U-1 完成；D-1/D-2。
- **Implementation Path**：在 Skills 下方增加 MCP 搜索和列表；每项使用 `el-switch`/checkbox 三列绑定 `agentForm.mcpTools.available/autoApprove/autoExpand`；普通 sortedAvailableTools/autoApproveOptions 过滤 MCP；保存 normalize 保证 autoApprove/autoExpand 子集关系；加载/copy 兼容旧数组并保留旧状态；child 不隐藏 MCP 列表，但仍由后端做最终权限过滤；补齐三语言 i18n 和现有 SCSS 风格。
- **Expected Result**：安装 MCP 后工具自动出现在独立列表；用户能在 primary/child Agent 中直接设置三项状态，保存再打开状态一致。
- **Verification**：V-1, V-5, V-6。
- **Allowed Local Decisions**：列表是否复用 Skills 的 checkbox 样式、搜索字段布局、开关标签宽度可按现有 dialog 尺寸选择；不得把 MCP 重新放回普通工具下拉。
- **Stop Conditions**：发现系统 Agent 的字段锁定策略无法允许新配置、或现有 locale 结构无法兼容新增键时停止。
- **Risks / Edge Cases**：无 MCP、MCP 卸载后保留旧 ID、child role 切换、系统 Agent copy、三语言 key 顺序。

### U-4: 重构 WorkflowInputArea 当前任务工具展示与编辑

- **Purpose**：让自动审批弹层展示当前任务的真实工具集合，并对 MCP 使用当前任务三项配置。
- **Covers**：AC-4, AC-7, AC-8, INV-1, INV-2, INV-3
- **Confirmed Targets**：`src/components/workflow/WorkflowInputArea.vue:207-260,1043-1130,1260-1300`；`src/composables/workflow/useWorkflowCore.ts:367-480`；`src/stores/workflow.js` 的 auto-approved/config reconcile。
- **Candidate Targets**：`src/views/Workflow.vue`，如需要传递当前任务 MCP metadata/config；`src/composables/workflow/workflowConstitution.test.js`。
- **Preconditions**：U-1/U-2；当前 workflow config 已成为 authority。
- **Implementation Path**：把当前 workflow canonical 普通/MCP 集合与 `agentStore.availableTools` 做交集；可用 tab 和 MCP 子列表不读取新的 selected Agent 配置扩大集合；auto-approve tab 只从当前可用集合生成；MCP 三开关写入 `update_workflow_agent_config`，取消可用时同步移除 MCP auto-approve/auto-expand；接收 `AgentConfigUpdated` 后更新 currentWorkflow agentConfig 和 workflow store。
- **Expected Result**：UI 显示集合满足 `MCP autoApprove ⊆ MCP available`、`autoExpand ⊆ MCP available`、普通 autoApprove ⊆ 当前可用；Agent 变更直到任务边界才体现。
- **Verification**：V-4, V-5, V-6。
- **Allowed Local Decisions**：MCP 在 tab 内单独分组或与普通列表同一 tab 分段展示可选择，但三项语义必须清楚、状态必须来自 currentWorkflow。
- **Stop Conditions**：需要通过 transcript 文本推导工具集合、或前端事件与后端 canonical config 无法建立 session 级关联时停止。
- **Risks / Edge Cases**：selected Agent 切换、workflow reload、MCP server 动态刷新、当前任务没有 MCP、旧 workflow config hydration。

### U-5: 补齐 focused tests 与静态合约

- **Purpose**：用最小 focused 验证覆盖配置兼容、边界同步、子代理和前端集合关系。
- **Covers**：AC-1, AC-2, AC-3, AC-4, AC-5, AC-6, AC-7, AC-8, AC-9, INV-1, INV-2, INV-3, INV-4, INV-5, INV-6
- **Confirmed Targets**：`src-tauri/src/commands/workflow.rs` tests、`src-tauri/src/workflow/react/engine.rs` tests、`src-tauri/src/commands/agent.rs` tests、`src/composables/workflow/workflowConstitution.test.js`、`src/composables/workflow/workflowUiContract.test.js`、`src/components/setting/Agent.performance.test.js`、`package.json` scripts。
- **Candidate Targets**：新增 `src-tauri/src/db/agent.rs` config serde tests、`src/composables/workflow/agentToolConfig.test.js`，仅当现有测试文件不适合承载新增断言时。
- **Preconditions**：U-1 至 U-4 完成；所有 canonical field 名称稳定。
- **Implementation Path**：Rust 测试覆盖旧数组转换、混合列表拆分、子集过滤、child MCP 过滤、边界 sync 后 persisted config 和 runtime MCP authorization；前端测试覆盖 MCP 不进入普通列表、三个开关和 workflow authority；执行 focused frontend workflow suite 与 `cargo test` 的相关模块/测试过滤。
- **Expected Result**：测试能证明行为和集合关系，而不是只检查代码字符串；所有 AC/INV 有对应证据。
- **Verification**：V-1 至 V-7。
- **Allowed Local Decisions**：可按现有测试组织选择新增测试文件或扩展已有 contract tests；不得只依赖编译通过作为验收。
- **Stop Conditions**：focused test 暴露现有生命周期/approval 回归，必须先修复或请求用户决定，不得通过放宽断言绕过。
- **Risks / Edge Cases**：Rust 测试耗时、MCP server 无法在单测启动、前端静态合约对模板重排敏感。

## 7. Verification Strategy and Acceptance Matrix

- **V-1**：执行 `cargo test --manifest-path src-tauri/Cargo.toml agent_config`（若无匹配测试，则执行新增/相关 `cargo test` 过滤项）以及 `cargo test --manifest-path src-tauri/Cargo.toml mcp_tool_exposure`；证据：旧数组/新对象解析、MCP 三集合子集、disabled/不存在过滤和 child 过滤测试通过。
- **V-2**：执行 `cargo test --manifest-path src-tauri/Cargo.toml task_boundary_sync` 及 `cargo test --manifest-path src-tauri/Cargo.toml inherited_workflow_config`；证据：首次/续发/clear-context 共享同步 helper，持久化 workflow config 与 Agent 能力交集符合预期。
- **V-3**：执行相关 workflow engine/commands focused tests，至少覆盖 `mcp_tool_exposure_respects_available_tools_authorization` 的更新版本、runtime config update、completed resume 和 manual clear；证据：运行时注册工具和 MCP loader/direct schema 遵从 canonical config，活动任务段不被边界外 Agent 编辑改变。
- **V-4**：检查并运行前端事件 contract：`grep`/静态断言确认 `AgentConfigUpdated` 更新 current workflow 的 available/MCP/auto-approve 字段；执行 `pnpm test:workflow`；证据：workflow UI authority 优先级、auto-approval 子集、旧消息/事件不扩大工具集合的断言通过。
- **V-5**：执行 `pnpm test:workflow`（仓库约定的 focused suite）并检查 `Agent.vue` 的新增 contract/performance tests；证据：primary/child MCP 列表、普通工具过滤、三开关持久化和 locale keys 结构断言通过。
- **V-6**：手动运行应用做最小 UI 验证：安装/刷新一个 MCP 后打开 primary 与 child Agent 编辑器，确认 MCP 自动出现于 Skills 下方；分别切换三列、保存、重开；创建 workflow，检查自动审批弹层的可用/自动审批集合；完成任务续发并 clear-context 后再次检查；证据：三类任务边界的前后端显示与实际 LLM tool list 一致。
- **V-7**：执行 `pnpm build`（若依赖/环境可用）并检查 `git diff --check`；证据：Vue 编译、i18n 引用、无空白/换行问题。若构建耗时或环境阻塞，记录未执行原因，不以编译替代行为测试。

接受矩阵：

| Requirement | Implementation | Verification |
|---|---|---|
| AC-1 | U-3, U-4 | V-4, V-5, V-6 |
| AC-2 | U-1, U-3 | V-1, V-5, V-6 |
| AC-3 | U-1, U-3 | V-1, V-2 |
| AC-4 | U-1, U-2, U-4 | V-1, V-2, V-3, V-6 |
| AC-5 | U-2 | V-2, V-3, V-6 |
| AC-6 | U-2, U-4 | V-3, V-4, V-6 |
| AC-7 | U-4 | V-4, V-6 |
| AC-8 | U-2, U-3, U-4 | V-4, V-5, V-6 |
| AC-9 | U-5 | V-1, V-2, V-3, V-4, V-5, V-7 |
| INV-1 | U-2, U-4, U-5 | V-3, V-4, V-6 |
| INV-2 | U-1, U-2, U-4, U-5 | V-1, V-2, V-3, V-4 |
| INV-3 | U-1, U-2, U-4, U-5 | V-1, V-3, V-6 |
| INV-4 | U-2, U-4, U-5 | V-2, V-3, V-6 |
| INV-5 | U-1, U-3, U-5 | V-1, V-5, V-6 |
| INV-6 | U-1, U-2, U-3, U-5 | V-1, V-2, V-5, V-7 |

## 8. Risk, Migration, and Rollback

- **配置载荷兼容风险**：复用 `mcp_tool_exposure` 列改变 JSON 形状。必须使用单一兼容 adapter；旧数组不可被任何新路径直接当对象解析。回滚时保留旧数组写出能力，或先将对象降级为 `auto_expand` 数组并说明可用/自动审批状态无法完整恢复。
- **权限放大风险**：不得把“已安装 MCP”自动写入 Agent 的可用集合；它只自动出现在 UI，用户勾选后才进入 config。旧配置迁移必须只保留原来已启用的 MCP。
- **运行时一致性风险**：边界同步只更新下一任务段；运行中显式 workflow config signal 仍需后端 enforce。失败时沿用现有 persist/rollback，不发送成功的 UI config event。
- **MCP 动态变化风险**：已卸载、disabled 或刷新后消失的 MCP 必须从 effective runtime/tool list 过滤，不能继续调用；auto-approve/auto-expand 同步清理。
- **子代理风险**：child 的独立配置不代表绕过权限。所有三项 MCP 集合需经过后端 child filter 和运行时 available authorization。
- **前端陈旧状态风险**：current workflow config 优先于 selected Agent；边界 `AgentConfigUpdated` 必须带 session id 和完整 canonical config，避免全局 Agent store 反向覆盖当前任务。
- **验证环境风险**：若没有活动 MCP server，自动化测试使用 metadata/集合 helper，手动验证补充真实安装 MCP。未能运行 `pnpm build` 或 Rust 全套测试时必须在交付报告中说明。
- **Rollback**：代码回滚前先保证配置 adapter 能读取新对象；不执行 destructive migration。需要恢复旧版本时，可通过兼容读取将对象的 `auto_expand` 写回旧 exposure 数组，但新 `available`/`auto_approve` 状态只能由旧字段或人工重建；因此发布前应保留配置导出备份路径并在测试中验证双向读取。

## 9. Handoff Checklist

- 首先检查 `src-tauri/src/db/agent.rs` 的 `AgentConfig`、`Agent` 和所有 `mcp_tool_exposure` 读写点，再检查 `src-tauri/src/commands/workflow.rs:1481` 的边界同步 helper。
- 第一实施单元执行 U-1，先做 repository freshness check：确认 `mcp_tool_exposure` 仍为所有读写入口共用列，确认没有用户未提交改动覆盖这些文件，确认 `CONSTITUTION.md` 没有更新。
- 首先运行已有的 `cargo test` 相关配置/继承测试和 `pnpm test:workflow`，建立 baseline 后再改动。
- 清点所有 `mcp_tool_exposure`、`available_tools`、`auto_approve` 的 JSON 读写，避免遗漏直接反序列化入口；检查 `db/config_transfer.rs` 是否需要只读兼容更新。
- 实施中若出现 schema migration、外部导入格式破坏、权限放大、运行中任务工具强制刷新、等待/审批语义变更或子代理规则变化，停止并请求用户确认。
- 完成前核对每一项 AC/INV 均有 U/V 覆盖；所有 U 已完成，所有 V 已执行或在报告中说明；无 `unwrap`/`expect` 新生产代码、无硬编码 UI 字符串、locale 结构一致、LF 行尾；执行 `pnpm test:workflow`、相关 `cargo test`、`pnpm build`/`git diff --check`（环境允许时）。

## 10. Plan Readiness Gate

- 所有用户目标已由 AC-1 至 AC-9 表达。
- 每个 AC 和 INV 都映射到至少一个 U 和 V，并已在 acceptance matrix 中列出。
- 目标文件和符号来自实际读取；候选目标和假设已明确标识。
- 任务边界、公共配置 payload、MCP persistence 兼容、子代理权限、前端 authority 和失败路径均有 stop condition。
- 验证覆盖配置解析、运行时工具注册、边界同步、UI 集合关系、primary/child、旧数据和 focused build/test，而非只检查代码存在。
- coding agent 可从 U-1 开始做窄 freshness check，不需要重复 broad investigation。
- `acceptance_contract` 将与本计划的 AC/INV/U/V 完全一致，`unresolved_blockers` 为空。
