# GPUI 工作流窗口交接说明

本文用于继续完善 `src-tauri/crates/gpui-observer` 这个原生工作流窗口。当前基线提交为 `d5930023 fix(gpui): keep workflow commands responsive during catalogue fetches`（功能基线 `9a1777a0`），已完成一轮对 Vue 工作流页面的功能与样式校对，但仍有一些未对齐项（见“已知差距”）。

## 1. 速览

- 目标：用 GPUI 原生窗口复刻 `src/views/Workflow.vue` 及其子组件，而不是嵌入 webview。
- 二进制：`chatspeed-gpui-observer`（`src-tauri/crates/gpui-observer`）。
- 运行：仓库根目录执行 `./run_gpui_observer.sh`；需要本机已启动 ChatSpeed runtime，并存在 discovery 文件（默认 `~/.local/share/ai.aidyou.chatspeed/runtime/control-plane-v1.json`）。
- 数据通道：只允许三类，见 `src/native.rs` 顶部注释
  - `POST /control/v1/data-commands/{command}`（allowlist）
  - `/control/v1/...` 控制面读写
  - 本进程内的宿主能力（目录列表、平台文件选择等）
- 不要为了图省事引入 Tauri command 依赖；这个窗口刻意与 Tauri shell 解耦。
- Vue 参考页面：
  - `src/components/workflow/StatusNotifier.vue`
  - `src/components/workflow/WorkflowMessageList.vue`
  - `src/components/workflow/WorkflowInputArea.vue`
  - `src/views/Workflow.vue`（状态、命令、默认值）
  - `src/composables/workflow/useWorkflowCore.ts`（配置持久化语义）

## 2. 目录与职责

```
src-tauri/crates/gpui-observer/
├── locales/observer.yml          唯一 UI 文案表（en / zh-Hans / zh-Hant）
├── src/
│   ├── main.rs                   Observer 状态、窗口渲染、交互入口、测试
│   ├── worker.rs                 Tokio 线程、runtime HTTP/SSE、Command 分发
│   ├── native.rs                 三类数据通道封装
│   ├── model.rs                  消息/工具/流式事件的本地状态
│   ├── presentation.rs           消息 → 可渲染 Row / ToolView / ChoiceGroupView
│   ├── types.rs                  Options / Command / UpdateKind / Message
│   ├── lib.rs                    i18n 初始化与 locale 回归测试
│   ├── icons.rs                  Vue 图标字体映射（含与 chatspeed.css 的一致性测试）
│   └── ui/
│       ├── status.rs             输入框上方状态栏
│       ├── composer.rs           输入框外壳 + 附件预览
│       ├── composer/footer.rs    底部选择器、+ 菜单、沙箱、发送区
│       ├── composer/panels.rs    配置弹窗（模型/Skills/MCP/工具/执行风格）
│       ├── composer/suggestions.rs @ / 斜杠建议
│       ├── approval.rs           工具审批卡
│       ├── frame.rs / titlebar.rs / sidebar.rs / icon.rs
```

## 3. 本次已完成的修改（交接要点）

### 3.1 状态栏文案

- `locales/observer.yml` 原来使用 Vue 的 `{{name}}`，而 rust-i18n 只认 `%{name}`，导致状态栏出现未解析占位符。
- 已全部改成 `%{...}`。
- `src/lib.rs` 与 `src/ui/status.rs` 均有回归测试，防止 `{{` 再次进入 locale。
- 新增 UI 文案时必须使用 `%{name}`，并且 `t!` 调用中的参数名要与 locale 一致。

### 3.2 ask_user

相关实现：

- `main.rs::ensure_ask_user_fields`：在等待回答时，为每个问题预先创建“其他”文本域实体。
- `main.rs::choice_group`：始终把文本域放在“其他”行下面；聚焦/输入时仅在尚未选择选项时才自动选中“其他”（对齐 Vue 的 `selectAskUserOtherIfUnset`）。
- `main.rs::submit_ask_user`：选项 + 补充文本会一起写入答案；提交后写入 `ask_user_answered`，立即切换为只读已答卡片。
- `main.rs::ask_user_projected_answer`：运行时的答案已投影但状态尚未切换时，也按已答处理。
- `ui/status.rs::live_state`：本地已答后不再显示“等待用户回复”，也不再回退到该 ask_user 的 pending 工具状态。
- `presentation.rs::ask_user_response`：选择具体选项时也会把补充文本追加到 `choice`（Vue 同样行为）。
- `main.rs::tool_row`：ask_user 卡片始终展开；pending 的 ask_user 不会被误渲染成审批卡。

注意：

- 不要把 ask_user 重新放进 `approval_card` 分支。
- `ask_user_answered` 是乐观状态，当前只在切换会话/新建会话时清空。

### 3.3 模型切换

- 根因之一：ChatSpeed 代理模型使用 `id = 0` + `model = "group@alias"`，旧的 `active_provider()` 找不到 provider，选择器直接不弹窗。
- `footer.rs::model_choices` 现在支持：
  - 代理别名：从 `get_all_config` 的 `chat_completion_proxy` 读取 `group -> alias -> [{id, model}]`。
  - 普通 provider 模型。
  - 代理表尚未加载或 provider 目录过期时，至少保留“当前模型”可点击兜底。
- `main.rs::model_label` 按 Vue 显示 `alias(group)`，规划模式下加 `Plan/` 前缀。
- `active_model_config` 在规划模式下优先 `plan`，否则 `act`。
- `footer.rs::model_selector` 写入的槽位是当前模式对应的 `act`/`plan`，不是固定 `act`。

### 3.4 “+” 菜单

`footer.rs::quick_actions` 现在包含：

1. 附件
2. 压缩上下文
3. 配置区
   - 模型配置
   - Skills 配置
   - MCP 配置
   - 工具配置
   - 执行风格
4. 运行时
   - 规划模式
   - 自动批准计划（仅规划模式显示）
   - 最终审查
   - 自动压缩
5. 审批级别
   - 默认 / 智能 / 完全放行

对齐细节：

- `+` 角标显示运行时选项数量（规划、自动批准、终审、自动压缩）。
- 选中项使用主色图标/勾选；“完全放行”选中时图标、文字、勾选为红色，未选中时保持普通文字色。
- 自动压缩默认值已从 `true` 改为 `false`，与 Vue 的 `agentConfig.autoCompress ?? false` 一致。

配置弹窗：

- `ui/composer/panels.rs::overlay` 是统一的居中弹层，对应用户在 Vue 中看到的 dialog/popover。
- 模型配置：左侧槽位（act/plan/vision/utility/lite），右侧模型列表。
- Skills 配置：启用开关 + 技能多选，立即通过 `update_workflow_skills_config` 持久化。
- MCP 配置 / 工具配置：可用、自动审批、展开参数三组开关。
- 执行风格：`preset:*` 列表 + agent 自定义风格。

注意：`McpToolConfig` 嵌在 camelCase 的 `AgentConfig` 里，但自身字段是 snake_case；读写 `mcpTools` 时必须使用 `auto_approve` / `auto_expand`，读取时可兼容 camelCase。

### 3.5 沙箱配置

- `footer.rs::sandbox_selector` 在“自动 / 仅沙箱”下增加沙箱 Profile 列表、刷新按钮和空态。
- 点击模式或 Profile 后面板保持打开，并通过 `cx.notify()` 让内容重建，从而立即显示配置区。
- `main.rs` 新增乐观字段 `sandbox_mode_draft` / `sandbox_scheme_draft`，让面板与触发器在命令落地前就显示新选择；切换会话时清空。

### 3.6 附件

- `footer.rs` 的附件项调用 `Observer::pick_attachments`，使用 GPUI 自带的 `prompt_for_paths`，未引入新依赖。
- `ui/composer.rs::attachment_row` 显示缩略图、文件名和移除按钮。
- 发送时 `send_message` 会传入：
  - `attached_context`：图片文件路径说明
  - `metadata.attachments`：`{type, name, path}`
- 无会话时 `StartNew` 也携带这两个字段；worker 会写入 `initial_attached_context` / `initial_metadata`。

**重要限制**：原生窗口目前没有图片识别能力。Vue 会先调用视觉模型生成 `<img_detail>`，再把结果作为 `attached_context`；独立 GPUI 窗口没有对应命令/路由，因此现在只传文件引用和元数据，模型无法直接“看到”图片。若后续要补齐，优先在 runtime 侧增加受控的图片分析命令，不要在 observer 里直连 provider。

## 4. 命令与数据契约

新增/使用的 `Command`（`types.rs`）：

- `SetPlanning { session, enabled }` → `update_workflow_phase`（`planning` / `standard`）
- `SetPersonality { session, personality }` → `update_workflow_personality`
- `SetSkills { session, enabled, selected }` → `update_workflow_skills_config`
- `SetAgentConfig { session, config }` → `update_workflow_agent_config`（整份 agent config 字符串）
- 既有：`SetModelConfig`、`SetSandbox`、`SetApprovalLevel`、`SetAutoCompress`、`SetFinalAudit`、`SetAgent`

`Command::Tools` / `UpdateKind::Tools`：按需获取 data-command `get_available_tools`，供 MCP / 工具配置面板使用。

注意：`Command::Catalogue` 整个目录拉取已经放到独立 `tokio::spawn` 中；`get_available_tools` 只在打开工具/MCP 面板时按需请求，并带 8 秒超时。不要在 worker 命令循环里直接 await 目录类请求，否则慢请求会把 `Select`、`StartNew` 一起堵住（这正是 `9a1777a0` 引入的回归）。

新增 UI 状态（`Observer`）：

- `proxy_targets`：`get_all_config.chat_completion_proxy`
- `planning_mode`
- `sandbox_mode_draft` / `sandbox_scheme_draft`
- `ask_user_answered`
- `quick_panel` / `model_config_slot`
- `skills_draft` / `tool_draft`
- `attachments`

## 5. 常用命令

```bash
cd src-tauri

# 编译
cargo check -p chatspeed-gpui-observer

# 单元 + 集成测试（当前应全绿）
cargo test -p chatspeed-gpui-observer --tests

# 格式
cargo fmt -p chatspeed-gpui-observer
cargo fmt -p chatspeed-gpui-observer -- --check

# Clippy（titlebar.rs 有 4 条既有告警，与本次改动无关）
cargo clippy -p chatspeed-gpui-observer --all-targets --message-format=short
```

运行窗口：

```bash
./run_gpui_observer.sh
```

当前开发环境没有 `DISPLAY` / `WAYLAND_DISPLAY`，无法实际启动窗口做视觉验收；涉及布局的改动需要在有桌面的环境手动看一遍。

## 6. 已知差距 / 下一步建议

按优先级排列：

1. **图片视觉预分析**：当前只传路径。建议在 runtime 增加受控命令（例如 `analyze_image_attachments`），由 runtime 调用现有 `ChatState`，observer 只传图片数据并拿回文本。
2. **模型推理强度**：Vue 在模型选择器里为支持 thinking 的选中模型提供 low/medium/high/max 预算按钮（`THINKING_LEVEL_TO_BUDGET`）。GPUI 目前没有这组按钮。
3. **MCP 实时状态**：Vue 的 MCP 面板会显示 running/starting/disabled 等状态和 `effective` 逻辑；当前只渲染配置开关。
4. **工具配置完整性**：Vue 的“工具配置”还包含 shell 命令策略、自动批准工具等；当前只覆盖 available/autoApprove。
5. **可切换状态**：Vue 在 live session 或已运行工作流中会禁用规划模式、终审等开关；GPUI 目前仍可点击（后端会校验，但 UI 未灰显）。
6. **“+” 菜单保持展开**：Vue 的 dropdown `hide-on-click=false`，运行时开关切换后不关闭；GPUI 目前切换后关闭。若要改成保持展开，可参考沙箱面板的做法：不 dismiss、更新状态后 `cx.notify()`，Popover 内容会在父视图重渲染时重建。
7. **全新窗口的 ask_user 状态栏**：如果窗口在“答案已持久化但 runtime 仍处于 waiting”时重开，`ask_user_answered` 是空的，状态栏可能短暂显示“等待用户回复”；卡片本身已按投影答案只读渲染。可把投影答案也纳入状态栏判断。
8. **附件与 Vue 互通**：`metadata.attachments` 目前没有 `url` / `sourceUrl`，在 Vue 侧渲染同一会话时可能没有缩略图。若需要，按 Vue 的 `image_preview` / `image_source_url` 语义补 URL，但要注意 webview 对 `file://` 的限制。
9. **视觉核对**：新增的配置弹层、附件条、+ 菜单高度（已加 `max_h` + 滚动）需要在真实窗口里检查暗色/亮色、窄窗口和长列表。

## 7. 修改指南

### 新增一个命令

1. `types.rs` 增加 `Command` 变体。
2. `worker.rs` 增加分支，调用 `native::workflow_command` / `control_post` / `data_command`。
3. UI 通过 `this.worker.commands.try_send(...)` 发送。
4. 需要立即反馈的，先在 `Observer` 里写乐观字段并 `cx.notify()`。

### 新增一个配置面板

1. `main.rs::QuickPanel` 增加变体。
2. `open_quick_panel` 初始化对应 draft / 默认槽位。
3. `panels.rs::overlay` 的标题与 body match 增加分支。
4. `footer.rs::quick_actions` 增加一行菜单项。
5. 状态字段放在 `Observer`，读取 `agent_config()`，写入走 `SetAgentConfig` / 专用命令。

### 新增文案

- 只能在 `locales/observer.yml` 添加，三种语言都补齐。
- 占位符用 `%{name}`。
- 尽量保持 key 按字母序，避免和 Vue locale 的 `{name}` 语法混淆。

### 容易踩的坑

- `Observer` 定义在 crate root，子模块可以访问其私有字段；`panels.rs` 里也直接实现了 `impl Observer`，这是现有风格。
- `Popover` 的 content 是每帧由父视图重新提供的；父视图 `cx.notify()` 后内容会按新状态重建（沙箱面板依赖这一点）。
- `ask_user` 卡片永远不要走审批卡分支；已有 `tool.choices.is_empty()` 与名称判断。
- 代理模型 `id = 0` 的判断不能简单用 `provider id` 查找。
- `McpToolConfig` 嵌套字段是 snake_case。
- 所有新增的 `let _ = entity.update(...)` 如果闭包返回 `()`，不要写 `let _ =`，会触发 Clippy。
- worker 命令循环里不要直接 await 可能很慢的目录/MCP 请求；用 `tokio::spawn` + `tokio::time::timeout`，否则会阻塞所有工作流命令，表现为“会话点开没记录、创建工作流没反应”。
