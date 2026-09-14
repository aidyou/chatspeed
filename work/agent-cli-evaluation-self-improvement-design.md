# ChatSpeed Agent CLI、评测与受控自我改进方案

> 状态：可行性研究完成，待分阶段设计与实施  
> 日期：2026-09-13  
> 目标：先建立统一 CLI 控制面，使模型与人工都能运行 ChatSpeed Agent、管理扩展、执行可靠实验，并在严格隔离和晋级门禁下改进 Agent。

## 1. 结论

建议实现一个 `chatspeed` CLI，但它不是桌面端旁边的简化 Agent，也不只是 benchmark launcher。它应成为 ChatSpeed 的无界面控制面，包含三组职责：

1. **运行面**：启动、交互、停止和导出真实 ChatSpeed workflow；
2. **管理面**：管理 Agent、HarnessProfile、skill、MCP、配置和环境诊断；
3. **实验面**：连接外部 benchmark harness，记录轨迹，比较候选，并执行受控自我改进 campaign。

核心原则：

- CLI 是 ChatSpeed 的正式客户端，通过主进程的版本化 loopback HTTP/JSON + SSE 控制面操作 workflow 和配置；
- 主进程独占 `MainStore`、ConfigCache、`ToolManager`、MCP 生命周期和 `WorkflowManager`，CLI 不直接打开产品数据库；
- 现有桌面前端继续使用 Tauri invoke/listen；Tauri adapter 与 Web/CLI adapter 调用同一个 Rust application service 和同一个 `WorkflowExecutor`；
- 不复制 `commands/workflow.rs` 的状态机，不建立第二个 agent loop；
- 数据库和结构化 workflow events 继续是 ChatSpeed workflow 权威；
- benchmark verdict 由独立 verifier 产生，模型输出和模型自报日志不是权威；
- skill 只编排 CLI，权限、校验、事务和审批必须由 CLI/service 代码强制；
- 候选只能修改 allowlist 中的实验 surface，不能接触生产数据域、评测器、晋级规则、secret 或 sandbox 信任核心；
- 第一阶段做人工单变量 A/B，稳定后才做 GEPA 式 prompt 优化，DGM 式开放 archive 更晚；
- 不在当前阶段开放代码级自我修改。

相关的极简 Agent 方案见 `work/minimal-coding-agent-design.md`。

## 2. 为什么先做 CLI 是正确顺序

没有 CLI 时，每次 prompt 或工具裁剪实验都要依赖桌面交互，难以保证：

- 输入、模型、prompt、工具 schema 和项目状态一致；
- 批量运行与超时取消；
- 完整轨迹、usage、成本和环境指纹可追溯；
- 结果可由自动测试判分；
- 候选配置不会污染日常使用环境；
- 同一个候选可以在多个 benchmark 和模型上重复验证。

CLI 还可以把现有散落在前端和 skill 脚本中的辅助功能收束为稳定接口。未来 Agent 只需要加载一个很小的内置 `chatspeed-cli` skill，就可以发现并调用 `skill`、`mcp`、`agent`、`config`、`doctor`、`eval` 等能力；无需在 system prompt 里常驻全部说明，也无需由多个 prompt 型 skill 直接修改配置文件。

## 3. 已确认的可复用基础

### 3.1 Workflow runtime 已具备 transport 抽象缝，但当前没有 Web workflow API

- `src-tauri/Cargo.toml` 的 library crate 包含 `rlib`，可以被新增 Rust binary 复用；
- 当前仅有桌面入口 `src-tauri/src/main.rs`，没有 headless binary；
- `src/libs/tauri.js` 的 `invokeWrapper` 直接调用 `@tauri-apps/api/core.invoke`；现有 Vue “Web 前端”实际使用 Tauri IPC，不是浏览器 HTTP API；
- workflow commands 只注册在 `src-tauri/src/lib.rs` 的 `tauri::generate_handler!`；
- `src-tauri/src/http/server.rs` 仅提供静态资源和 `/save/png`，ccproxy 仅提供 OpenAI/Anthropic/Gemini/Ollama 兼容的模型 API；二者都不是 workflow 控制面；
- `src-tauri/src/workflow/react/gateway.rs:12-38` 已定义纯 Rust `Gateway` trait，但生产实现只有 `TauriGateway`；
- `WorkflowExecutor` 接受 `Arc<dyn Gateway>`、`MainStore`、`ChatState`、`ToolManager`、`SubAgentFactory` 等显式依赖，本身不直接持有 `AppHandle`；
- `GatewayPayload` 已包含 message、state、approval、tool、task-completed、compression、usage 等结构化实时事件；DB 中的 `WorkflowEventRecord.id` 可作为 durable replay cursor，但当前 `get_workflow_events` 只支持全量读取。

因此 CLI 可以成为新的正式客户端，但不能“直接按现有 Web 接口调用”，因为当前没有该接口。正确做法是：抽出 Tauri-free application service 与唯一 runtime hub/event broker，在主进程新增 loopback HTTP/JSON command adapter 和 SSE event adapter；保留现有 Tauri adapter。

只替换 Gateway 还不够。`commands/workflow.rs` 当前承担 workflow 创建、snapshot 合并、恢复、manager 注册、policy、run loop 和 cleanup；`TauriGateway` 又同时持有 session input registry 和 UI event batching。必须先把 application orchestration、输入路由和事件 fan-out 从具体 Tauri transport 中解耦，不能简单并列创建两个各自持有 session channel 的 Gateway。

### 3.2 管理功能已有底层能力，但没有统一 service

- MCP DB CRUD：`src-tauri/src/db/mcp.rs`；
- MCP 进程和工具注册：`src-tauri/src/tools/tool_manager.rs`；
- MCP command 当前混合 DB/cache 更新与 live runtime 启停：`src-tauri/src/commands/mcp.rs`；
- Agent 持久化前的清洗、角色约束和 sandbox 校验目前位于 `src-tauri/src/commands/agent.rs`，CLI 不能绕过它直接写 DB；
- 配置导入导出底层已较纯：`src-tauri/src/db/config_transfer.rs`，Tauri command 主要增加 event；
- `SkillScanner` 只负责扫描和解析，见 `src-tauri/src/workflow/react/skills.rs`，没有 Rust 安装、更新、卸载或 lock API；
- 当前 `skill-installer` 使用 Python 脚本直接写 `~/.chatspeed/skills`，`skill-vetter` 只是 prompt 规则，不能形成程序化安全门禁。

这些能力适合抽为共享 Rust services，再由 Tauri 与 CLI 共同调用。

## 4. 推荐总体架构

```text
┌─────────────────────┐       ┌──────────────────────┐
│ Desktop/Tauri UI    │       │ chatspeed CLI        │
│ invoke/listen       │       │ HTTP/SSE client      │
│ adapter             │       │ + JSONL renderer     │
└─────────┬───────────┘       └──────────┬───────────┘
          │ Tauri IPC                   │ loopback HTTP/JSON + SSE
          └──────────────┬───────────────┘
                         ▼
┌────────────────────────────────────────────────────┐
│ ChatSpeed main process: sole runtime/data owner    │
│ Tauri adapter │ /control/v1 API │ SSE subscriptions│
├────────────────────────────────────────────────────┤
│ Shared WorkflowApplicationService                  │
│ RunConfigResolver / ExperimentService              │
│ RuntimeHub / EventBroker / idempotency / revision  │
│ AgentService / SkillPackageService / McpService    │
├────────────────────────────────────────────────────┤
│ Existing runtime                                   │
│ WorkflowExecutor / WorkflowManager / ToolManager   │
│ MainStore / ccproxy / approval / context / sandbox │
└────────────────────────────────────────────────────┘
```

### 4.1 首期选择：主进程唯一 owner + loopback Web 控制面

CLI 的确可以视为多客户端中的一个，但复用的是**同一 domain protocol 与 application service**，不是当前 Tauri invoke 的传输实现。首期推荐：

- ChatSpeed 主进程是 `MainStore`、ConfigCache、`ToolManager`、ccproxy、MCP child processes 和 `WorkflowManager` 的唯一 owner；
- mutation/query 使用版本化 loopback HTTP/JSON，例如 `/control/v1/...`；
- workflow 实时事件使用 SSE。当前主要是 server → client 单向事件，控制动作适合普通 HTTP，SSE 又有 `Last-Event-ID`，因此第一版不需要 WebSocket；
- CLI 只做 discovery、认证、typed HTTP request、SSE 订阅、输出和退出码映射；
- 现有 Vue/Tauri 前端第一版不迁移到 HTTP，继续使用 invoke/listen，避免把 CORS、认证和重连风险同时带入产品 UI；
- Tauri handlers 与 HTTP handlers 都必须是薄 adapter，调用同一个 `WorkflowApplicationService`；
- 如果主进程未运行，CLI 可按明确策略启动一个无窗口主进程再连接，CLI 自身仍不构造 `MainStore`。

不应把 control plane 挂到现有 permissive HTTP router 或 ccproxy 上。现有 static server 使用 `CorsLayer::allow_origin(Any)`，ccproxy 具有不同的 LLM proxy 认证语义；workflow 管理接口必须是独立 router、独立 middleware 和独立日志脱敏边界。

### 4.2 HTTP/SSE 协议最小契约

建议 v1 提供：

```text
GET    /control/v1/meta
POST   /control/v1/conversations
GET    /control/v1/conversations/{id}
PATCH  /control/v1/conversations/{id}/config
POST   /control/v1/conversations/{id}:freeze
POST   /control/v1/conversations/{id}:start
POST   /control/v1/conversations/{id}:signal
POST   /control/v1/conversations/{id}:stop
GET    /control/v1/conversations/{id}/events?after=<cursor>
POST   /control/v1/experiments:resolve
POST   /control/v1/experiments:run
```

协议要求：

- wire DTO 使用 canonical `snake_case`；camelCase 仅保留在 Vue/Tauri boundary；
- 每个 schema/envelope 带 major protocol version、request ID、server instance ID；
- create/start/signal/stop 与 experiment run 使用 `Idempotency-Key`；同 key 同 body 返回原结果，同 key 不同 body 返回 conflict；
- config patch 使用原子 revision/CAS；freeze 返回 resolved config、revision、config/prompt/tool-schema hashes；start 校验 expected revision/hash；
- SSE event 带 session ID、stream cursor、可用时的 durable event ID、event kind 和 typed payload；
- 每个 subscriber 使用有界队列。慢客户端不得反压 engine；chunk 可合并，state/approval/terminal 不得静默丢失；溢出时断开并返回 `reset_required`；
- 重连优先从短期 ring buffer 按 `Last-Event-ID` 续传；游标过期或主进程重启后，客户端拉 authoritative snapshot + durable events，再重新订阅；不承诺逐 token chunk 永久精确 replay；
- control plane 只绑定 loopback，不监听 LAN；启动生成 256-bit bearer token，写入 current-user-only discovery file，token 只进 `Authorization` header，不进 URL、日志或 artifact；
- CORS 默认不允许任何浏览器 origin，不使用 cookie。未来浏览器客户端需显式 origin allowlist；
- discovery 使用 OS 分配端口而非扫描固定端口，并包含 pid、server instance、endpoint、protocol version 和 token。Unix 文件权限为 `0600`；Windows 使用当前用户 ACL。

由于 loopback HTTP 在 Windows、macOS、Linux 和 container 内行为一致，它比“Unix socket + Windows named pipe”更适合正式 v1。若企业策略明确禁止 loopback TCP，才在同一 DTO/service/broker 之上增加 OS-local IPC adapter；不能为其定义第二套业务协议。

### 4.3 主进程、无窗口模式与未来 daemon

第一版可以让同一个 core bootstrap 支持 GUI/Tauri mode 和无窗口 control-plane mode。需要注意：当前只有 Tauri desktop main，尚不能把“启动 HTTP listener”等同于“已经 headless”；必须先解耦窗口绑定的 runtime 装配。

仅在出现以下需求时，再把 owner 提取为独立 `chatspeedd`：

- 需要操作系统级自启动和长期后台运行；
- 桌面升级/退出时 workflow 或 improve campaign 仍需继续；
- 多个客户端需要稳定的跨 GUI 生命周期订阅；
- MCP 与 ccproxy 需要脱离桌面包独立部署。

无论是否拆 daemon，GUI、CLI 和未来浏览器客户端都只能通过相同 application services 操作有状态资源；不能保留 CLI 直开数据库的兼容路径。

## 5. CLI 命令面

CLI 默认连接当前 ChatSpeed 主进程和用户当前数据域。实验不是用一条 `run` 命令临时拼参数，而是先创建一个 workflow conversation，再配置并冻结该 conversation，最后启动。用户在前端维护 Agent 默认配置；CLI 只对本次 conversation 创建显式 override，不写回 Agent 默认值。

```text
chatspeed <command> [--output human|json|jsonl]

conversation
  create             --agent <id> [--workspace <path>]
  config show        <conversation-id> [--resolved]
  config patch       <conversation-id> --file <agent-config.json>
  config reset       <conversation-id> --from-agent
  tools set|add|remove       <conversation-id> ...
  mcp set                     <conversation-id> --file <mcp-tools.json>
  skills set                  <conversation-id> --enabled <bool> --names <list>
  mode set                    <conversation-id> --phase standard|planning|implementation
  plan set                    <conversation-id> --auto-approve <bool>
  final-audit set             <conversation-id> --enabled <bool>
  model set                   <conversation-id> --slot plan|act|vision|utility|lite --model-config <json>
  approval set                <conversation-id> ...
  sandbox set                 <conversation-id> ...
  freeze                      <conversation-id>
  start                       <conversation-id>
                              (--prompt <text> | --problem-file <path>)
                              --expect-config-hash <sha256>
                              [--budget <path>] [--artifact-dir <path>]
  status|events|signal|approve|reject|stop <conversation-id>

agent
  list|get|validate|export
  plan-put|put|remove

skill
  list|inspect|resolve|vet
  plan-install|install|verify|remove|rollback

mcp-server
  list|inspect
  acquire|verify-package
  plan-register|register|smoke|enable|disable|remove|rollback

config
  export|inspect|diff
  plan-import|import|rollback

doctor
  run
  check skill-integrity|mcp-connectivity|config-consistency
        model-availability|sandbox-runtime|main-process

experiment
  resolve            --agent <id> --spec <experiment.json> [--dry-run]
  run                --agent <id> --spec <experiment.json>
                     (--prompt <text> | --problem-file <path>)
                     [--follow] [--artifact-dir <path>]
  status|events|signal|approve|reject|stop <run-id>

eval
  run                --suite <ref> --experiment-spec <file>|--conversation <id>
  status|inspect|cancel
compare
report
replay
  verify|sandbox

improve
  init|status|propose|evaluate|compare|promote|reject|stop
  archive list|show|lineage

internal
  serve|owner-status|protocol-schema
```

`mcp-server` 管理全局已安装的 MCP server；`conversation mcp set` 只选择本次 conversation 可见、自动批准或自动展开的 MCP tools。二者必须分开，避免“为了测试一个 MCP 组合”意外改动全局 server 注册。

CLI 提供两层入口，但只有一套语义：

- `conversation ...` 是低层原语，便于逐步创建、观察、修改和调试；
- `experiment run` 是日常实验的推荐简化入口。它把 Agent defaults、experiment spec overrides、prompt/workspace、预算和 artifact 要求一次提交给主进程，由 `ExperimentService` 原子执行 resolve → normalize → freeze → create → start；
- `experiment resolve --dry-run` 只返回 resolved manifest 和 hashes，不创建 conversation；
- 高层入口不得在 CLI 客户端连续拼接多次 mutation，也不得直接调用 executor。它必须在服务端复用与低层 API 相同的 `RunConfigResolver`、freeze 和 start operations，从而避免中间状态竞态或运行语义分叉。

因此用户可以在前端优先配好 Agent 默认值；大多数实验只需一条 `experiment run` 并在 spec 中写差异项。需要精细诊断时再展开成 `conversation create/config/freeze/start`。

不建议公开任意 `api call <method>`，以免绕过 command 级能力和审批边界。CLI 是 typed HTTP/SSE client，用户和 Agent 只能调用稳定 commands。

### 5.1 输出和退出契约

- human 输出默认供人阅读；
- `--output json` 返回单个稳定 envelope；
- `--output jsonl` 用于事件流；
- stdout 只输出结果/协议，日志和进度写 stderr；
- error code 必须机器可判定，例如 `INVALID_ARGUMENT`、`NOT_FOUND`、`CONFLICT`、`POLICY_DENIED`、`BUDGET_EXCEEDED`、`INFRA_FAILURE`；
- 安装、启停、删除命令应幂等；重复达到目标状态应 exit 0 并返回 `already_applied=true`；
- 任何 schema 都带 `schema_version`。

建议退出码：

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功 |
| 1 | 通用执行失败 |
| 2 | 参数或 schema 错误 |
| 3 | 资源不存在 |
| 4 | 权限/安全策略拒绝 |
| 5 | 版本、revision 或 owner 冲突 |
| 6 | 网络/依赖获取失败 |
| 7 | 状态恢复失败，需要 doctor |
| 8 | batch 部分成功 |
| 9 | 预算或资源上限触发 |

## 6. Conversation 配置与 Headless workflow 协议

### 6.1 配置来源、覆盖和冻结

实验配置采用三层合并，全部由主进程中的 `RunConfigResolver` 完成：

```text
用户在前端保存的 Agent 默认配置
+ conversation 创建时或随后写入的 workflow-level overrides
+ benchmark runner 注入的非产品元数据（预算、artifact、task/run ID）
= resolved immutable run snapshot
```

前两层对应现有 `Agent` 与 `AgentConfig` 语义。`AgentConfig` 已覆盖：

- `availableTools` 与 `autoApprove`；
- `mcpTools.available/autoApprove/autoExpand`；
- `skillEnabled` 与 `selectedSkills`；
- `phase`、`autoApprovePlan`、`finalAudit/finalReviewMode`；
- `models.plan/act/vision/utility/lite`；
- `approvalLevel`、`autoCompress`、`maxContexts`；
- `allowedPaths`、`shellPolicy`、sandbox mode/scheme/config；
- personality。

主进程必须复用现有 `build_workflow_config_for_request`、`merge_inherited_workflow_config`、`fill_missing_agent_config_fields`、`normalize_agent_tool_config` 与 `enforce_auto_approve_tool_visibility` 所表达的规范，抽成前端和 CLI 共用的 resolver，不能在 CLI 中重写一份字段合并逻辑。

规则：

1. `conversation create --agent` 从该 Agent 当前默认配置创建持久化 conversation，但不立即执行；
2. `conversation ... set/patch` 只更新 conversation 的 `AgentConfig`，不得写回 Agent 表；
3. 用户未显式覆盖的字段继续继承创建时解析出的 Agent 默认值；
4. 工具、MCP、skill 和 auto-approve 必须在主进程根据当前真实 registry 过滤并归一化；
5. `conversation freeze` 返回 canonical resolved snapshot、`config_hash`、`prompt_hash` 和 `tool_schema_hash`；
6. `conversation start --expect-config-hash` 使用 compare-and-start，hash 不一致就拒绝，防止前端或另一个客户端在 inspect 与 start 之间改了配置；
7. start 后 snapshot 成为该 trial 的权威配置。运行中允许的变更仍遵循现有 tool boundary、phase transition 和 resume 规则，不能为实验新增即时热切换捷径；
8. artifact 必须记录 Agent default revision、conversation override、resolved snapshot 和上述 hashes，才能重现实验。

### 6.2 模型切换

用户可以先在前端为 Agent 配好 `models` 默认值。实验只用 `conversation model set` 覆盖需要比较的 slot：

- 标准/执行阶段主要使用 `act`；
- planning 阶段使用 `plan`，缺失时按现有规则回退；
- 压缩、轻量任务和视觉分别使用现有 `utility/lite/vision` 语义。

不能只提供一个全局 `--model` 覆盖所有 slot，因为这会改变计划、执行、压缩和辅助调用的含义，使实验结果无法与前端真实配置对齐。每次模型 override 都必须包含完整的 provider/model config 引用及 temperature、thinking、function-call、context size、max tokens 等受支持字段，并经过主进程验证。

### 6.3 两种等价的实验入口

#### 低层入口：逐步控制 conversation

适合调试协议、人工探索和检查每个中间状态：

```bash
# 1. 从用户已配置好的 Agent 默认值创建 conversation
chatspeed conversation create --agent coding --workspace /workspace --output json

# 2. 只修改本次实验变量
chatspeed conversation tools set <id> --from-file tools-minimal.json
chatspeed conversation skills set <id> --enabled true --names chatspeed-cli
chatspeed conversation mcp set <id> --file mcp-none.json
chatspeed conversation mode set <id> --phase planning
chatspeed conversation plan set <id> --auto-approve false
chatspeed conversation final-audit set <id> --enabled true
chatspeed conversation model set <id> --slot act --model-config act-model.json
chatspeed conversation model set <id> --slot plan --model-config plan-model.json

# 3. 让主进程归一化和冻结，取得 revision/config_hash
chatspeed conversation freeze <id> --output json

# 4. compare-and-start；事件以 JSONL 输出
chatspeed conversation start <id> \
  --problem-file task.md \
  --expect-revision 8 \
  --expect-config-hash sha256:... \
  --artifact-dir run-001 \
  --output jsonl
```

#### 高层入口：一步式实验 facade

面向常规 A/B 和 benchmark，CLI 只提交一份 spec，主进程原子完成同样的步骤：

```bash
chatspeed experiment run \
  --agent coding \
  --spec experiments/minimal-planning-final-audit.json \
  --problem-file task.md \
  --follow \
  --artifact-dir run-001 \
  --output jsonl
```

示例 experiment spec：

```json
{
  "schema_version": 1,
  "workspace": "/workspace",
  "overrides": {
    "harness_profile": "minimal",
    "available_tools": ["read_file", "edit_file", "bash"],
    "auto_approve": ["read_file"],
    "mcp_tools": {"available": [], "auto_approve": [], "auto_expand": []},
    "skill_enabled": true,
    "selected_skills": ["chatspeed-cli"],
    "phase": "planning",
    "auto_approve_plan": false,
    "final_review_mode": "sub_agent_review",
    "models": {
      "act": {"id": 101, "model": "provider/model-a"},
      "plan": {"id": 102, "model": "provider/model-b"}
    }
  }
}
```

这里的 JSON 使用 control-plane canonical snake_case；主进程 adapter 会映射到现有 Rust `AgentConfig`。高层 `experiment run` 返回 run ID、conversation ID、revision、完整 resolved snapshot 和 hashes；其结果必须与手工执行低层流程完全等价。

benchmark matrix 可从同一 Agent 默认值派生多份 experiment spec，每份只改变一个变量。例如：

```text
Full + standard + final-audit off + current models
Minimal + standard + final-audit off + current models
Minimal + planning + final-audit on + alternate plan model
Minimal + selected MCP tools + selected skills + alternate act model
```

比较时必须记录完整 resolved snapshot，不能用模板名称代替实际配置。

### 6.4 运行必须复用真实 kernel

`conversation start` 应调用抽出的 `WorkflowRuntimeService::start/resume/signal/stop`，该 service 负责：

- workflow/snapshot 创建和恢复；
- resolved Agent/harness profile；
- WorkflowManager 注册；
- signal channel；
- ExecutionPolicy 和 approval；
- `WorkflowExecutor::run_loop()`；
- hot resume、取消和资源清理；
- ccproxy key 与 usage attribution。

不能在 CLI 中复制 `workflow_start`，也不能单独用一个小 ReAct loop 模拟桌面 Agent。否则 benchmark 测到的不是产品实际行为。

### 6.5 SSE 事件与 CLI JSONL 投影

主进程 SSE 应复用现有 `GatewayPayload` 类型，并为 transport 增加稳定 envelope：

- `protocol_version`、`server_instance_id`、`session_id`；
- `stream_cursor` 与可用时的 `durable_event_id`；
- `kind` 与 typed payload；
- emitted timestamp 和必要的 provenance。

CLI 的 `--output jsonl` 是 SSE envelope 的逐行投影，不另外定义 command/response/event 状态机。普通 HTTP response 使用统一 JSON envelope，并由 request ID 关联。

完成判定必须来自结构化 terminal state 或 `TaskCompleted`，不能依赖最后一段 assistant 文本。流式 delta 不应重复携带完整累积文本，避免轨迹二次增长。

`events.jsonl` 是 DB 权威事件的导出或带 provenance 的 runner 投影，不取代 DB。候选 Agent 写入的日志必须标为 `provenance=model`，不能用于满足“测试已运行”等门禁。SSE stream cursor 与 DB durable event ID 不能混为一个字段：前者用于短期实时重连，后者用于 snapshot/replay 权威恢复。

### 6.6 审批与停止

- 普通实验默认继承 conversation 从前端 Agent defaults 解析出的审批、工具、MCP、skill、计划和终审配置；只有 benchmark template 明确声明 override 时才覆盖；
- auto-approve 必须仍是实际可用工具的子集；
- shell policy 继续独立生效；
- budget、timeout 和 cancel 必须传播至 LLM、工具、MCP child 和 benchmark trial；
- 停止后先关闭新 effect admission，再等待/终止活动操作；
- CLI 不应通过删除 session 或杀死单个 UI channel伪造停止。

## 7. CLI 作为 skill/MCP/Agent 管理控制面

### 7.1 单一内置 `chatspeed-cli` skill

Minimal Agent 不应完全删除 `skill` 工具，而应只允许激活一个内置 skill：`chatspeed-cli`。

该 skill 的职责仅是按需加载：

- CLI 命令索引；
- typed 参数和输出示例；
- 风险等级；
- 哪些操作需要用户审批；
- 如何先 plan/dry-run，再 apply，再 doctor。

安全要求：

- `chatspeed-cli` 是保留名称，从不可变 bundled resource 加载；
- 用户目录中的同名 skill 不得覆盖它；发现冲突时忽略并由 doctor 报告；
- skill 文本不包含 secret、control-plane bearer token、discovery 路径或动态 shell 拼接；
- skill 不能授予权限；所有权限在 CLI/service 中校验；
- skill 不允许直接写 `~/.chatspeed`、SQLite 或 lock manifest；
- 通用用户 skills 不在 Minimal 的模型可见列表中。

后续可把 `skill-installer`、`skill-vetter`、`skill-creator` 等多个辅助 skill 的真实功能迁入 CLI；原 skill 最终只保留兼容跳转或移除。

### 7.2 Mutation 使用 plan → approve → apply

每个高风险 mutation 先生成 canonical plan：

```json
{
  "schema_version": 1,
  "data_domain_id": "experiment-123",
  "operation": "skill_install",
  "source": {},
  "resolved_digest": "sha256:...",
  "permissions": {},
  "effects": [],
  "rollback": [],
  "plan_digest": "sha256:...",
  "approval_required": true
}
```

批准凭据绑定：

```text
data_domain_id + command + plan_digest + actor + expiry + nonce + max_effects
```

凭据一次使用后失效，不能出现在 argv、环境变量、模型 transcript 或普通 artifact 中。生产数据域的 mutation 必须有人类审批；improve campaign 获得的窄 capability 永远不得指向 production。

### 7.3 Skill 安装 pipeline

推荐：

```text
resolve source/version
→ download to staging
→ enforce archive/path/size limits
→ verify digest/signature if trusted metadata exists
→ parse manifest
→ programmatic vet and permission extraction
→ approval
→ immutable content store
→ update lock/journal
→ atomic target switch
→ rescan and doctor
```

注意：

- hash 只有与可信 registry 中的预期 digest 绑定时才证明来源内容没有被替换；仅对下载结果自行计算 hash 不证明作者身份；
- signature 也必须绑定受信 key、source、version 和 digest；
- prompt 型 vetter 只能提供解释或补充 review，不能给最终安全 verdict；
- 必须检查路径穿越、绝对路径、symlink/hardlink/device、zip bomb、文件数/大小、可执行文件、脚本、依赖安装、网络域名和权限声明；
- 文件系统、SQLite 和 live runtime 无法形成一个数据库事务，应使用 journaled saga，每个步骤幂等且可补偿；
- 在每个 commit point 崩溃后，doctor 必须能确定性 roll-forward 或 rollback。

### 7.4 MCP acquisition 与注册分离

MCP 比普通 skill 风险更高，因为 stdio server 会执行进程，streamable HTTP 会携带网络与 token。

建议流程：

```text
mcp acquire
→ verify-package
→ plan-register
→ register disabled
→ sandbox smoke test
→ inspect exposed tools and permissions
→ human approval
→ enable
```

smoke test 必须：

- 不挂载宿主 home、生产 DB 或 secrets；
- 默认无网络，按域名最小放行；
- 限制 CPU、内存、磁盘、进程数和时间；
- 将 package acquisition 与 MCP config registration 分别记录；
- 失败后保持 disabled，而不是重试到成功。

当前 `McpServerConfig` 包含 `bearer_token` 和 `env`，CLI/GUI 输出应默认 redact；MCP secrets 应迁入与 API key 相当的受保护存储，而不是作为普通可导出 JSON 处理。

## 8. 数据域、主进程所有权与并发一致性

需要区分三个不同概念：

- **HarnessProfile**：Full/Minimal，只控制 AI 可见 prompt 与工具投影；
- **Agent defaults**：用户在前端保存的长期默认配置；
- **Conversation snapshot**：一次实验实际使用的 workflow `AgentConfig` 和模型/tool/MCP/skill/phase/final-audit 组合。

普通本机实验应直接使用当前主进程和当前用户数据域，不需要为每次 trial 复制产品数据库。并发一致性规则是：

- 只有主进程打开 `MainStore` 并拥有 ConfigCache、ToolManager、MCP 和 WorkflowManager；
- CLI 与前端都通过主进程 commands 修改或读取 conversation；
- conversation config 更新使用 revision/CAS；stale revision 必须拒绝；
- `freeze` 与 `start --expect-config-hash` 提供 compare-and-start，防止配置竞态；
- 多个实验 conversation 可以并行，但每个 conversation 的 mutable config 和 workflow lifecycle 仍由主进程串行化到规范边界；
- Agent defaults 在 conversation 创建后变化，不得静默改写已冻结 trial；如需采用新默认值，应显式 `config reset --from-agent` 并重新 freeze。

外部 Harbor/container benchmark 可以启动独立的**无窗口主进程实例**，并为该实例使用隔离数据域：

```text
experiment-data/<run-id>/
  chatspeed.db
  skills/
  packages/
  cache/
  artifacts/
  journals/
  runtime/
```

该隔离实例仍由自己的主进程唯一打开数据库；CLI 只连接它。隔离范围包括 ccproxy key/port/socket、MCP child、HOME/env allowlist、workspace、artifact、budget 和 secret references。conversation template 可以从用户配置导出后导入隔离实例，但必须去除或显式映射 secrets。

SQLite WAL 和进程内 lock 不能替代这一 owner 模型；不应保留 CLI 直开任一产品或实验数据库的路径。

## 9. Benchmark 选择

不能用一个榜单覆盖所有风险。建议按用途分层。

### 9.1 Aider Polyglot：快速开发集

官方说明：从 Exercism 六语言 697 题中选择最难的 225 题，语言为 C++、Go、Java、JavaScript、Python、Rust。它适合：

- prompt/tool schema 快速 A/B；
- edit/write 格式、测试执行和六语言基本能力；
- 低成本高频 smoke/dev gate。

限制：

- 题目公开，污染风险高；
- 多为小型算法练习，不代表真实 repository maintenance；
- 不能单独决定产品晋级。

来源：
- <https://aider.chat/2024/12/21/polyglot.html>
- <https://github.com/Aider-AI/polyglot-benchmark>

### 9.2 SWE-bench Verified：真实仓库稳定主集

官方说明：500 个由软件工程师确认可解的问题；将候选 patch 应用到真实 GitHub repository，并在 Docker 中运行测试。

适合：

- 稳定的真实 issue 解决能力对比；
- patch、定位、回归测试和 repository navigation；
- 与公开结果对照。

限制：

- 主要为 Python；
- 公开且使用广泛，污染风险存在；
- 本地评测资源要求较高，官方建议约 120GB 磁盘、16GB RAM、8 CPU，ARM 支持仍有限；
- harness 按 `run_id + instance_id` 缓存，候选 diff 变化时必须使用新 run ID。

来源：
- <https://www.swebench.com/SWE-bench/>
- <https://www.swebench.com/SWE-bench/guides/evaluation/>
- <https://github.com/SWE-bench/SWE-bench>

### 9.3 SWE-bench-Live/MultiLang：新鲜真实任务集

官方仓库截至 2026-08 报告 MultiLang 有 1,077 个任务、431 个 repository、8 种语言，并支持 Linux/Windows 数据。任务持续增加，更适合降低陈旧 benchmark 的污染风险。

协议尤其重要：rollout 只能让 Agent 访问 `problem_statement` 与 task image；不能访问 hint、FAIL_TO_PASS、test patch 或 ground-truth evaluation result；完整轨迹需保留以审查是否泄漏。

适合：

- 多语言真实 repository 验证；
- 新模型与新 Agent 的污染敏感 gate；
- 定期刷新能力评估。

限制：仍是公开数据，不能替代私有 holdout。

来源：
- <https://github.com/microsoft/SWE-bench-Live>
- <https://swe-bench-live.github.io/>

### 9.4 Terminal-Bench 2.0 / Harbor：系统与终端能力

Harbor 是 Terminal-Bench 2.0 的官方 harness，支持容器任务、Agent adapter、并行运行、网络 policy、独立 verifier 和多指标 reward。

适合：

- 构建、命令行、服务配置、依赖安装和端到端系统任务；
- 验证 bash、环境恢复、长任务和资源限制；
- 与多个标准 Agent 对照。

限制：任务不全是 coding；成本和环境复杂度高，不适合每个候选都全跑。

来源：
- <https://github.com/laude-institute/harbor>
- <https://harborframework.com/docs>
- <https://harborframework.com/docs/agents>
- <https://harborframework.com/docs/task-format>

### 9.5 SWE-smith：训练和开发任务来源

SWE-smith 提供约 52K task instances 和 250+ environments，可以把 repository 转成 SWE-gym 并合成任务。适合：

- 建立大规模 train/dev pool；
- 生成失败轨迹；
- 为反思型优化提供样本。

它不应作为唯一最终 test，因为任务生成和大量公开轨迹可能被候选优化过程接触。

来源：<https://github.com/SWE-bench/SWE-smith>

### 9.6 私有 ChatSpeed holdout：最终晋级必需

需要维护一批不进入 proposer 上下文的私有任务，覆盖：

- Rust/Tauri command 与 SQLite 状态；
- Vue/TypeScript 交互；
- 跨 Rust/Vue 的命令契约；
- workflow waiting/approval/recovery/compression；
- prompt/tool schema 变化；
- skill/MCP/config 管理；
- 用户工作树保护和最小 diff；
- 安全、路径、secret 与 sandbox 边界。

私有 holdout 只允许 promotion finalist 低频运行，只返回聚合 verdict 和置信区间，不给逐题失败细节。记录每个 task 的 exposure count，达到阈值后轮换。每道题必须有隐藏测试、oracle 或明确人工 rubric，并在纳入前验证 oracle 100% 通过。

## 10. 采用 Harbor，而不是自造 benchmark runtime

ChatSpeed 只需实现 Harbor installed-agent adapter：

- 在 task container 内启动隔离的无窗口 ChatSpeed 主进程；
- 安装 `chatspeed` CLI，并让它只通过该主进程的 loopback HTTP/SSE control plane 工作；
- 导入去密后的 Agent defaults 或 conversation template；
- 为每个 trial 调用 `conversation create`，应用本次 matrix overrides，再 `freeze`；
- 将 instruction 传给 `conversation start --expect-config-hash`；
- 收集 resolved conversation snapshot、结构化 trajectory、usage、patch 和终止状态；
- 让 Harbor 在候选不可见的 verifier 环境中判分。

首版不推荐 external-agent adapter，因为那要求 ChatSpeed 从宿主远程控制容器内文件系统和工具，容易形成第二套工具 backend。installed-agent 能直接测试最终 CLI 形态及其自带工具。

适配后必须做 parity：同一 Agent、模型、prompt、配置和任务在原始 benchmark 与 Harbor adapter 上多次运行，得分范围应重叠；先做 5–10 题 sanity，再双方各一次 full，最后三次以上估计均值和 sample SEM。

## 11. 可靠判分

### 11.1 结果优先于自述

优先级：

1. 隐藏测试与环境最终状态；
2. 编译、静态分析和安全扫描；
3. runner-owned 工具事件、policy violation 和成本；
4. 必要时使用经人工校准的 LLM rubric；
5. 人工 spot check 与最终晋级审批。

模型说“测试通过”不构成任何分数。工具执行事实必须来自 dispatcher/runner 产生的 admitted/started/finished 事件；verdict 来自候选不可写的 separate verifier。

### 11.2 Correctness 与成本不能先混成单分数

采用字典序 gate：

1. 安全和完整性 hard gate；
2. correctness 最低线及关键 suite 非回归；
3. 只有前两项通过后，才比较 token、成本、延迟、轮次和审批次数；
4. 保留 Pareto front，不强行把不同权衡压成一个 score。

一个更便宜但正确率显著下降的候选不得晋级；一个正确率略高但成本无上限的候选也不得晋级。

## 12. 受控自我改进闭环

### 12.1 Mutable surface

首期只允许候选修改声明式实验数据，并通过新的 conversation override 表达：

- Minimal core/coding prompt 的候选文本或其版本引用；
- 本次 conversation 的工具暴露；
- AI-facing tool description 版本；
- 本次 conversation 的模型 slot、MCP、skills、phase、auto-approve-plan 与 final-audit 组合；
- 单一内置 CLI skill 的非权限性说明。

用户在前端维护的 Agent defaults 作为基线输入，不由候选直接修改。候选晋级后如需成为新的产品默认值，必须生成独立变更计划并由用户批准。

禁止候选修改：

- evaluator、private holdout 和 verifier；
- promotion policy 与预算账本；
- main-process ownership、control-plane auth 和 CLI capability；
- secret 管理；
- PathGuard、sandbox 和审批信任核心；
- workflow reducer、完成边界和 DB authority；
- benchmark image、测试和 oracle。

代码级自改不是上述阶段的自然延伸；如未来需要，必须另立项目、隔离 repository 和人工 code review。

### 12.2 推荐阶段

#### Stage 0：人工单变量 paired A/B

每个候选只改变一个 surface，例如：

- 删除一段 prompt；
- 隐藏一组工具；
- 缩短一个工具 description；
- 保留/移除 `glob`；
- 合并 `todo_list/get` 的实验版本。

使用相同 task、模型快照、预算、环境和执行次序交替跑 baseline/candidate。先校准 runner、artifact、成本和方差，不要一开始自动生成大量候选。

进入下一阶段前：至少完成三个独立 campaign，能从保存 artifact 重算 promotion verdict，且无泄漏、预算、Agent defaults、conversation 或数据域污染事故。

#### Stage 1：GEPA-like 反思优化

proposer 只读取 train 轨迹和 evaluator 提供的可行动诊断，提出 prompt/tool/config candidate；候选在 dev 上快速筛选，在 val 上做晋级。

GEPA 的可借鉴点：

- 从完整轨迹而非单个 scalar 反思；
- 将失败归纳为高层规则；
- 测试 prompt mutation；
- 用 Pareto selection 保留对不同任务子集有效的候选；
- 使用比 RL 少得多的 rollout。

必须保留独立 val 与最终私有 test；如果 val 参与大量候选选择，它也会逐步过拟合，应定期轮换。

来源：
- <https://arxiv.org/abs/2507.19457>
- <https://dspy.ai/learn/optimization/optimizers/>

#### Stage 2：DGM-like allowlisted archive

只有 Stage 1 连续 campaign 停滞、评测器稳定后才增加：

- parent/lineage；
- 从非最优 ancestor 分支；
- novelty/diversity；
- 跨模型、语言和 suite transfer gate；
- archive/Pareto 多样保留。

DGM 的经验说明开放 archive 能找到 patch validation、更好的文件读取/编辑、多方案排序和历史记忆等改进；但其官方也记录了伪造工具日志和删除 reward detector marker 的 reward hacking。因此 ChatSpeed 不应让候选读写 verifier 或自己声明测试事实。

来源：
- <https://sakana.ai/dgm/>
- <https://arxiv.org/abs/2505.22954>
- <https://github.com/jennyzzt/dgm>

### 12.3 一次 campaign

```text
freeze baseline Agent defaults + suite/image/policy digests
→ create a new conversation from the baseline Agent
→ apply one allowlisted conversation/config candidate override
→ resolve and freeze conversation; record config/prompt/tool-schema hashes
→ create isolated benchmark worktree or container data domain
→ static policy and schema validation
→ cheap smoke
→ public train/dev trials
→ paired comparison
→ val gate
→ shortlist finalists
→ private holdout, low frequency
→ independent promotion verdict
→ human review/approval
→ package signed candidate
→ promote or archive/reject
```

失败候选仍可保留在 archive 中作为诊断或多样性节点，但不能获得生产能力。

## 13. 防 reward hacking、污染、噪声和成本失控

### 13.1 Reward hacking

- runner、artifact manifest 和 verifier 位于候选不可写边界；
- Harbor 使用 separate verifier；candidate 只交付 allowlisted patch/artifact；
- event 带 `provenance=runner|tool|model|verifier`；
- model-generated 日志永远不能满足 gate；
- evaluator、promotion、sandbox、secret 和 CLI trust core 只读；
- 任何 verifier tamper、隐藏测试读取、生产写入、网络越界或伪造事件都直接 hard fail，不能用更高正确率抵消；
- safety violation 冻结整个 lineage，不能只删除当前节点后继续从其后代优化。

### 13.2 Benchmark 污染

- split registry 固化为 train/dev/val/private_holdout；
- SWE-bench-Live 严格只给 problem statement 与 image；
- private holdout 不进入 proposer、archive 反思或逐题报告；
- 每个 finalist 的 holdout 调用限频并计 exposure；
- 检测通用 prompt/skill 中的 task-specific issue、patch 和测试字符串；
- public benchmark 只证明公开 benchmark 能力，不单独触发生产晋级。

### 13.3 随机噪声

- baseline 与 candidate 使用相同 task、环境、模型、预算；
- 按时间 block 交替运行，减少 provider 漂移；
- 二元正确性使用 paired bootstrap 与 McNemar/sign test；
- 报告 95% CI、effect size、每语言和每 suite floor；
- 距晋级阈值一个标准误以内的候选至少三次独立 rollout；
- 预先写定 threshold，不在看到结果后移动门槛。

初始晋级建议：

- hard-safety violation = 0；
- 主正确性指标提升至少 2 个百分点，且差值 95% CI lower bound > 0；
- 任一关键 regression suite 的退化上界不超过 1 个百分点；
- p95 latency、成功任务成本和 token 不超过预设 cap；
- Full + 弱模型路径零非预期变化。

样本不足时应先做 power analysis，而不是机械套用该阈值。

### 13.4 预算

预算必须在 effect admission 前执行，而不是只做事后统计：

- per-request；
- per-trial；
- per-candidate；
- per-campaign。

限制美元、input/output/cache token、wall time、tool calls、进程、磁盘、网络字节和并发数。ccproxy 在发请求前 reserve 最坏成本；账本不足则 fail closed。infra failure 单独计数，不应算作 Agent correctness failure，但超过阈值应暂停 campaign，不能无限 retry。

### 13.5 停止条件

任一条件满足即停止当前 campaign：

- 达到成本、时间、trial 或 candidate 上限；
- 连续五个有效 generation 无超过最小效应的 val 改进；
- val 与 holdout gap 超过预设阈值；
- 任一 hard-safety violation；
- infra failure 超过 5%；
- artifact provenance 或预算账本不完整；
- 关键 suite 显著回归；
- 提升完全来自突破成本上限；
- archive 多样性坍缩且连续两轮没有新 Pareto 点。

## 14. Artifact 最小集

每个 run：

```text
run.json
workflow.db-ref.json
events.jsonl
trajectory.jsonl
result.json
patch.diff
artifacts/manifest.json
verifier/verdict.json
```

每个 campaign：

```text
campaign.json
candidates/<id>/candidate.json
candidates/<id>/mutation.json
candidates/<id>/evaluation.json
archive.jsonl
promotion/verdict.json
```

关键字段：

- `run.json`：run/conversation/candidate ID，base Agent ID/revision，resolved `AgentConfig` 与 override hash，HarnessProfile，`models.plan/act/vision/utility/lite`，dataset/version/digest/split，image/policy/tool-schema/system-prompt hash，repo/base commit，budget、seed 和 timestamps；
- `events.jsonl`：run、sequence、event ID、type、provenance、actor、session/tool-call ID、payload hash、usage、前一事件 hash；
- `candidate.json`：parent、mutable surface、base/change hash、proposer、rationale、lineage 和 holdout exposure；
- `verdict.json`：per-suite metrics/CI、safety/cost gates、infra failures、最终决定与 human approval reference；
- `install-plan.json`：source identity、resolved ref/digest、signature、permissions、risk、effects、rollback 和 plan digest；
- `lock-manifest.json`：package/version/origin/content digest/signer/files/permissions/transaction/previous version。

禁止写入 API key、control-plane bearer token、完整环境变量和 private verifier 内容。公开 SWE-bench-Live 轨迹前需运行专用 redaction 和协议审计。

## 15. 分阶段实施建议

### Phase 0：共享 Application Contract 与 characterization

只定义并验证：

- `WorkflowApplicationService` 的 transport-neutral DTO 和稳定错误；
- `RunConfigResolver` 与 conversation config revision/hash；
- `ExperimentService::resolve/run` 对低层原语的等价编排；
- RuntimeHub/EventBroker 的 ownership、stream/durable cursor 和 backpressure 语义；
- HTTP/JSON、SSE 与 CLI JSONL schema v1。

验收：保持现有 Tauri UI 运行不变；同一 fixture 经 Tauri adapter 与直接 service 调用得到等价 resolved config、snapshot、durable events 和终态；高层 experiment run 与低层 create/config/freeze/start 结果及 hashes 一致；并发 config patch 的 stale revision 100% 被拒；schema round-trip/golden 100%。

### Phase 0.5：端到端原型（Thin Vertical Slice）

**目标**：用最小实现验证整个链路可行性，早期发现集成问题，验证技术选型正确性。

**实现范围**：
- 一个简化的 HTTP endpoint：`POST /control/v1/experiments:run-minimal`；
- 一个最小 CLI 命令：`chatspeed experiment run-minimal`；
- 只支持一个固定 Agent 配置（minimal coding agent）；
- 只运行一个简单任务（如"创建 hello.txt 文件并写入指定内容"）；
- 验证完整链路：conversation 创建 → freeze → start → tool execution → completion → artifact 收集；
- 基础 SSE 事件流：至少支持 state change 和 completion 事件；
- 简化的 artifact 收集：`run.json`、`events.jsonl`、`result.json`。

**不包含**：
- 完整的 config override 和 revision/CAS；
- 复杂的 benchmark integration；
- MCP 和 skill 动态配置；
- Budget admission 和 resource limiting；
- 完整的错误恢复和 replay。

**验收标准**：
- CLI 能成功触发一次完整 workflow；
- 能通过 SSE 接收到 real-time events；
- 能收集到 structured events 和 final verdict；
- 相同任务重复运行 10 次，成功率 100%；
- 单次运行端到端延迟 < 30 秒（不含 LLM 响应时间）；
- HTTP/SSE 连接异常后 CLI 能正确报错退出；
- 主进程 crash 后遗留的临时资源能被 doctor 检测到。

**价值**：
- 验证 loopback HTTP/SSE 方案在实际环境中的可行性；
- 暴露 Tauri/HTTP adapter 共存的潜在冲突；
- 为后续 phase 提供 working skeleton 和 integration test 基础；
- 建立最小的端到端性能 baseline；
- 验证 artifact 存储和检索的基本流程。

### Phase 1：Workflow service 抽取、Conversation 配置与 Tauri parity

- 将 create/config/freeze/start/resume/signal/stop 编排下沉到共享 service；
- Tauri commands 变成薄 adapter，现有 Vue 继续 invoke/listen；
- 把 session input registry 与 Tauri event batching 分离为唯一 RuntimeHub + fan-out broker；
- 复用同一 `AgentConfig` 归一化与继承规则；
- WorkflowManager 仍是 live session 唯一 registry。

验收：从前端配置的 Agent defaults 创建 conversation 后，tools、MCP、skills、models、phase、autoApprovePlan、finalAudit、approval、sandbox 和 shell policy 的 resolved snapshot 与旧前端执行路径一致；active、所有 wait reason、等待中 refresh/restart、approval round-trip、completed resume、compression/rebuild 场景通过；权威 event/state golden 无差异。

### Phase 2：Loopback Web control plane 与基础 CLI

- 新建独立 `/control/v1` Axum router，不复用 static server 的 Any CORS 和 ccproxy auth；
- 实现 current-user discovery、bearer auth、protocol negotiation、idempotency、revision/CAS；
- 实现 HTTP commands 与 SSE subscriptions/replay/reset/backpressure；
- 提供 `conversation create/config/freeze/start/status/events/stop`、`experiment resolve/run` 和 `doctor`；
- CLI 将 HTTP response/SSE 投影为 human、JSON 或 JSONL。

验收：缺失/错误 token、URL token、非 loopback bind 和未批准 Origin 100% 拒绝；重复 create/start/signal 不产生双重 effect；SSE 断线在窗口内续传、窗口外通过 snapshot + durable events 收敛；慢客户端不阻塞 workflow 且不会静默漏掉控制事件；Tauri UI 与 CLI 同时观察/操作同一 conversation 后状态一致。

### Phase 3：无窗口主进程与 container runtime

**主进程生命周期策略**：
- **短期方案（Phase 3-6）**：GUI 和主进程绑定，长时间实验建议在 container 中运行独立实例；
- **长期方案（Phase 7+）**：演进为独立 daemon（`chatspeedd`），GUI 成为 client 之一；
- 用户关闭 GUI 时，若有 active experiments，弹出警告："N 个实验正在运行，关闭将中止这些实验。建议：[最小化到托盘] [仍然关闭]"；
- Container 内的无窗口实例使用独立数据域，与用户桌面环境完全隔离。

**实现重点**：
- 解耦窗口绑定的 ccproxy/tool setup 和 core bootstrap；
- 支持无 display 启动相同 main-process services 与 control plane；
- 增加 budget admission、取消传播和 isolated data-domain bootstrap；
- Harbor 使用同一 loopback HTTP/SSE client contract。

验收：不构造 Tauri window 即完成一次真实 LLM + tool workflow；同一 Agent defaults 可创建至少四种不同 experiment spec 并记录不同 config hash；stop p95 < 2 秒；预算不超过 reserve 上限；CLI crash 不影响主进程权威状态且无孤儿 MCP/LLM task；两个 owner 针对同一数据域启动时第二个明确失败；control plane 默认不监听 container 外部接口。

### Phase 4：共享管理 services

- 抽 Agent sanitize/validation；
- 抽 MCP persistence/runtime transition；
- 复用 config transfer；
- 增加 revision/CAS 与 audit。

验收：GUI/CLI 交错 1,000 次 mutation 无 lost update 或 cache/runtime 分叉；stale revision 100% 拒绝。

### Phase 5：安全 package pipeline 与单一内置 skill

- Rust 原生 skill/package 安装；
- 程序化 vet；
- permission manifest；
- approval-bound apply；
- journal/rollback；
- reserved `chatspeed-cli` skill。

验收：path traversal、symlink/hardlink/device、zip bomb、超限文件、未声明执行/网络 100% 拒绝；每个 commit point crash 后 doctor 100% 收敛；未批准 MCP 永不启动；用户 skill 不能覆盖 CLI skill。

### Phase 6：Harbor adapter 与分层评测

- installed-agent adapter；
- Polyglot 快速集；
- SWE Verified/Live；
- Terminal-Bench 选定子集；
- 私有 ChatSpeed holdout。

验收：固定 patch 在固定 verifier/image 上 verdict 100% 一致；伪造日志/reward/隐藏测试读取 100% 被拒；所有 run 可由 manifest 和 hash 验证 provenance；Harbor adapter 与原 benchmark parity 成立。

### Phase 7：人工 A/B，再到 GEPA-like

- 先建立统计、成本和晋级门禁；
- 至少三个稳定 manual campaign 后，开放 allowlisted proposer；
- 私有 holdout 限频；
- promotion 可从 artifact 独立重算。

### Phase 8：条件式 DGM-like archive

仅在 GEPA-like 搜索停滞且 evaluator 稳定时增加 lineage、diversity 和开放分支；仍不得进入代码级 trust-core mutation。

## 16. 当前实施优先级

第一批实现不应直接覆盖完整命令树。最有价值且风险最小的顺序是：

1. `WorkflowApplicationService`、typed DTO 和 characterization tests；
2. `RunConfigResolver`、revision/CAS 与 freeze/hash；
3. `ExperimentService::resolve/run` 与低层 conversation 原语等价性；
4. 唯一 RuntimeHub/EventBroker 与 Tauri adapter parity；
5. 独立 loopback `/control/v1` HTTP/SSE control plane；
6. 基础 CLI：conversation 原语 + 一步式 `experiment run`；
7. 无窗口主进程和结构化 artifact/usage/budget；
8. Harbor installed-agent adapter 和小型 Polyglot smoke；
9. 管理 services、单一 CLI skill 和安全 package pipeline；
10. 分层 benchmark 与人工 A/B；
11. GEPA-like 自动候选；
12. 条件式 DGM-like archive。

若直接从“自动安装任意 MCP/skill + 自动改进”开始，会在 runtime owner、DB/cache 一致性、package trust、verifier 独立性和预算控制尚未成立时扩大攻击面，因此不建议。
