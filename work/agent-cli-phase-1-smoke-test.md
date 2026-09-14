# cs CLI 第一期真实环境冒烟测试记录

- 日期：2026-09-14
- 环境：Linux，`pnpm tauri dev` 已启动（主进程 PID 3302777，控制面 `http://127.0.0.1:35735`，协议 v1）
- 分支：`feature/cli`，基线提交 `4a6e951c`（feat(workflow): add cs CLI and loopback HTTP/SSE control plane）
- 结论：**通过**。CLI → loopback HTTP/SSE 控制面 → 与 Tauri 窗口同一 workflow runtime 的全链路打通，模型覆盖生效，生命周期事件完整。

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
