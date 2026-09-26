# ChatSpeed `cs` CLI 第一期实施计划

## 1. Problem Statement

### 1.1 目标与交付物

第一期只交付名为 `cs` 的本地 CLI，以及供它和未来聊天软件客户端共同使用的独立 loopback HTTP/JSON + SSE workflow 控制面。`cs` 是 ChatSpeed workflow 的客户端，不是第二套 Agent runtime：CLI 创建、启动、恢复、交互、停止的任务必须与 Workflow 窗口创建的普通 workflow 使用相同的 `MainStore`、`WorkflowManager`、executor、signal validation、approval/recovery 和 durable event authority，并能立即在现有 Workflow 窗口查看和继续。

批准后，首先把本计划落盘为 `work/agent-cli-phase-1-implementation-plan.md`，之后再开始源代码修改。交付物包括：

- 当前 `src-tauri` Cargo package 中名为 `cs` 的独立 binary crate target；
- `src-tauri/src/workflow/react/client/http/` 下的独立 `/control/v1` HTTP/SSE server/protocol；
- `src-tauri/src/workflow/react/client/tauri/` 下迁移后的现有 Tauri workflow event transport；
- transport-neutral `WorkflowApplicationService` 与唯一 `WorkflowRuntimeHub`；
- CLI 的 Agent 发现、workflow 创建/运行/查询/事件/交互/停止和连接诊断命令；
- 版本化 DTO、认证/discovery、幂等、SSE 重连/backpressure、稳定 JSON/JSONL/退出码；
- colocated tests、前端契约回归和同一 session 的真实 smoke evidence。

当前 Agent workflow 只能通过 Tauri `invoke/listen` 使用。真实运行编排集中在 `src-tauri/src/commands/workflow.rs`，且直接接收 `State<Arc<TauriGateway>>`；`TauriGateway` 同时持有 Tauri `AppHandle`、事件 batching 和 session input sender。若直接为 CLI 并列一个 HTTP gateway，会产生第二套 input registry/事件路径，无法保证 CLI 与 Workflow 窗口操作同一 live executor，并违反 workflow constitution 的后端权威和单一 canonical path 约束。

### 1.2 任务类型与适用维度

- **任务类型**：跨边界 Rust 架构调整 + 本地 HTTP API + 新 Rust CLI + focused tests + 开发者文档。
- **架构/公共接口**：适用；新增版本化本地控制协议，并抽取共享 application/runtime 边界。
- **并发/性能**：适用；SSE 订阅必须有界且不得反压 executor，session input route 只能有一份。
- **安全**：适用；控制面可执行 Agent 与工具，必须 loopback-only、bearer auth、current-user discovery、日志脱敏。
- **数据库/迁移**：不适用；第一期不新增表或 schema，继续使用现有 workflow/messages/events/usage authority。
- **前端 UX**：不适用；Vue/Tauri 第一阶段不切 HTTP，不修改界面或现有 `invoke/listen` 契约。
- **部署/daemon**：部分适用；构建出 `cs` binary，但不实现 installer bundling、daemon 或 headless owner。主进程未运行时 CLI 明确失败。
- **评估与自我改进**：明确不适用；题库、evaluator、candidate/campaign、源码 worktree 实验和 promotion 全部属于第二期。

### 1.3 范围边界与非目标

本计划不实现：题库、benchmark adapter、LLM judge、verifier、compare/report；AI 自主出题、candidate/campaign、源码 worktree/分支修改或生产晋级；MCP/skill 安装、Agent/config 导入导出；headless/daemon、自动拉起主进程、跨机器监听或浏览器 CORS；experiment artifact writer、预算 admission 或数据库 schema；Vue Workflow 窗口迁到 HTTP；任何与 CLI 无关的 workflow 重构。

第一期只保留第二期可直接消费的稳定基础：普通 workflow snapshot、durable events、实时结构化事件和现有 `TaskCompleted.usage_summary`，不在本期构建评估语义。

## 2. Target Outcome and Acceptance Contract

主进程运行后，用户或 AI 可使用 `cs` 发现系统/用户 Agent，创建与 Workflow 窗口同构的普通 workflow，启动真实 Agent，订阅结构化事件，在等待状态提交 typed signal/审批，查询 authoritative snapshot/durable events，并安全停止。Workflow 窗口与 CLI 可同时观察/操作同一 session，最终收敛到相同 backend state。

HTTP 是独立、通用的 server adapter；`cs` 只负责 discovery、HTTP/SSE client、渲染和退出码。未来 Tauri 改用 HTTP 时不需要建立第三套业务语义。

### Acceptance Criteria

- **AC-1 — `cs` 可构建与发现命令**：`cargo build --bin cs` 生成名为 `cs` 的 binary；`cs --help` 展示 `doctor`、`agent`、`workflow`；增加第二 binary 后默认 desktop/Tauri 运行仍明确选择 `chatspeed`。
- **AC-2 — 安全发现与协议协商**：运行中的 ChatSpeed 在 current-user discovery file 发布 endpoint、protocol version、server instance、pid 和随机 bearer token；`cs doctor` 能认证并显示兼容性，未运行、陈旧 discovery、错误 token 或不兼容 major version 均返回稳定错误和非零退出码。
- **AC-3 — Agent 查询**：`cs agent list|get` 通过主进程查询与 `MainStore` 相同的系统/用户 Agent 集合，不直接打开数据库，并可按稳定 Agent ID 选择顶层、未禁用 Agent。
- **AC-4 — 同构 workflow 创建与启动**：`cs workflow create|start|run` 与 Tauri `create_workflow`/`workflow_start` 调用同一个 `WorkflowApplicationService`；复用相同 Agent config merge/normalization、sandbox snapshot、session key、executor/factory/manager 注册和 run loop；该 session 可立即由 Workflow 窗口列出、查看、恢复和继续。
- **AC-5 — 权威查询与控制**：`cs workflow list|get|signal|message|approve|reject|continue|stop` 通过同一 service/hub 执行；snapshot state、wait reason、pending approvals、messages 和 liveness 来自现有 backend authority；typed signal 复用现有验证，stop 在 active/wait/retry 场景保持可用。
- **AC-6 — 实时与持久事件**：`cs workflow follow|events` 分别消费 SSE 实时 `GatewayPayload` envelope 和 DB durable `WorkflowEventRecord`；SSE 使用独立 opaque cursor 与有界 ring，支持窗口内续传，过期/重启时通过 `reset_required`、snapshot 和 durable events 收敛；慢客户端不阻塞 executor/Tauri UI。
- **AC-7 — 机器稳定输出**：CLI 支持 `--output human|json|jsonl`；stdout 只含结果/事件，诊断写 stderr；error code、schema/protocol version、字符串 ID/cursor 和退出码稳定；任意 SSE 网络分片、多行 data 与断线不会破坏 JSONL。
- **AC-8 — 独立可扩展控制面与文档**：独立 `/control/v1` listener/router 不复用 Any-CORS static router 或 ccproxy auth，覆盖 meta、agents、workflows、snapshot、durable events、SSE、start/signal/stop；批准计划和最终实际协议记录于 `work/agent-cli-phase-1-implementation-plan.md`，明确第二期不在本次实现中。

### Protected Invariants

- **INV-1 — Tauri 外部契约不变**：现有 Vue 使用的 Tauri command 名、camelCase 参数/响应、`workflow://event/{session_id}` 事件名和 `GatewayPayload` 形状保持不变；Workflow 窗口不切 HTTP。
- **INV-2 — 单一 runtime/data authority**：`MainStore`、`WorkflowManager`、executor 和 session signal route 各自只有一个权威实例；CLI 不开数据库、不创建第二 run loop/input registry/lifecycle registry。
- **INV-3 — 结构化权威不降级**：lifecycle、wait、approval、recovery、completion 继续依赖 snapshot、`ExecutionContext`、structured events 和 typed signals，不从 transcript/assistant 自述推断。
- **INV-4 — constitution 场景保持**：active、所有 wait reason、等待中刷新/重启恢复、approval round-trip、completed-session hot resume、compression/context rebuild 和 stop 语义不变。
- **INV-5 — 现有 HTTP/ccproxy 不受影响**：static file server、`/save/png`、ccproxy 端口/CORS/认证和 MCP 生命周期不受 control plane 影响。
- **INV-6 — 第一期不膨胀**：不加入 evaluator、自我改进、源码 mutation、headless/daemon、管理面、数据库迁移或 Vue HTTP 切换。
- **INV-7 — secret 与本机边界**：token 不进入 URL、stdout、普通日志、workflow transcript 或文档；listener 只绑定 loopback，带 `Origin` 的浏览器请求默认拒绝，discovery 文件仅当前用户可访问。
- **INV-8 — 兼容与可回滚**：不改变 workflow DB schema/记录语义；关闭 control plane 或移除 `cs` 后桌面 workflow 仍按原契约运行，CLI 创建的 workflow 仍是普通 workflow。

## 3. Current State and Evidence

### 工程与构建

- `src-tauri/Cargo.toml` package 名为 `chatspeed`，library 输出 `rlib`，当前 desktop binary 来自 `src/main.rs`。可在同一 package 增加 `src/bin/cs.rs`，两个 binary 是独立 crate target；设置 `default-run = "chatspeed"` 避免 Tauri 开发命令歧义。
- 用户已批准新增 `clap = { version = "4.5", features = ["derive"] }`。现有 `reqwest` 已启用 `json/stream`，`tokio`、`futures-util`、`serde`、`rand`、`hex`、`lru` 可复用，无需再引入 SSE/parser/auth crate。
- `package.json` 已有 `pnpm test:workflow`；用户确认 `pnpm install` 已完成且 `pnpm tauri dev` 可运行。
- 适用规则为根 `AGENTS.md`、`src-tauri/AGENTS.md`、`src-tauri/src/workflow/react/CONSTITUTION.md`、`src-tauri/src/db/CONSTITUTION.md`。Rust production code 不得 `unwrap/expect`；开发者注释英文；外部大整数 ID string 化。

### 当前 workflow 路径

- `src/composables/workflow/useWorkflowCore.ts` 先安装 `workflow://event/{sessionId}` listener，再调用 `create_workflow`/`workflow_start`；恢复与交互调用 `workflow_signal`，停止调用 `workflow_stop`。
- `src-tauri/src/commands/workflow.rs`：`CreateWorkflowRequest`（约 1124）为当前 camelCase wire；`build_workflow_config_for_request`（约 1231）实现 Agent defaults、继承、plan/final-audit override 与 tool visibility；`create_workflow`（约 1741）生成 TSID、持久化 workflow、生成 ccproxy session key、异步标题；`get_workflow_snapshot`（约 2991）合并 durable messages、ExecutionContext、liveness 和 pending state；`workflow_start`（约 3905）负责 config sync、hot resume、signal channel、executor/factory/policy、manager 注册、run loop/cleanup；`workflow_signal`（约 4507）负责 typed signal normalization、wait validation、live injection 与 cold/hot recovery；`workflow_stop`（约 5179）传播取消并等待 reconciliation；`get_workflow_events`（约 6143）当前全量返回 durable events。
- `workflow/react/gateway.rs` 的 `Gateway` trait 已抽象 send/register/unregister/inject；当前 `TauriGateway` 同时包含 `AppHandle.emit`、100ms chunk batching、64KiB buffer、session input map 和 event sender map。
- `WorkflowExecutor::new` 已接收 `Arc<dyn Gateway>`，但只装配 `TauriSink`；`DefaultSubAgentFactory` 使用同一 gateway 创建 child executor。
- `WorkflowManager` 是唯一 lifecycle registry；兼容 `GLOBAL_SIGNAL_TX` 必须保留为 compatibility layer，不能演化为第二 authority。
- `GatewayPayload` 已覆盖 chunk、message、state/wait、approval、tool、sub-agent、`TaskCompleted { usage_summary }`；`WorkflowEventRecord.id` 是 durable DB cursor，但当前没有 after/limit 查询。
- `workflow/react/usage.rs` 与 `db/workflow_usage.rs` 已有 terminal usage summary、partial/unpriced 标记；本期只透传。

### 装配与 HTTP 边界

- `src-tauri/src/lib.rs` setup 依次注册 `MainStore`、`ChatState`、`TsidGenerator`、`TauriGateway`、`WorkflowManager`、`DefaultSubAgentFactory`，随后启动 workflow automation scheduler 和现有 HTTP server。
- `src-tauri/src/http/server.rs` static router 使用 `CorsLayer::allow_origin(Any)`，另启 ccproxy；workflow control plane 不可挂载在该 router。
- `workflow/automation/service.rs` 直接调用 `commands::workflow::workflow_start`，scheduler 从 Tauri state 组装 concrete gateway。抽取时必须改调同一 application service，但保持 automation 的 create/continuous-context 语义不变。
- Agent `id` 是稳定 string；第一期按 `agent_id` 选择，不发明新的 revision schema。

### 现有验证

`commands/workflow.rs`、`engine.rs`、`orchestrator.rs`、`sinks.rs`、`manager.rs`、`db/workflow_usage.rs` 已有 focused Rust tests/recording gateway fixture；`workflowConstitution.test.js` 等保护 stop、event listener、snapshot、approval/recovery。当前缺少 control-plane、discovery、SSE replay 和 CLI parser tests。

## 4. Recommended Solution and Architecture

### 4.1 选定方案

采用“共享 application service + 唯一 runtime hub + 两个 transport adapter”的窄垂直切片。

不采用 HTTP 监听 Tauri event 的临时桥，因为它仍依赖 `AppHandle` 且不能统一 input ownership；不采用并列 `HttpGateway`，因为 executor 只能绑定其一并会复制 input map；不允许 CLI 直开 DB 或构造 executor，因为会绕过 ConfigCache、ToolManager、MCP、WorkflowManager 和 recovery rules。

```text
Vue/Tauri invoke ──> thin Tauri commands ─┐
                                          ├─> WorkflowApplicationService
HTTP /control/v1 ──> thin Axum handlers ──┘           │
                                                       ▼
                                         WorkflowManager + WorkflowRuntimeHub
                                                       │
                                              existing executor/run_loop
                                                       │
                           ┌───────────────────────────┴────────────────────┐
                           ▼                                                ▼
              Tauri event adapter                                  SSE broker/ring
          workflow://event/{session}                         /control/v1/.../stream
```

### 4.2 `WorkflowApplicationService`

位于 `src-tauri/src/workflow/react/application.rs`，持有显式依赖（`MainStore`、`ChatState`、`TsidGenerator`、`WorkflowRuntimeHub`、`SubAgentFactory`、`WorkflowManager`、`app_data_dir`），提供 Agent list/get、workflow list/create/snapshot/start/signal/stop/events-after。把 create/start/signal/stop/snapshot 所需 orchestration helpers 从 `commands/workflow.rs` 移入或收敛到该模块；Tauri wrapper 原名/wire 不变，只拆 `State` 并委托。HTTP 将 snake_case DTO 转成相同 service request。`workflow_signal` 的所有 recovery-to-start 分支调用 service canonical start，不能回调 Tauri command。

### 4.3 `WorkflowRuntimeHub`

位于 `src-tauri/src/workflow/react/client/hub.rs`，是唯一 `Gateway` 实现和 session input sender owner：

- `register/unregister/inject` 维护唯一 map；
- `send` 保持原 Tauri delivery/batching，再发布 versioned SSE envelope；
- 每 session 使用有界 ring + `tokio::broadcast` 或等价非阻塞 fan-out；lag 触发 `reset_required`，不反压 executor；
- stream cursor 为 `${server_instance_id}:${monotonic_sequence}` 的 opaque string，只用于当前实例短期 replay；durable DB event ID 单独 string，二者禁止混用；
- hub 不持有 lifecycle status，liveness 仍查询 `WorkflowManager`。

### 4.4 Tauri adapter

把 `TauriGateway` 机械迁移到 `workflow/react/client/tauri/gateway.rs`，保留 `AppHandle.emit`、事件名、chunk/reasoning batching 和 flush；input map 迁入 hub 后，Tauri adapter 只做输出 transport。`commands/workflow.rs` 保留 `#[tauri::command]` wrappers，核心编排下沉 service。

### 4.5 HTTP adapter

位于 `workflow/react/client/http/`，不依赖现有 static/ccproxy router：

- `server.rs`：绑定 `127.0.0.1:0`，router/body limits、启动/关闭；
- `auth.rs`：bearer middleware、Origin/URL-token rejection、stable error envelope；
- `discovery.rs`：256-bit token、server instance，原子写 `${CHATSPEED_HOME:-~/.chatspeed}/runtime/control-plane-v1.json`；Unix dir/file 0700/0600；启动覆盖 stale，关闭只删除匹配 instance；
- `dto.rs`：HTTP canonical snake_case v1，Tauri 保留 camelCase；IDs/cursors string；
- `sse.rs`：envelope、Last-Event-ID、ring replay、lag/reset、keepalive。

最小 endpoints：

```text
GET  /control/v1/meta
GET  /control/v1/agents
GET  /control/v1/agents/{agent_id}
GET  /control/v1/workflows
POST /control/v1/workflows
GET  /control/v1/workflows/{session_id}
POST /control/v1/workflows/{session_id}:start
POST /control/v1/workflows/{session_id}:signal
POST /control/v1/workflows/{session_id}:stop
GET  /control/v1/workflows/{session_id}/events?after=<durable-id>&limit=<n>
GET  /control/v1/workflows/{session_id}/stream
```

Mutation 支持 `Idempotency-Key`。当前 server instance 内，用有界 LRU 保存 key + canonical body hash + response；同 key/同 body 返回原 response，同 key/不同 body 为 `CONFLICT`。不声称跨主进程重启幂等，该限制写入文档。

### 4.6 `cs` CLI

在同一 package 增加 `src/bin/cs.rs` 及子模块。HTTP server 仍属于 `chatspeed_lib`，`cs` 是独立 binary crate target：它不导入 workflow runtime、不打开 DB，只读取 discovery 后使用 `reqwest`。

```text
cs doctor
cs agent list
cs agent get <agent-id>
cs workflow list
cs workflow create --agent <id> [--prompt/--prompt-file] [--allowed-path ...]
cs workflow start <session-id> [--prompt/--prompt-file] [--follow]
cs workflow run --agent <id> --prompt/--prompt-file [--allowed-path ...] [--follow]
cs workflow get <session-id>
cs workflow events <session-id> [--after <id>] [--follow]
cs workflow signal <session-id> (--json <json> | --file <path>)
cs workflow message <session-id> (--text <text> | --file <path>)
cs workflow approve <session-id> --tool-call-id <id> [--all]
cs workflow reject <session-id> --tool-call-id <id> [--message <text>]
cs workflow continue <session-id>
cs workflow stop <session-id>
```

全局参数：`--output human|json|jsonl`、`--discovery-file`、`--lang`。`workflow run` 只顺序调用相同 create/start endpoint；start 失败时输出已创建的 `session_id`，不删除 workflow。CLI 默认断开或 Ctrl-C 只关闭 follow，不停止 workflow。

使用用户已批准的 Clap derive；SSE 以现有 `reqwest` stream + 自有小型增量 framing parser 完成。human 文案复用 Rust i18n 的 en/zh-Hans/zh-Hant；JSON 字段/error code 不本地化。

### 4.7 错误、恢复、迁移和回滚

- service 产生稳定 domain error；Tauri 保持现有 string error，HTTP 映射 status/code，CLI 映射 exit code。
- HTTP 不自动 retry mutation；CLI 仅可带同一 idempotency key 重试一次 transport failure。
- SSE 只保证当前实例 ring window 内 replay；`Chunk` 等非持久事件不伪装为 durable history。
- cursor stale/instance mismatch 返回 `reset_required`；CLI 拉 snapshot + durable events 后重订阅。terminal 判断来自 structured state/`TaskCompleted`，不解析 assistant 文本。
- control-plane 启动失败记录脱敏错误，但不阻断 desktop/static/ccproxy。
- 无 DB migration；session ID 仍是 TSID string；第一期 `run_id == session_id`。
- `cs` 通过 `cargo build --bin cs` 或 `cargo install --path src-tauri --bin cs` 提供开发版，不修改 installer bundle。
- 回滚不需迁移数据；已由 CLI 创建的 session 是普通 workflow。若 hub parity 失败，回退 hub，不保留双 gateway 临时主路径。

## 5. Decision and Uncertainty Ledger

### Confirmed Decisions

- **D-1**：分两期；本计划仅处理 CLI/control plane，评估与源码自改为第二期。
- **D-2**：CLI 名为 `cs`；在当前 Cargo package 增加独立 binary crate target并设置 desktop `default-run`。
- **D-3**：用户已批准新增 Clap derive；不新增其他 CLI/SSE 依赖。
- **D-4**：HTTP server/protocol 独立于 CLI crate，位于 `workflow/react/client/http`，可供未来聊天客户端或 Tauri HTTP 迁移复用。
- **D-5**：Tauri 与 HTTP 汇合到同一个 application service/runtime hub；禁止第二 workflow/session/input/DB authority。
- **D-6**：第一期主进程必须已运行；不实现 headless/daemon/auto-launch。
- **D-7**：HTTP wire snake_case，Tauri wire保留camelCase；转换仅在各adapter边界。
- **D-8**：只暴露现有 structured events/snapshot/usage，不创建评估 artifact/candidate/evaluator。
- **D-9**：control plane 不合并到 Any-CORS static router 或 ccproxy。

### Assumptions

- **A-1**：用户提供的环境状态有效：依赖已安装且 `pnpm tauri dev` 当前可运行；实施前只做窄 freshness check。
- **A-2**：Cargo build/install 足够作为第一期分发；若要求 installer 自动包含 `cs`，必须另行确认发布/签名范围。
- **A-3**：真实 smoke 时存在配置完整的顶层 Agent/模型；若没有，mock tests和不发 LLM 的真实 discovery/create/get 仍执行，付费模型段如实标记未验证。
- **A-4**：`${CHATSPEED_HOME:-~/.chatspeed}/runtime` 可作为 discovery 根；server/CLI 必须复用同一算法并做跨平台测试。若 token 文件或 loopback TCP 被企业策略禁止，停止询问，不自行改用其他 IPC。

### Open Questions / Blockers

无未决用户决策，`unresolved_blockers: []`。

### Stop Conditions

若发现必须改变 Tauri command/event/payload 或用户可见 workflow 行为；新增/迁移 DB schema；引入除 Clap 外的新依赖；实现 daemon/headless、installer、LAN/CORS；开放 Agent/MCP/skill/config mutation；把评估或源码 worktree 实验带入第一期；或无法建立唯一 input/lifecycle authority，则停止相应 unit 并询问用户。

## 6. Execution Map

### U-1: 固化 Tauri parity 并机械迁移 Tauri transport

- **Purpose**：先将批准计划落盘到 work/，锁定外部契约，再把 Tauri gateway 放入用户指定目录。
- **Covers**：AC-4、AC-5、AC-8；INV-1、INV-3、INV-4、INV-8。
- **Confirmed Targets**：`work/agent-cli-phase-1-implementation-plan.md`；`workflow/react/gateway.rs`、`workflow/react/mod.rs`；新增 `workflow/react/client/{mod.rs,tauri/mod.rs,tauri/gateway.rs}`；`lib.rs`、`commands/workflow.rs`、`workflowConstitution.test.js`。
- **Candidate Targets**：仅在 import 编译需要时调整 automation 相关 use path。
- **Preconditions**：实施前检查目标文件 diff，保护用户改动。
- **Implementation Path**：原样落盘批准计划；补 create/start/signal/stop 参数、event name、chunk flush、register/unregister、snapshot field characterization；保留 `Gateway` trait，将 `TauriGateway` 机械迁移；更新 imports/构造。
- **Expected Result**：Tauri workflow 完全按原路径工作，Tauri-specific code 位于 `client/tauri`。
- **Verification**：V-1、V-2、V-6。
- **Allowed Local Decisions**：`pub(crate)` re-export位置和test helper命名。
- **Stop Conditions**：迁移需要改变 event name/payload/buffering/command signature。
- **Risks / Edge Cases**：漏改 concrete type import；characterization 不应固化内部文件位置。

### U-2: 建立唯一 `WorkflowRuntimeHub`

- **Purpose**：将 session input ownership 提升到共享 runtime boundary，并建立非阻塞 SSE event source。
- **Covers**：AC-4、AC-5、AC-6；INV-1、INV-2、INV-3、INV-4、INV-7、INV-8。
- **Confirmed Targets**：新增 `client/hub.rs`；`gateway.rs`、Tauri adapter、`engine.rs`、`sinks.rs`、`orchestrator.rs`、`manager.rs`、`lib.rs`、`commands/workflow.rs`。
- **Candidate Targets**：所有 freshness search 命中的 `Arc<TauriGateway>` consumers。
- **Preconditions**：U-1 parity tests通过。
- **Implementation Path**：定义 sink/broker envelope/cursor；将唯一 input map和source日志迁入hub；让primary/child executor和command injection共享同一hub；删除Tauri重复input map；实现per-session bounded ring+broadcast；测试multi-subscriber、lag/reset、Tauri parity、cleanup。
- **Expected Result**：每 session 只有一个 input route；Tauri/SSE共源，慢CLI不影响runtime/UI。
- **Verification**：V-2、V-3、V-6。
- **Allowed Local Decisions**：有界容量、内部channel类型可按测试选择并写入文档。
- **Stop Conditions**：hub必须成为第二lifecycle registry，或无法保持Tauri event order/batching。
- **Risks / Edge Cases**：cleanup race、completed hot resume sender replacement、parent/child bridge、lag与sequence overflow。

### U-3: 抽取 `WorkflowApplicationService`

- **Purpose**：让 Tauri/HTTP/automation 复用真实编排，不复制 command 状态机。
- **Covers**：AC-3、AC-4、AC-5、AC-6；INV-1、INV-2、INV-3、INV-4、INV-6、INV-8。
- **Confirmed Targets**：新增 `workflow/react/application.rs`；`mod.rs`、`commands/{workflow,agent,workflow_automation}.rs`、`workflow/automation/{service,scheduler}.rs`、`lib.rs`、`db/workflow.rs`。
- **Candidate Targets**：helper的colocated tests随helper移动，或保留wrapper tests。
- **Preconditions**：U-2 hub ownership通过。
- **Implementation Path**：定义service deps/domain request/response/error并使用`app_data_dir: PathBuf`；抽Agent/workflow operations及原config/title/recovery/compression/cleanup helpers；Tauri wrapper保留wire；signal recovery改调service start；automation改调同一start但保留自身create语义；按依赖顺序manage service；新增bounded events-after查询。
- **Expected Result**：CLI/HTTP/UI无法走不同runtime，automation与Tauri外部行为不变。
- **Verification**：V-2、V-3、V-6。
- **Allowed Local Decisions**：可按config/lifecycle/projection拆service子模块；不得形成平行路径。
- **Stop Conditions**：需要改变config merge、automation context、approval/recovery或DB schema。
- **Risks / Edge Cases**：helper依赖面大，重点保护hot resume、manual compression、cleanup、title generation。

### U-4: 实现安全的 HTTP/JSON + SSE control plane

- **Purpose**：把共享service/hub暴露为通用本地协议，不污染旧HTTP/ccproxy。
- **Covers**：AC-2、AC-3、AC-4、AC-5、AC-6、AC-8；INV-2、INV-3、INV-5、INV-7、INV-8。
- **Confirmed Targets**：新增 `client/http/{mod,server,auth,discovery,dto,sse}.rs`；`client/mod.rs`、`lib.rs`、`db/workflow.rs`。
- **Candidate Targets**：纯 discovery path helper可放 `client/discovery.rs` 供 server/CLI共享；绝不放旧router。
- **Preconditions**：U-3 parity通过。
- **Implementation Path**：定义v1 DTO/envelope/error；实现127.0.0.1:0 listener、token/instance/discovery、权限与脱敏；auth/Origin/query-token/body limits；handler薄委托service；有界instance-local idempotency；SSE replay/live/reset/keepalive；server failure降级而非阻断desktop。
- **Expected Result**：本地第三方客户端安全使用同一runtime，旧HTTP不变。
- **Verification**：V-3、V-4、V-7。
- **Allowed Local Decisions**：HTTP status精细映射、LRU容量和keepalive周期，在安全/测试/文档约束内选择。
- **Stop Conditions**：需要LAN/CORS、复用ccproxy token/router、token日志或新依赖。
- **Risks / Edge Cases**：stale discovery、并发启动、权限、partial mutation、SSE UTF-8/chunk、lag、instance cursor冲突。

### U-5: 实现 `cs` 客户端

- **Purpose**：提供 AI/用户可稳定编排的纯 HTTP/SSE CLI。
- **Covers**：AC-1、AC-2、AC-3、AC-4、AC-5、AC-6、AC-7；INV-2、INV-3、INV-6、INV-7、INV-8。
- **Confirmed Targets**：`Cargo.toml`、`Cargo.lock`；新增 `src/bin/cs.rs` 和 `src/bin/cs/{args,client,discovery,error,output,sse}.rs`；Rust i18n三种locale。
- **Candidate Targets**：只有在不会引入Tauri/runtime链接时才共享纯DTO；否则用CLI DTO+golden防漂移。
- **Preconditions**：U-4 schema稳定，Clap已批准。
- **Implementation Path**：配置两个bin/default-run和Clap；实现命令/参数互斥；discovery precedence与meta validation；agent/workflow calls与run create→start；idempotency；健壮增量SSE parser与reset收敛；localized human、stable JSON/JSONL、stdout/stderr和exit codes；Ctrl-C不stop。
- **Expected Result**：AI可用稳定machine mode控制真实Agent，用户可human mode交互。
- **Verification**：V-1、V-5、V-7。
- **Allowed Local Decisions**：human表格和内部module/client trait命名；machine contract不可随意改。
- **Stop Conditions**：需要CLI开DB/起executor、新依赖、daemon或hardcoded secret。
- **Risks / Edge Cases**：quoting、stdin/file、UTF-8、broken pipe、SSE半帧、JSONL污染、断线误停。

### U-6: 文档、回归和同一 session smoke

- **Purpose**：让 work/文档反映最终实现，并用多层证据证明第一期完成。
- **Covers**：全部 AC-1..AC-8 与 INV-1..INV-8。
- **Confirmed Targets**：`work/agent-cli-phase-1-implementation-plan.md`、所有新增模块 tests、`workflowConstitution.test.js`。
- **Candidate Targets**：不修改已有52K主设计；新一期文档仅链接它，避免混写。
- **Preconditions**：U-1至U-5完成。
- **Implementation Path**：同步build/install、discovery/security、commands、HTTP/SSE、reconnect/idempotency、exit codes和phase-2 defer；跑fmt/check/clippy、focused Rust/CLI/router和frontend tests；启动Tauri，执行doctor/agent/create/get及可用时一个最小真实run；UI/CLI交叉观察/signal/stop、断线重连、secret审计。
- **Expected Result**：第一期功能、兼容、安全和文档有证据，第二期未混入。
- **Verification**：V-1至V-8。
- **Allowed Local Decisions**：选择可用顶层Agent和临时目录；禁止生产workspace写入测试。
- **Stop Conditions**：smoke会产生不可控费用或非临时写入，或UI/CLI出现authority分叉。
- **Risks / Edge Cases**：模型随机性不作为10次100%门槛，只验证runtime/protocol同构与structured terminal state。

## 7. Verification Strategy and Acceptance Matrix

- **V-1 — 构建与静态质量**：覆盖 AC-1、AC-7、INV-1、INV-6、INV-8。运行 `cargo fmt --all -- --check`、`cargo check --bin chatspeed --bin cs`、两个bin clippy和`cs --help`；证据为构建、命令树、default target和无新增warning。
- **V-2 — Tauri/application parity**：覆盖 AC-4、AC-5、INV-1、INV-2、INV-3、INV-4、INV-8。运行commands/workflow、application、engine、orchestrator、manager、sinks、automation focused tests；比较同构request，覆盖active/wait/approval/resume/stop/compression；证据为state/config/events相同且单manager/sender/executor。
- **V-3 — Hub/SSE 并发**：覆盖 AC-5、AC-6、INV-1、INV-2、INV-3、INV-4、INV-7。Tokio tests覆盖replace/unregister、双subscriber、Tauri sink、ring replay、stale/lag、parent-child、cleanup；证据为不反压、明确reset、Tauri顺序相同、无孤儿sender。
- **V-4 — HTTP安全/协议/幂等**：覆盖 AC-2至AC-6、AC-8、INV-2、INV-5、INV-7、INV-8。Axum in-process+real listener tests覆盖token/Origin/loopback/body/version/string IDs/idempotency/events/SSE/discovery；证据为未授权全拒绝、无双重effect、旧router隔离、无token泄露。
- **V-5 — CLI mock E2E**：覆盖 AC-1至AC-7、INV-2、INV-3、INV-6、INV-7。`cargo test --bin cs`以mock server覆盖所有commands、golden、stdout/stderr、exit codes、partial start、随机SSE分片/重连/Ctrl-C；证据为JSONL逐行可解析且断开不stop。
- **V-6 — 前端契约回归**：覆盖 AC-4、AC-5、INV-1、INV-3、INV-4。运行 `pnpm test:workflow`，必要时`pnpm build`；证据为Vue仍用原IPC/event。
- **V-7 — 真实主进程/CLI smoke**：覆盖 AC-2至AC-6、INV-1、INV-2、INV-5、INV-7、INV-8。运行Tauri、doctor、agent、临时create/get及可用时最小run/follow，UI/CLI交叉signal/stop/reconnect；证据为同session ID和同state/terminal，断开不终止且无secret。
- **V-8 — 文档/范围审计**：覆盖 AC-8、INV-5、INV-6、INV-7、INV-8。对照work文档与help/routes/DTO/tests并搜索越界模块；证据为文档一致且无phase-2/headless/CLI DB代码。

| Requirement | Units | Verification |
|---|---|---|
| AC-1 | U-5,U-6 | V-1,V-5 |
| AC-2 | U-4,U-5,U-6 | V-4,V-5,V-7 |
| AC-3 | U-3,U-4,U-5,U-6 | V-4,V-5,V-7 |
| AC-4 | U-1..U-6 | V-2,V-4,V-5,V-6,V-7 |
| AC-5 | U-1..U-6 | V-2,V-3,V-4,V-5,V-6,V-7 |
| AC-6 | U-2..U-6 | V-3,V-4,V-5,V-7 |
| AC-7 | U-5,U-6 | V-1,V-5 |
| AC-8 | U-1,U-4,U-6 | V-4,V-8 |
| INV-1 | U-1,U-2,U-3,U-6 | V-1,V-2,V-3,V-6,V-7 |
| INV-2 | U-2..U-6 | V-2,V-3,V-4,V-5,V-7 |
| INV-3 | U-1..U-6 | V-2,V-3,V-5,V-6,V-7 |
| INV-4 | U-1,U-2,U-3,U-6 | V-2,V-3,V-6,V-7 |
| INV-5 | U-4,U-6 | V-4,V-7,V-8 |
| INV-6 | U-3,U-5,U-6 | V-1,V-5,V-8 |
| INV-7 | U-2,U-4,U-5,U-6 | V-3,V-4,V-5,V-7,V-8 |
| INV-8 | U-1..U-6 | V-1,V-2,V-4,V-7,V-8 |

## 8. Risk, Migration, and Rollback

1. `workflow_start/signal/snapshot` 私有helper与recovery分支很多：严格按 characterization → hub → service → HTTP → CLI 次序，每unit保持可编译。
2. hub/dispatcher双fan-out会重复事件：最终只允许 `engine -> existing dispatcher sink -> hub -> Tauri + SSE`；sequence test证明。
3. completed hot resume、stop和延迟cleanup会替换sender：保留带source注册日志并测试marker/race。
4. SSE cursor不能替代durable ID：字段、DTO、文档和恢复逻辑强制分开。
5. auth/discovery/Origin/body limit/idempotency/脱敏与router同unit落地，不允许先开放未认证listener。
6. Unix权限显式0700/0600；Windows使用current-user profile runtime目录并验证继承ACL；无法确认时停止，不降级公开token。
7. 第一阶段只承诺Cargo build/install，不承诺Tauri installer包含CLI。
8. 真实模型smoke只有一个有界临时任务；无模型/API则明确唯一未验证项，不扩大范围配置secret。

无数据库迁移。Tauri wire 零变化；HTTP v1为新增接口。`cs` JSON/JSONL自v1起受schema_version保护，human输出不作为machine contract。control-plane failure/disable不影响desktop；stale discovery下次启动覆盖并由instance/meta拒绝。回滚不需数据恢复，CLI workflow保持普通workflow。

## 9. Handoff Checklist

- [ ] 首先检查 gateway、commands/workflow、lib、automation、Cargo和work目标文件当前diff，保护用户改动。
- [ ] 将本批准计划原样落盘到 `work/agent-cli-phase-1-implementation-plan.md`，再执行U-1 characterization与机械迁移；不要先写HTTP。
- [ ] 使用CodeGraph优先、native grep补充，freshness确认所有 `Arc<TauriGateway>`、register/unregister、workflow_start内部调用和automation caller。
- [ ] 先运行现有 `pnpm test:workflow` 和最窄Rust baseline。
- [ ] U-2完成前不得创建第二HttpGateway/input map；U-3完成前HTTP不得调用command wrapper或复制helper。
- [ ] U-4必须连同auth/discovery/security tests完成。
- [ ] U-5只做client；确认无SQLite/MainStore/executor依赖。
- [ ] 命中stop condition时使用`ask_user`，不得扩大到第二期或新依赖。
- [ ] 最终逐一核对AC-1..AC-8、INV-1..INV-8、U-1..U-6、V-1..V-8；todos终态，报告实际/跳过验证和风险。

## 10. Plan Readiness Gate

- [x] 每个用户目标至少由一个AC覆盖；范围严格限定第一期。
- [x] 每个AC/INV至少映射一个U与一个V，ID唯一且矩阵一致。
- [x] Tauri契约、唯一authority、structured recovery、安全和范围均有INV。
- [x] confirmed targets来自已检查源码；candidate targets只做窄freshness确认。
- [x] U-1→U-6依赖有序无环。
- [x] verification覆盖行为、并发、安全、协议、CLI输出、前端回归和真实同session smoke，不只编译。
- [x] DB、frontend UX、daemon、评估等不适用维度已说明。
- [x] stop conditions覆盖public contract、schema、安全、依赖、remote access、第二期扩scope和authority分叉。
- [x] 实施代理可从窄freshness check与U-1开始，无需重复广泛调查。
- [x] `acceptance_contract`与本文AC/INV/U/V一致，且 `unresolved_blockers: []`。

---

## 11. Implementation Record (Phase 1 as built)

本节记录第一期实际交付的实现与协议，与上文批准计划对应。批准计划正文保持原样，本节仅追加事实记录。

### 11.1 代码结构（实际落点）

- `src-tauri/src/workflow/react/gateway.rs`：仅保留 transport-neutral `Gateway` trait（send / register_session_input / unregister_session_input / inject_input）。
- `src-tauri/src/workflow/react/client/tauri/gateway.rs`：`TauriGateway`（输出-only transport）。保留 `workflow://event/{session_id}` 事件名、100ms chunk/reasoning batching、64KiB buffer、code-block 检测、控制消息 flush、5ms yield、channel 关闭 flush。batching 状态机抽取为 `EventBatcher`（无 `AppHandle` 可测）；输出经 `EventSink` trait（生产为 `TauriEventSink(AppHandle)`，测试为 recording sink）。
- `src-tauri/src/workflow/react/client/hub.rs`：唯一 `WorkflowRuntimeHub`（生产唯一 `Gateway` 实现）+ `SessionInputRegistry`（唯一 session input sender map）+ `SessionEventBroker`（per-session 有界 ring 1024 + `tokio::broadcast` 256，非阻塞 fan-out）。`StreamEnvelope { schema_version=1, server_instance_id, sequence(全局单调), session_id, payload }`；cursor 为 `${server_instance_id}:${sequence}` opaque string，与 durable DB event ID 严格分离。stale/foreign/越窗 cursor → `SubscribeError::ResetRequired`；lagging subscriber → 显式 lag → reset。
- `src-tauri/src/workflow/react/application.rs`：`WorkflowApplicationService`（deps: MainStore/ChatState/TsidGenerator/Hub/SubAgentFactory/WorkflowManager/app_data_dir）+ `ApplicationError{kind,message}`（NotFound/InvalidInput/Conflict/State/Gateway/Internal；Display 仅 message，Tauri string error 保持逐字节不变）+ 领域请求 `WorkflowCreateRequest`/`WorkflowStartRequest`/`WorkflowEventsQuery`（snake_case serde）。
- 编排 core（`create_workflow_core`/`list_workflows_core`/`get_workflow_snapshot_core`/`workflow_start_core`/`workflow_signal_core`/`workflow_stop_core`/`get_workflow_events_core`）位于 `commands/workflow.rs`，`pub(crate)`，仅经 service 调用；Tauri `#[tauri::command]` wrapper 保留原名/wire，仅拆 State 委托 service。`workflow_signal` 的全部 recovery-to-start 分支改调 `workflow_start_core`；`workflow_approve_plan` 经 `workflow_signal_core`；automation `run_automation_now` 改调 `workflow_start_core`（create 语义不变）。
- `src-tauri/src/db/workflow.rs`：新增 `list_workflow_events_after(session_id, after, limit)`（默认 200，上限 `WORKFLOW_EVENTS_MAX_LIMIT=500`，ASC，`id > after`）。无 schema 变更。
- `src-tauri/src/workflow/react/client/http/`：独立 `/control/v1` control plane。
  - `server.rs`：绑定 `127.0.0.1:0`；`DefaultBodyLimit` 1MiB；有界 instance-local 幂等 LRU（1024，key+body hash+response；同 key 同 body replay，异 body 409 `idempotency_key_conflict`）；handle 存入 `ACTIVE_HANDLE`，`request_shutdown()` 供应用退出时 graceful shutdown + discovery 清理；启动失败仅记录脱敏日志，不阻断 desktop。
  - `auth.rs`：bearer 常量时间比较；带 `Origin` 请求 403 `origin_forbidden`；URL query token 400 `token_in_url_forbidden`；缺失/错误 token 401 `unauthorized`；错误 envelope `{"error":{"code","message"}}`。
  - `discovery.rs`：`${CHATSPEED_HOME:-~/.chatspeed}/runtime/control-plane-v1.json`，原子写（tmp+rename），Unix dir 0700 / file 0600；关闭时仅删除匹配 instance 的文档。
  - `dto.rs`：snake_case v1；`to_snake_case_keys` 在 HTTP 边界统一转换 Tauri-shaped 结构；错误映射 NotFound→404/InvalidInput→400/Conflict·State·Gateway→409/Internal→500。
  - `sse.rs`：`Last-Event-ID`（或 `?cursor=`）ring replay；`event: reset_required`（reason: instance_mismatch/malformed_cursor/unknown_sequence/replay_window_exceeded/replay_window_unavailable/subscriber_lagged）；15s keepalive comment；lag → reset 后结束流。
- `src-tauri/src/bin/cs.rs` + `src/bin/cs/{args,client,discovery,error,output,sse}.rs`：`cs` 独立 binary target（`default-run = "chatspeed"`）。Clap derive 命令树：`doctor`、`agent list|get`、`workflow list|create|start|run|get|events|signal|message|approve|reject|continue|stop`；全局 `--output human|json|jsonl`、`--discovery-file`、`--lang`。CLI 不导入 workflow runtime、不开 DB；mutation 携带 UUID idempotency key，transport 失败同 key 重试一次；SSE 自研增量 framing parser（分片/CRLF/多行 data/keepalive 安全）；follow 断线重连一次（带 Last-Event-ID），`reset_required` 时拉 snapshot+durable events 收敛后重订阅；Ctrl-C/断开仅退出 follow，不停止 workflow；terminal 判定仅来自 structured `task_completed`/terminal state。human 文案经 rust-i18n（en/zh-Hans/zh-Hant，`cs.*` 键）；JSON/JSONL 与 error code 不本地化。
- `src-tauri/i18n/{en,zh-Hans,zh-Hant}.yml`：新增 `cs` 段（键按字母序插入）。

### 11.2 实际 HTTP 协议（v1）

```text
GET  /control/v1/meta                              → {service, protocol_version:"1", schema_version:1, server_instance_id, pid}
GET  /control/v1/agents                            → [Agent]（snake_case）
GET  /control/v1/agents/{agent_id}                 → Agent | 404 not_found
GET  /control/v1/workflows                         → [Workflow]
POST /control/v1/workflows                         → 201 {session_id}（支持 Idempotency-Key）
GET  /control/v1/workflows/{session_id}            → snapshot（snake_case，含 hasLiveSession 等）
POST /control/v1/workflows/{session_id}/start      → {session_id}（body: WorkflowStartRequest snake_case）
POST /control/v1/workflows/{session_id}/signal     → {session_id, result}（body: 原样 typed signal JSON）
POST /control/v1/workflows/{session_id}/stop       → {session_id, stopped}
GET  /control/v1/workflows/{session_id}/events?after=<durable-id>&limit=<n> → [WorkflowEventRecord]
GET  /control/v1/workflows/{session_id}/stream     → SSE（Last-Event-ID replay / reset_required / keepalive）
```

说明：计划中的 `:start`/`:signal`/`:stop` verb 后缀实现为 `/start`、`/signal`、`/stop` 子路径（axum matchit 对段内 `:` 的支持不保证，子路径语义等价且更稳健）。这是对计划 4.5 节 endpoint 拼写的唯一偏差，不改变协议能力。

### 11.3 CLI 退出码（稳定契约）

- 0 成功；1 服务端/I/O 错误；2 用法错误；3 未运行/discovery 缺失或过期/连接失败；4 认证失败；5 协议版本不兼容。

### 11.4 验证证据（实际执行）

- V-1：`cargo fmt --all` 通过；`cargo check --bin chatspeed --bin cs` 0 error 0 warning；`cargo clippy` 新增代码 0 warning（lib 存量 402 条为基线噪音，未触碰）；`cs --help` 展示 doctor/agent/workflow 与退出码说明；`default-run = "chatspeed"`。
- V-2：`commands::workflow` 64 通过；`workflow::automation` 18 通过；`db::workflow_usage` 5 通过。
- V-3：`workflow::react::client` 29 通过（batcher/registry/broker：单调序列、cursor replay、双订阅者、窗口越界 reset、lag 非阻塞、session 隔离、register/replace/unregister、closed-channel 错误）。
- V-4：`workflow::react::client::http` 11 通过（真实 ephemeral listener + reqwest：missing/wrong token、Origin 拒绝、URL token 拒绝、meta 协议字段、agent snake_case 与 404 envelope、create 幂等 replay/无双重创建/异 body 409、events after+limit 与非法参数 400、SSE replay+reset、discovery 0700/0600 与关闭清理）。
- V-5：`cargo test --bin cs` 9 通过（discovery 优先级/协议 major 校验/缺失文件、SSE 分片/CRLF/多行/keepalive、human 渲染）；真实运行 `cs doctor`/`cs agent list`（无主进程）→ stderr 稳定错误 + exit 3。
- V-6：`pnpm test:workflow` 62 通过 0 失败（Tauri command/event 契约未变）。
- V-7：**已执行（2026-09-14 补测）**。在真实 `pnpm tauri dev` 环境完成端到端 smoke：`cs doctor` 连通控制面、`cs agent list`、`cs workflow run --agent builtin:coding --model cs@free:ds-v4-flash --follow` 全链路成功（SSE 流式事件、模型覆盖持久化、恰好 1 条初始 user 消息、终态 completed、10 条持久化事件含完整生命周期），并实测 404/400 错误路径。测试中发现并修复 CLI 缺少会话级模型/终审/计划模式参数的 parity 缺口（新增 `--model`/`--agent-config`/`--final-audit`/`--plan`）。详见 `work/agent-cli-phase-1-smoke-test.md`。付费模型 smoke 未执行（按用户要求使用 free 分组模型）。
- V-8：范围审计通过：`src/bin/cs*` 无 MainStore/sqlite/executor/WorkflowManager 引用；`src/http`、`src/ccproxy` 零改动；无 evaluator/candidate/campaign/headless/daemon 代码；无 DB schema 变更。
- 自审（完整 diff 复查）：发现并修复两个问题——(1) `cs workflow start` 此前发送 `agent_id: null`，会被 HTTP 层以 400 拒绝（runtime 确实按 agent_id 加载 Agent 配置）；现 `start` 子命令从权威 snapshot 解析 `workflow.agent_id`，`run` 子命令直接使用 `--agent` 值；(2) 幂等 replay 响应丢失 `application/json` Content-Type，已通过 `replay_response` 修复。修复后全部相关测试复跑通过（client 29 / http 11 / cs 9 / commands::workflow 64 / automation 18），双 bin check 0 warning。
- 终审修复（final review 两项 major）：
  1. **CLI run/start 丢失 initial_prompt**：`workflow run` 此前将已解析的 prompt 只写入 create 的 user_query，start 时发送 `initial_prompt: null`，导致 runtime 不追加任何初始用户消息（与 Tauri create→start 序列不同构）。修复：`run` 将解析后的 prompt 作为 `initial_prompt` 传入 start；`start` 未显式给 `--prompt` 时从权威 snapshot 读取存储的 `user_query` 作为 initial_prompt（存储为空则不注入，兼容等待态恢复）。解析逻辑抽取为纯函数 `resolve_start_inputs` 并配 5 个单元测试（run 直传、start 回退存储 query、显式 prompt 覆盖、空 query 不注入、缺 agent 报协议错误）。
  2. **幂等并发竞态**：原实现"查锁→释放→执行→回写"，并发同 key 同 body 请求可双双未命中并重复执行。修复：改为原子预约——同 key 首个请求插入 `InFlight`（含 body hash + broadcast 通道）后执行；并发同 key 同 body 请求订阅该通道等待唯一结果；同 key 异 body 立即 409；执行完成后写入 `Done` 供后续顺序 replay。新增并发重复 create 测试（5 个并发同 key 请求 → 全部 201 且同一 session_id、仅持久化 1 个 workflow）。
  - 修复后验证：http 12（含新并发测试）/ cs 14（含 5 个新 resolve 测试）/ client 30 / commands::workflow 64 / automation 18 / engine 60 / orchestrator 19 / manager 10 / sinks 4 / workflow_usage 5 全部通过；`pnpm test:workflow` 62 通过；fmt/check 双 bin 0 warning；新增代码 clippy 干净。
- 二审修复（re-review 一项 major：InFlight 预约仍存在两个竞态——broadcast 晚订阅者收不到已发布结果会挂起/误报 500；有界 LRU 可在执行中驱逐 InFlight 条目导致重复执行）。重构为 `IdempotencyTracker`：
  1. **in-flight 预约移出有界缓存**：独立 `HashMap` 保存 `InFlightReservation`，执行期间不可能被驱逐；完成后才移入有界 done LRU 供顺序 replay。
  2. **watch 通道保留结果**：`watch::Sender<Option<IdempotencyDone>>` 发布后保留当前值，任何时刻（发布前或发布后）订阅的等待者都能读到结果——消除"发布后订阅"竞态；执行失败/panic 由 `InFlightGuard`（Drop）发布 `None` 并释放预约，等待者得到稳定 500 而非挂起，key 立即可复用。
  3. **原子性**：`reserve` 持 in_flight 锁完成 check+insert（并发首次请求不可能双双预约）；`complete` 同步执行 send→done.put→release，且不与 in_flight 锁嵌套（锁序 in_flight→done 单向），发布与缓存完成之间不存在可观测的中间状态。
  4. **确定性测试**（5 个新增，覆盖终审要求的场景）：完成后重复请求 replay 不再执行；done 缓存饱和时 in-flight 预约不被驱逐且重复请求等待、完成后 replay；等待者（含发布后订阅）读到保留结果不挂起；abort 释放等待者并复用 key；同 key 异 body 在执行中与完成后均 409。HTTP 层 5 并发同 key create 测试保留。
  - 修复后验证：http **17**（12+5 tracker 测试）/ cs 14 / client 35 / commands::workflow 64 / automation 18 / engine 60 / orchestrator 19 / manager 10 / sinks 4 / workflow_usage 5 全部通过；`pnpm test:workflow` 62 通过；fmt/check 双 bin 0 warning；新增代码 clippy 干净。
- 三审修复（re-review 一项 major：`watch::Sender::send` 在零接收者时不保留值——初始 Receiver 在预约时即被丢弃，常见"执行期间无等待者"场景下结果未保留，发布后、finish 前窗口内的重复请求会订阅到 `None` 并在 sender 关闭后误报 500）。修复：
  1. `publish`/`abort` 改用 `watch::Sender::send_replace`——无论接收者数量都保留新值，零接收者场景下结果同样被保留，晚订阅者立即读到。
  2. `complete` 拆分为 `publish`（send_replace 保留结果）+ `finish`（done LRU 写入 + 释放预约）两个同步步骤；`complete` 依次调用二者，对外仍原子。`with_idempotency` 继续调用 `complete`，生产行为不变。
  3. 新增确定性测试 `idempotency_duplicate_subscribing_after_publication_before_release_gets_result`：显式在"发布后、finish 前"窗口让重复请求订阅——验证其读到保留结果（非 500、不挂起），finish 后的重复请求 replay。abort 的 `send_replace(None)` 保证发布前后订阅者都有明确终态。
  - 修复后验证：http **18**（新增窗口测试）/ cs 14 / client 36 / commands::workflow 64 / automation 18 / engine 60 / orchestrator 19 / manager 10 / sinks 4 / workflow_usage 5 全部通过；`pnpm test:workflow` 62 通过；fmt/check 双 bin 0 warning；新增代码 clippy 干净。

### 11.5 第二期边界（未实现，明确 defer）

题库/benchmark adapter/LLM judge/verifier/compare/report、AI 自主出题、candidate/campaign、源码 worktree 实验与晋级、MCP/skill 安装、Agent/config 导入导出、headless/daemon/auto-launch、LAN/CORS、installer bundling、Vue HTTP 切换、评估 artifact/预算 admission——均不在本次交付中。

