# Runtime Ownership Inventory（U-1）

> 状态：静态盘点完成，作为 U-2/U-3/U-5 的迁移闸门。本文只记录当前代码证据，不表示 runtime 独立化已经完成。
>
> 盘点日期：2026-10-01；仓库：`/home/xc/dev/rust/chatspeed-plugin`。

## 1. 当前结论

- 当前唯一实际后台 owner 是 Tauri `chatspeed` 进程；workspace 没有 `chatspeed-runtime` binary 或 runtime workspace crate。
- `WorkflowApplicationService`、`WorkflowRuntimeHub`、`WorkflowManager`、`ChatState`、`ToolManager`、`CapabilityApplicationService` 和 `MainStore` 均在 Tauri `.setup()` 中创建并通过 `app.manage()` 注册。
- `src-tauri/contracts/src/lib.rs` 仍为空迁移边界；当前 HTTP DTO、discovery 和 SSE 类型仍属于主 Tauri crate。
- `chatspeed-cli` 已经是独立 workspace package，依赖 `chatspeed-contracts`、reqwest、tokio 等 client 依赖，不依赖 `tauri`、`wry`、`gtk` 或 `MainStore`。
- 当前发现的 Tauri runtime 初始化包含一个违反目标 fail-closed 语义的 `MainStore::new(":memory:")` fallback（`src-tauri/src/lib.rs:718-741`）；迁移到 runtime 时必须删除，不能带入新路径。

## 2. 启动、所有权和退出矩阵

| 当前位置 | 当前构造/行为 | 目标归属 | 证据与迁移要求 |
|---|---|---|---|
| `src-tauri/src/lib.rs:718-755` | 解析 DB 路径、`MainStore::new`、失败后 fallback 到 `:memory:`、注册 state | Runtime | runtime 必须解析独立 data dir、执行 migration、持有单实例/DB lock；失败直接 fail-closed。Tauri 不再注册 `MainStore`。 |
| `src-tauri/src/lib.rs:857-865` | `ChatState::new(..., Some(AppHandle), main_store)` | Runtime execution + Tauri event adapter | execution state 和 ToolManager 迁入 runtime；AppHandle 事件通知改为 HTTP/SSE/client bridge。 |
| `src-tauri/src/lib.rs:897-910` | `TauriGateway` + `WorkflowRuntimeHub::new` | Runtime hub + Tauri client output adapter | `WorkflowRuntimeHub::with_transport` 已提供抽象；`TauriGateway` 留在 client，runtime 不链接 Tauri。 |
| `src-tauri/src/lib.rs:913-920` | `WorkflowManager`、TerminalManager | 前者 Runtime；后者 Tauri-only | Workflow lifecycle registry 必须唯一留在 runtime；workflow-window PTY/terminal 留在 Tauri，需避免其成为 workflow executor owner。 |
| `src-tauri/src/lib.rs:922-946` | `DefaultSubAgentFactory` 与 `WorkflowApplicationService` | Runtime | service 持有 MainStore、ChatState、hub、factory、manager、capability；整体迁移或抽成 runtime-neutral crate。 |
| `src-tauri/src/lib.rs:960-1000` | capability recovery/reconcile 并注册 `CapabilityApplicationService` | Runtime | capability journal、MCP runtime effect 和 recovery 随 runtime 迁移；不能在 Tauri 保留第二个 service。 |
| `src-tauri/src/lib.rs:1006-1025` | spawn `/control/v1` server；失败只 log，桌面继续运行 | Runtime | runtime 必须把 control plane 作为 ready 前置条件；bind/discovery/migration 失败不可继续伪装可用。 |
| `src-tauri/src/lib.rs:1032` | Tauri 内启动 automation scheduler | Runtime | scheduler 属 runtime-owned 后台任务；客户端只通过协议操作。 |
| `src-tauri/src/lib.rs:1085-1105` | 注册 native tools、启动 legacy static/ccproxy server、后台启动 MCP | 分拆 | core tools/MCP runtime 属 Runtime；static/ccproxy/AppHandle/http server 留 Tauri-only 或按目标协议另行迁移，不能混入 control plane。 |
| `src-tauri/src/lib.rs:1140-1142` | Exit 时只调用 `request_shutdown()` 清理 control-plane discovery | Runtime lifecycle | runtime 需拥有有序 shutdown、MCP/executor drain、instance-fenced discovery cleanup；Tauri 退出只 release lease。 |

## 3. Runtime / client / Tauri-only 边界

### Runtime-owned（必须唯一存在于 runtime）

- `MainStore`、`DbRuntime`、所有 workflow/session/message/snapshot/event/context 数据访问。
- `WorkflowApplicationService` 与 `commands/workflow.rs` 中的 `*_core` canonical path。
- `WorkflowManager`（constitution 规定的唯一 session lifecycle registry）。
- `WorkflowRuntimeHub` 的 input registry、SSE broker、durable/live event authority。
- `DefaultSubAgentFactory`、workflow executor、AI/model/session execution。
- `ToolManager` 及普通 MCP/Skills/capability runtime、capability journal/recovery/reconcile。
- automation service/scheduler、runtime-owned background tasks。
- `/control/v1` server、auth、discovery、idempotency、SSE、lease 和 lifecycle coordinator。

### Tauri-only / client-only（不得进入 runtime 生产依赖图）

- `src-tauri/src/workflow/react/client/tauri/gateway.rs`：`AppHandle`/WebView event sink；只作为 client output adapter。
- `src-tauri/src/chat_hub/**`、`window.rs`、`tray.rs`、`shortcut.rs`、`frame_edges.rs`、updater、Tauri plugins。
- `TerminalManager` 与 workflow window PTY 生命周期。
- `scraper/**`、`tools/web_fetch.rs`、`tools/web_search.rs` 中依赖 WebView/桌面交互的能力；一期应通过显式 client Web MCP/bridge 暴露。
- `src-tauri/src/http/server.rs` 的 legacy static-file/ccproxy/AppHandle 路径；不可与 runtime `/control/v1` 合并。
- UI/window/clipboard/filesystem/desktop shortcut 等 command。

### 已有但尚未完成的边界钩子

- `WorkflowEventTransport` + `WorkflowRuntimeHub::with_transport`：已抽象 transport，但生产 hub 当前仍由 TauriGateway 构造。
- `ChatState::new(..., Option<AppHandle>, ...)`：`None` 可跳过 MCP 桌面事件，但 runtime 仍需移除对 `WindowChannels`/Tauri 类型的生产依赖。
- `UnavailableRuntimePort/Effects`：正确拒绝“无 runtime”状态，但不是 runtime binary 实现。
- `NoWindowTransport`：目前为 `#[cfg(test)]`，不能当作生产 runtime transport 证据。

## 4. Tauri command / database access matrix

| 文件/命令面 | 当前状态 | 目标分类 | U-5 处理 |
|---|---|---|---|
| `commands/workflow.rs` | Tauri wrapper + `*_core` 直接使用 application service fields，含 executor 创建/恢复 | Runtime canonical path + Tauri HTTP adapter | 保留命令名，改为 RuntimeClient；core 只在 runtime crate。 |
| `commands/workflow_automation.rs` | 全部通过 `WorkflowApplicationService::automation()` / `automation_run_compat` | Runtime-owned facade | 改 HTTP adapter；scheduler 只在 runtime。 |
| `commands/capability.rs` | 全部注入 `CapabilityApplicationService`，只委托 service | Runtime-owned facade | 改 HTTP adapter；不保留本地 capability service。 |
| `commands/mcp.rs` | 当前已统一委托 capability service；`list_mcp_servers` 额外读取 `chat_state.tool_manager` 做 live status overlay | Runtime-owned MCP + client compatibility adapter | status 也经 runtime wire 返回；删除 Tauri 对 ChatState/ToolManager 的直连。 |
| `commands/chat.rs` | `list_models` 注入 MainStore；`chat_completion`/`stop_chat` 注入 ChatState、ToolManager，并读取配置 | 混合，runtime-owned execution | chat/model/stop 改 HTTP；window/filter/UI-only 行为留 client；不得复制 chat executor。 |
| `commands/agent.rs` | add/update/delete/get 直接操作 MainStore，校验 sandbox scheme | Runtime-owned agent/config | 抽 application facade 或 HTTP route；Tauri 不直连 DB。 |
| `commands/message.rs` | conversation/message CRUD 直接走 MainStore/DbRuntime；部分命令向 Window emit | 数据 runtime-owned；emit 为 client adapter | 数据操作走 HTTP；窗口事件在 Tauri adapter 转发。 |
| `commands/note.rs` | note/tag CRUD 直接走 MainStore/DbRuntime | Runtime-owned persistence | 增加协议 facade；Tauri 仅保留兼容命令。 |
| `commands/sandbox.rs` | sandbox scheme CRUD 直接 MainStore，并带 AppHandle sync-state emit | runtime-owned policy data；emit client-only | 迁移 CRUD；客户端只处理通知。 |
| `commands/ccproxy.rs` | ccproxy stats 直接 MainStore/DbRuntime | 若 stats 随 runtime model execution，则 Runtime-owned | 先明确与 legacy ccproxy server 的边界，再改 HTTP；不可跨进程双写。 |
| `commands/setting.rs` | backup/restore 直接 MainStore，含 AppHandle | DB maintenance/runtime-owned；UI file picker client-only | restore 通过 runtime maintenance protocol；不能让 Tauri 直接替换 runtime DB。 |
| `commands/dev_tool.rs` | `AppHandle<Wry>` + ChatState ToolManager，直接调用 web tools | Tauri-only WebView bridge | 不迁移到 runtime；建立受控 client capability bridge。 |
| `commands/terminal.rs`、window/clipboard/fs 等 | window/PTY/desktop state | Tauri-only | 不进入 runtime owner；只保留客户端能力。 |

## 5. Database ownership inventory

`MainStore` 当前聚合全部下列 schema/访问模块，尚未完成按表进程隔离；在拆分完成前不能让 Tauri 和 runtime 同时打开同一 owner DB：

- workflow authority：`workflows`、`workflow_messages`、`workflow_context_messages`、`workflow_snapshots`、`workflow_events`、workflow usage/memory 相关表。
- runtime configuration/capability：`config`、`agents`、`ai_models`、`ai_skills`、`mcps`、`capability_operations`、automation 表。
- execution telemetry：`ccproxy_stats` 及 workflow attribution。
- client/product data currently co-located：`conversations`、notes/tags、sandbox schemes、proxy groups、chat hubs、plugin remnants、window/UI settings。

证据：schema migration 位于 `src-tauri/src/db/sql/migrations/v1.rs`–`v18.rs`，workflow 表在 `v5.rs`，capability/automation 表在对应 migration；`MainStore::new` 在 `src-tauri/src/db/main_store.rs:307-365` 初始化 migration/config 并创建 `DbRuntime`。数据库 constitution 要求所有访问经 `MainStore`/`DbRuntime`，但这不是进程级 ownership lock，WAL 也不能替代 runtime lock。

**闸门结论：** 当前不能证明某些 co-located UI/config 表已经可以安全留在 Tauri。D-5 要求的表级拆分尚未完成；在证明前，相关 Tauri direct DB commands 必须统一改为 runtime adapter，而不是让两个进程共享 SQLite。

## 6. Protocol / discovery / CLI 现状

- `src-tauri/src/workflow/react/client/http/discovery.rs`：`127.0.0.1`、原子 discovery 写入、Unix `0700/0600`、instance-id fenced remove 已有 focused tests。
- `http/auth.rs`：Authorization bearer、拒绝 Origin、拒绝 URL token 的安全边界已存在，应原样迁移。
- `http/server.rs`：`/control/v1/meta`、workflow CRUD/start/signal/stop/events/stream、capability/MCP/automation 路由已存在；`ControlPlaneState` 直接持有 `WorkflowApplicationService`。
- `http/sse.rs` + `hub.rs`：instance-local cursor、bounded replay、`reset_required` 与 lag handling 已存在；durable event ID 和 live cursor 分离。
- `src-tauri/src/bin/cs/discovery.rs` / `client.rs` / `sse.rs`：CLI 读取 discovery、使用 Authorization header、执行 JSON/SSE 请求；protocol major 不匹配时拒绝；当前仍维护 duplicate discovery/client/SSE types。
- `src-tauri/contracts/src/lib.rs` 只有 crate attribute，尚无唯一 wire schema或 fixture。

## 7. Workspace / packaging evidence

- `src-tauri/Cargo.toml` workspace members 目前为 `[".", "contracts", "cli"]`；binary 只有 `chatspeed` 与 feature-gated legacy `cs`。
- 主 crate 依赖图同时包含 Tauri、Wry、GTK、Tauri plugins、rusqlite、rmcp、axum、tokio 等；不能直接复制主 crate 作为 runtime。
- `src-tauri/cli/Cargo.toml` 是独立 `chatspeed-cli`，无 Tauri/Wry/GTK/MainStore 依赖，已证明 client 依赖方向可成立。
- `src-tauri/tauri.conf.json` 的 `externalBin` 目前只有 `target/release/cscli`；`beforeBuildCommand` 会构建并复制 cscli target binary；尚无 runtime binary 的 dev/prod resolution、bundle externalBin、权限或 stop 参数契约。
- `src-tauri/assets` 仅作为 bundle `resources`，不能默认视为 runtime binary 资源位置。

## 8. V-1 evidence and unresolved gates

### 已完成证据

- `cargo metadata --no-deps --format-version 1`：确认 workspace/package/binary 结构。
- `cargo tree -p chatspeed --depth 1`：确认主 crate 的桌面与后台依赖混合。
- `src-tauri/cli/Cargo.toml` 与 CLI 源码检查：确认 CLI 不打开 DB、不创建 executor。
- CodeGraph status：636 indexed files、16,434 nodes、66,436 edges；用于确认 `WorkflowApplicationService` 的 37 个依赖文件和 `WorkflowRuntimeHub` 的 8 个直接使用文件。
- 定向源码检查：`lib.rs` setup/exit、application、hub、HTTP server/discovery/auth/SSE、commands、db constitution。
- `commands/mcp.rs` 逐行检查：当前 mutation 通过 capability service；尚有 status overlay 直接读取 ToolManager，需迁移。

### 未完成但不构成当前架构改变的后续闸门

1. **runtime binary 路径/打包契约未实现**：A-1 仍待 U-3/U-5 验证；不能宣称已支持生产 bundle。
2. **共置 DB 表的最终 ownership 未拆分**：D-5 需要逐表/逐命令迁移；在完成前不允许两个进程共享 DB。
3. **Tauri 退出时 runtime shutdown/drain 尚无实现**：U-4/U-8 负责；当前只清理 control-plane discovery。
4. **legacy ccproxy/static HTTP server 与 runtime server 的边界尚未实现切换**：不能把二者合并。
5. **WebView bridge 尚无协议实现**：U-7 负责；runtime 不得依赖 WebView。

### U-1 stop conditions 检查结果

- 未发现“必须同时由 Tauri 和未来 runtime 写入同一表”的不可分割需求；但 co-located 表尚未证明可留在 client，故作为迁移闸门而非已解决事实。
- 当前构建配置无法证明 runtime binary 可随生产包可靠携带；这是待 U-3/U-5 验证的风险，不在 U-1 擅自设计新打包方案。
- 当前 workflow HTTP DTO 依赖主 crate 类型，尚未发现协议必须升级 major 的事实；U-2 需通过 fixture 复核。

## 9. U-1 handoff

U-1 输出允许进入 U-2，但不允许跳过以下边界：

- U-2 必须先将唯一 wire schema 放入 `contracts`，不能在 CLI/server/Tauri 各自新增第三套 DTO。
- U-3 必须新建不依赖 Tauri/Wry/GTK 的 runtime target，并移除 `:memory:` fallback。
- U-4 必须在 discovery 发布前完成 ready/lock/fencing 语义；当前 discovery writer 尚无 process-level single-instance lock。
- U-5 必须优先处理 direct DB/ChatState command matrix，并保留 workflow constitution 的单一 canonical path。
- 任何需要改变 protocol major、共享 DB、引入 OS daemon、或把 WebView 工具迁入 runtime 的实现，均超出当前批准策略，需停止并请求裁决。
