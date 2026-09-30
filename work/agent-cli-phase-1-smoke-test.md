# cs CLI 第一期真实环境冒烟测试记录

- 日期：2026-09-14（含同日 MVP 补测）
- 环境：Linux，`pnpm tauri dev` 已启动（主进程 PID 3302777，控制面 `http://127.0.0.1:35735`，协议 v1）
- 分支：`feature/cli`，基线提交 `4a6e951c` + `14b68b4b`
- 结论：**通过（附 1 个既有 runtime 边界问题）**。CLI → loopback HTTP/SSE 控制面 → 与 Tauri 窗口同一 workflow runtime 的全链路打通；断开不终止、计划模式等待/恢复、跨端同权威列表均实测通过；发现"对已完成会话执行 stop 会卡在 stopping"的既有 runtime 边界问题（Tauri 同受影响，见下）。

## MVP 补测（同日，覆盖 V-7 要求的 signal/stop/reconnect 交叉项）

以下会话均使用 `--model cs@free:ds-v4-flash`（free 分组，控制成本）。

### T1 — 断开不终止（reconnect 语义）

```bash
cargo run --bin cs -- workflow run --agent builtin:coding --model "cs@free:ds-v4-flash" \
  --prompt "请写一段约150字的关于软件回归测试重要性的短文……"   # 不加 --follow
# 输出: Workflow started: 0rkyq74ag0400，CLI 随即退出
cargo run --bin cs -- workflow get 0rkyq74ag0400 --output json
```

实测：CLI 退出后 workflow 独自运行至 `completed`——**断开不终止**（AC-6/INV-2）。

### T1b — active 态 stop（发现既有问题）

```bash
cargo run --bin cs -- workflow run --agent builtin:coding --model "cs@free:ds-v4-flash" \
  --prompt "请依次完成以下三件事，每件都写200字以上……"        # 长任务
cargo run --bin cs -- workflow get 0rkyqdqh40400 --output json   # state: thinking（active）
cargo run --bin cs -- workflow stop 0rkyqdqh40400                # 返回 stopped: true
```

实测：stop 命令本身成功，但该任务在 stop 处理期间已自然完成（free 模型速度快，事件 02:35:41 `workflow_completed`），stop 后到达，会话状态被置为 `stopping` 且**永久卡住**（快照 `stopping`，无 executor 处理 stop 信号）。

**根因（既有 runtime 行为，非控制面引入）**：`workflow_stop_core`（`src/commands/workflow.rs`，Tauri `workflow_stop` 与 HTTP stop 共用）不校验会话是否已处于终态——对 completed/failed/cancelled 会话仍注入 stop 信号并把状态写为 `Stopping`，此后无 executor 消费该信号，状态无法收敛。Tauri 窗口内对已完成会话触发 stop 理论上同样复现。

**修复（已实施，提交见 git log）**：`workflow_stop_core` 入口新增终态守卫 `is_terminal_workflow_status`（`completed`/`error`/`cancelled` 直接返回成功，不注入信号、不改状态），并附单测 `terminal_workflow_statuses_skip_stop`（同时断言所有非终态 stop 仍可用，符合 CONSTITUTION.md §5.3）。守卫带结构化日志（§13）。**真机复验待办**：dev 应用重启后执行 `cs workflow stop <已完成session>`，确认状态保持 `completed` 不再变为 `stopping`。既有卡住的 `0rkyqdqh40400` 记录保留原状。

### T2 — 审批回路（协议回路已验证，命令执行层待查）

**重要根因修复（提交见 git log）**：早期 CLI 会话 `availableTools` 全为空的根因不是模型行为——CLI `--model` 合成的最小继承配置经 `AgentConfig::from_json` 规范化后 `availableTools` 变为显式空列表，`merge_inherited_workflow_config` 交集后清空了 agent 的全部内置工具（界面表现为所有工具未勾选）。修复：`validated_inherited_agent_config` 对缺失 `availableTools` 键的继承配置恢复"无偏好"语义；内置工具能力范围回归 agent 配置，skills/MCP 仍走用户安装+偏好。附回归测试 `partial_inherited_config_keeps_agent_tool_capabilities`。

**修复后真机验证（会话 0rkzdyhkr0400，python 审批路径）**：
- 工具集恢复 9 个（含 bash）✅
- bash 执行 `python3 -c 'print(123)'` 触发 `awaiting_approval`（`approval_kind: shell_command`）✅ —— python 解释器命令确需审批，符合预期
- `cs workflow approve <session> --tool-call-id <id>` → `approval_resolved(approved)` → `tool_started` ✅ 审批协议回路端到端打通
- **遗留**：批准后 python 命令执行挂起（`tool_started` 后 4 分钟无 `tool_completed`，会话回到 `awaiting_approval` 但无新 `approval_requested` 事件）。该环节位于 shell/sandbox 执行层（会话 `sandboxExecutionMode: auto`），非 CLI 控制面问题；待排查 microsandbox 环境或 auto 模式的二次确认行为。

**挂起根因（用户定位）**：msb 沙箱缺少 python 配置的镜像，导致沙箱执行无限等待。用户已将沙箱重新配置为 docker + 基础镜像。

**修复后复验（会话 0rkzz7q3g0400）**：
- 审批回路完整闭环：`approval_requested` → CLI approve → `approval_resolved` → `tool_started` → 1 秒内 `tool_failed`（快速失败，不再挂起）→ 模型自动重试并再次请求审批 ✅
- 挂起期间引擎阻塞导致 `workflow get/events` 超时的现象随根因消除
- **新残留（沙箱配置，非 CLI 问题）**：docker 模式下报 `Denied (ProfileUnavailable)`——scheme 中无匹配 python3 命令的 profile 且未启用通用（catch-all）profile。需在沙箱 scheme 配置中为 python3 添加 command_patterns 或启用通用 profile。重试会话已 stop 清理。

### T3 — 计划模式等待/恢复（signal 回路）

```bash
cargo run --bin cs -- workflow run --agent builtin:coding --model "cs@free:ds-v4-flash" \
  --plan --prompt "任务：写一句关于自动化测试的口号。请先给出简短计划等待确认。"
cargo run --bin cs -- workflow get 0rkysga2m0400 --output json
# state: awaiting_user, wait_reason: user_input   ← 计划产出后等待确认
cargo run --bin cs -- workflow message 0rkysga2m0400 --text "计划确认，请按计划执行并完成任务。"
# Signal delivered
cargo run --bin cs -- workflow get 0rkysga2m0400 --output json
# state: thinking → completed                     ← 恢复并完成
```

实测：`--plan` 生效，等待态与恢复回路（UserMessage signal → awaiting_user → thinking → completed）全链路通过（AC-5）。

### T4 — 跨端同权威（UI/CLI 交叉）

```bash
cargo run --bin cs -- workflow list --output json
```

实测：本次全部 CLI 创建的会话（含 `stopping` 卡住的那个）与既有会话同列于 23 条 workflow 记录中，同一 `MainStore`/`WorkflowManager` 权威，无第二数据源（AC-4/INV-2）。UI 窗口可见性请在本机 Workflow 窗口目视复核（这些会话应直接出现在列表中，且可从窗口继续操作）。

## 前置条件

1. `pnpm install` 已执行；`pnpm tauri dev` 正在运行（主进程启动时会在 `~/.chatspeed/runtime/control-plane-v1.json` 发布 discovery 文档）。
2. CLI 二进制构建：
   ```bash
   cd src-tauri && cargo build --bin cs
   # 或直接用 cargo run --bin cs -- <args>（推荐，避免沙箱/宿主 glibc 差异问题）
   ```

## 测试步骤与结果

### 1. 连通性：`cs doctor`

```bash
cargo run --bin cs -- doctor
```

预期输出（实测）：

```
Connected to ChatSpeed control plane: endpoint http://127.0.0.1:35735, protocol v1, instance 86399de..., pid 3302777
Connectivity, authentication and protocol version are OK
```

- 验证点：discovery 发现、Bearer token 认证、协议 major 版本协商全部通过。

### 2. Agent 列表：`cs agent list`

```bash
cargo run --bin cs -- agent list --output json
```

- 实测返回 5 个内置 Agent（builtin:coding / code-explorer / final-code-reviewer / marketing / zhugeliang），含完整 `models` 配置。
- 注意：子 Agent（如 builtin:code-explorer）不能作为顶层 workflow agent，创建时会返回 400 `Child agents cannot be used as top-level workflow agents`（实测确认）。

### 3. 端到端运行：`cs workflow run`（含模型覆盖）

```bash
cargo run --bin cs -- workflow run \
  --agent builtin:coding \
  --model "cs@free:ds-v4-flash" \
  --prompt "这是一次 CLI 冒烟测试。请直接回复一行文本：SMOKE-OK，不要调用任何工具。" \
  --follow
```

实测结果：

- 创建 + 启动成功，session id `0rkyk7x4g0400`。
- `--follow` 通过 SSE 实时流式输出事件（reasoning_chunk / message / context_usage / state / tool_completed），sequence 单调递增。
- 模型覆盖生效：会话 `agent_config.models.act` 为 `{"id":0,"model":"cs@free:ds-v4-flash"}`（provider id 0 = `cs` 代理分组），token 计量正常（prompt 18987 / completion 132）。
- Agent 按指令回复 `SMOKE-OK` 并调用 `complete_workflow` 结束。

### 4. 终态验证：`cs workflow get`

```bash
cargo run --bin cs -- workflow get 0rkyk7x4g0400 --output json
```

实测：`workflow.status = "completed"`；`messages` 共 5 条，其中恰好 1 条初始 user 消息（与 Tauri create→start 序列同构，AC-4）。

### 5. 事件回放：`cs workflow events`

```bash
cargo run --bin cs -- workflow events 0rkyk7x4g0400 --output json
```

实测：10 条持久化事件，完整生命周期：`workflow_started` → `effective_task_objective_changed` → `state_changed`×5 → `tool_completed` → `task_completed` → `workflow_completed`；durable id 单调（257156 → 257165），支持 `--after` 增量拉取。

### 6. 错误路径（附带实测）

- `cs workflow run --agent free:ds-v4-flash ...`（把模型引用当 agent id）→ HTTP 404 envelope：`Agent free:ds-v4-flash not found (not_found)`，无副作用。
- `--model` 传对象而非字符串的旧实现 → 400 `invalid_input`（已修复为 JSON 字符串，见下）。

## 本次测试中发现并修复的问题

1. **CLI 缺少会话级模型/终审/计划模式参数**（parity 缺口）：Tauri 前端创建会话时可设置模型（`inheritedAgentConfig`）、终审（`finalAudit`），启动时可设计划模式（`planningMode`），但第一期 CLI 未暴露。已补齐：
   - `cs workflow create|run --model GROUP@MODEL`（构造 act 模型覆盖的 inheritedAgentConfig；provider id 0 = `cs` 代理分组）
   - `cs workflow create|run --agent-config <JSON>`（原始 camelCase AgentConfig 字符串，与 Tauri 契约一致，二者互斥）
   - `cs workflow create|run --final-audit`
   - `cs workflow start|run --plan`
   - 服务端无需改动：HTTP create handler 直接反序列化应用层 `WorkflowCreateRequest`，本就含 `inherited_agent_config`/`final_audit`/`auto_approve_plan`。
2. `inherited_agent_config` 是 **JSON 字符串**（前端 `JSON.stringify` 后的文档），不是嵌套对象——首次实现误发对象导致 400，已修复。

## 回归验证（修改后全部复跑）

- `cargo check --bin cs --bin chatspeed`：0 error 0 warning；`cargo fmt --check` 通过。
- `cargo test --bin cs` 14 通过；`workflow::react::client::http` 18 通过；`commands::workflow` 64 通过。

## 复现注意事项

- 直接执行 `target/debug/cs` 可能因沙箱/宿主 glibc 版本差异失败（`GLIBC_2.38 not found`），用 `cargo run --bin cs -- ...` 在宿主侧执行即可。
- `--model` 的 provider id 固定为 0（`cs` 分组）；其他分组请用 `--agent-config` 传完整 ModelConfig（含正确 `id`）。
- `--plan` 仅在 start 时传 `planning_mode: true`；计划模式下前端实际使用 `plan` 模型，如需覆盖 plan 模型请用 `--agent-config`。
- 评估/自改进（题库、judge、candidate/campaign 等）属第二期，不在本测试范围。
