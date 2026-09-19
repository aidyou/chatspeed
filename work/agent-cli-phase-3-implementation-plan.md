> 本文件是 Phase 1/2 完成后的**下一阶段正式路线文档与实施计划**。
> 当前开发入口是 **Phase 3：能力管理（3A + 3B + 3C）**；3D、3E 只登记边界，不进入本次实现。
> 事实来源：当前仓库代码、`work/agent-cli-phase-1-implementation-plan.md`、
> `work/agent-cli-phase-2-implementation-plan.md` 以及
> `work/agent-cli-evaluation-self-improvement-design.md`。

# Agent CLI Phase 3 能力管理实施计划

## 0. 当前阶段指针

- **已完成基础**：Phase 1 的共享 workflow application service、loopback control plane 与 `cs` CLI；
  Phase 2 的本地 artifact、budget、experiment、campaign、隔离 owner、Harbor contract 与 promotion 闭环。
- **当前开发计划**：把以下三个步骤作为一个开发计划，按三个可独立验收的桌面端垂直切片小步推进：
  1. **3A：共享能力管理 service 与 operation/doctor 基础**；
  2. **3B：Skills 检查、安装、卸载与多目标投放**；
  3. **3C：MCP 安装、卸载、状态检查与可用工具查询**。
- **后续两次独立实现**：
  - **3D：本地自动化 facade 与桌面端操作/观测**；
  - **3E：外部实验数据与公开 benchmark 扩展评测**。
- **明确排除**：当前与后续已登记范围均不新增通用数据导出/导入功能；不扩展 config transfer，
  不实现 Agent、Skills、MCP、实验数据或用户配置的一键导出/导入。
- **推进规则**：3A → 3B → 3C。每个切片必须形成桌面端可见、CLI 可调用、可自动验证的完整能力，
  通过该切片验证后再进入下一切片；不等待三个切片全部写完才进行第一次可用性验证。

## 1. 目标与交付物

### 1.1 总体目标

在主进程唯一 owner 的约束下，把当前分散在 Tauri command、SQLite/ConfigCache、`ToolManager`、
`SkillScanner` 与 Python skill 脚本中的能力管理逻辑收敛到共享 Rust service。Tauri 桌面端与 `cs` CLI
调用同一套 service 语义，不能形成两个安装器、两个 MCP 生命周期或两个状态来源。

本阶段最终交付：

- transport-neutral 的能力管理 service、稳定 DTO/error、幂等 mutation、operation 状态与 `doctor`；
- Rust 原生、无需 LLM 的 Skill 安全检查器；
- Skill 从本地或受支持远端来源进入隔离 staging，经检查后安装到选择的目标；
- Skill 卸载，仅删除由 ChatSpeed 管理且内容未漂移的目标；
- 默认只安装到 ChatSpeed Skills 目录；用户可显式选择常用 Agent 工具的 Skills 目录；
- MCP 安装、卸载、状态检查、工具列表/刷新；
- Skills 与 MCP 的 `/control/v1` API、`cs` 命令和桌面设置入口；
- focused tests、临时 HOME/data-dir 集成测试与真实桌面 smoke 记录。

### 1.2 “安装”的产品语义

**Skill 安装**不是把 URL 直接复制到执行目录，而是：

```text
resolve source
→ download/copy into non-executable staging
→ archive/path/size validation
→ deterministic security check
→ parse manifest and derive permissions/findings
→ create immutable install plan
→ per-target collision check
→ atomic install missing targets; skip same-name targets
→ rescan and doctor
```

**MCP 安装**对用户表现为一个动作，但 service 内必须保留安全分层：

```text
resolve preset/descriptor/source
→ validate and redact
→ optional bounded acquisition
→ register disabled
→ bounded connectivity smoke
→ list declared tools
→ explicit enable (or install --enable after smoke passes)
```

“已注册”不等于“正在运行”；“能连接”也不等于“已允许 Agent 使用其全部工具”。

## 2. 当前代码事实与可复用基础

### 2.1 Skills

- `src-tauri/src/workflow/react/skills.rs` 的 `SkillScanner` 已能扫描 `SKILL.md`、`skill.json`、
  `manifest.json`，当前搜索顺序包含 `~/.chatspeed/skills`、`~/.agents/skills`、应用数据目录与内置资源。
- `get_system_skills` 已供 Workflow 与 Agent 设置页读取系统 Skills，但只返回扫描结果，没有安装记录、
  digest、安全状态、目标目录或卸载能力。
- `src-tauri/assets/skills/skill-installer/scripts/install-skill-from-github.py` 能从 GitHub 下载后直接复制，
  但不属于共享 Rust service，也没有程序化安全门禁、operation journal 或多目标管理。
- `src-tauri/assets/skills/skill-vetter/SKILL.md` 是 prompt 型审阅指南，只能作为解释性参考，不能给安装
  pipeline 提供权威 verdict。
- `src/components/setting/Skill.vue` 管理的是数据库 `ai_skill` prompt 记录，不等同于 workflow runtime 使用的
  文件型 Agent Skills；本阶段必须在 UI 和类型命名上区分两者，不能混用同一个 store/CRUD。

### 2.2 MCP

- `src-tauri/src/db/mcp.rs` 已提供 MCP 持久化 CRUD，ConfigCache 是桌面运行中的配置投影。
- `src-tauri/src/tools/tool_manager.rs` 已提供 register/unregister/start/stop/status/refresh/list-tools 能力。
- `src-tauri/src/commands/mcp.rs` 当前把 DB/cache mutation 与异步 runtime transition 混在 Tauri command 中；
  add/update/enable/disable/delete 返回与后台启动/停止结果可能不同步。
- `src/components/setting/Mcp.vue` 与 `src/stores/mcp.js` 已有服务器列表、启停、删除、刷新和工具展开 UI，
  是 3C 的桌面端复用入口。
- `McpServerConfig` 目前可能包含 `bearer_token` 与 `env` 值；所有新 DTO、日志、operation 与 CLI 输出必须
  默认脱敏，不能把 secret 带入 argv、普通日志、artifact 或模型 transcript。

### 2.3 共享基础

- Phase 1 已有认证 loopback `/control/v1`、`Idempotency-Key`、稳定 JSON error 与 `cs` HTTP client。
- 主进程已经是 `MainStore`、ConfigCache、`ToolManager` 和 MCP child process 的唯一 owner。
- Phase 2 已有隔离 owner 内部的 run-scoped verified bundle MCP/Skill 生命周期；它不是用户全局安装产品，
  本阶段不得把实验 lease 直接提升为全局安装，也不得破坏它的隔离语义。

## 3. 范围边界与非目标

### 3.1 Phase 3 当前范围（3A + 3B + 3C）

- 只管理**文件型 Agent Skills**和用户全局 MCP server；不重构数据库 `ai_skill` prompt 功能。
- Skill v1 来源支持：
  - 本地目录；
  - 本地归档（仅安全支持的归档格式）；
  - GitHub repository/path URL 或等价的 `owner/repo + path + ref` 描述。
- MCP v1 安装支持：
  - 现有桌面预设；
  - strict JSON descriptor；
  - `stdio` 与 `streamable_http`；SSE 继续按当前实现明确拒绝。
- MCP 需要外部 package runner 时，只允许结构化 argv 的受支持 adapter；禁止拼接 shell 字符串。
- Skills/MCP mutation 均由主进程 service 执行；CLI 不直接写 SQLite、`~/.chatspeed` 或外部 Skills 目录。

### 3.2 明确不做

- **不做任何新的通用数据导出/导入**，包括配置、Agent、Skills、MCP、实验数据和凭据迁移；
- 不实现 Skill 自动更新、自动覆盖、依赖解析器或 marketplace；
- 不允许用户 Skill 覆盖内置/保留的 `chatspeed-cli` Skill；
- 不使用 LLM 作为安装放行条件；不提供跳过程序化检查的 `--force`；
- 不扫描或修改任意自定义目录；v1 只允许内置 target registry 中的目录；
- 不执行 MCP tool 来证明安装成功；3C 只做连接、状态和 `list_tools`；
- 不在本阶段实现 automation facade、Experiment Launcher/Monitor 或外部 benchmark 数据；
- 不改变普通 workflow、实验 run-scoped capability、promotion、ccproxy 或旧 static HTTP router 语义；
- 不做无关的 Agent CRUD、config transfer、依赖升级或 UI 全面重设计。

## 4. 目标架构

```text
Settings UI / Tauri commands ─┐
                              ├─> CapabilityApplicationService
cs /control/v1 client ────────┘               │
                                               ├─ SkillPackageService
                                               │   ├─ source resolver/staging
                                               │   ├─ deterministic checker
                                               │   ├─ target registry
                                               │   └─ install manifest/uninstall
                                               ├─ McpApplicationService
                                               │   ├─ MainStore + ConfigCache
                                               │   └─ ToolManager runtime transition
                                               └─ CapabilityOperationStore + Doctor
```

### 4.1 Service 与 operation contract

共享 service 接收显式依赖：`Arc<MainStore>`、`Arc<ChatState>`/session-independent `ToolManager`、
`app_data_dir`、canonical ChatSpeed home、HTTP downloader 和 clock/id generator。Tauri/HTTP adapter 只做 wire
转换、认证与错误映射。

所有 mutation 返回版本化 `CapabilityOperationV1`：

- `operation_id`、`kind`、`actor`、`idempotency_key`、`request_hash`；
- `state=planned|staging|checking|applying|completed|blocked|failed|needs_reconcile`；
- source identity、目标 ID、开始/结束时间；
- 每个目标的 `installed|skipped_existing|blocked|rolled_back` 结果；
- redacted findings/status/error 与可执行的 doctor 建议。

Skill 文件系统与 MCP SQLite/runtime 无法形成同一个事务，因此使用 journaled saga。每个 commit point 先写
intent，再执行 effect，再写 observation；重复同一 idempotency key 不产生第二次安装或第二个 MCP 记录。
`doctor` 只根据 journal、install manifest、DB desired state 与 runtime observation 收敛，不从日志文本猜测成功。

### 4.2 Skill 安装记录

每个由 ChatSpeed 管理的 Skill 记录：

- canonical name、manifest version、source/ref、source/content digest；
- deterministic checker version、verdict 与 findings digest；
- 安装目标、物理路径、文件 manifest 与安装 operation；
- `installed_at`、当前内容 digest、是否 drifted；
- 卸载所需的 ownership proof。

记录不得包含下载凭据、GitHub token、文件原文或环境变量值。

### 4.3 Skill target registry

v1 提供稳定 target ID：

- `chatspeed`（默认且唯一默认目标）；
- `agents`（通用 `.agents/skills`）；
- `claude-code`；
- `codex`；
- `opencode`；
- `cursor`；
- `windsurf`；
- `cline`；
- `trae`。

约束：

1. 未显式 `--target` 时只选择 `chatspeed`，绝不自动扩散到所有工具目录；
2. 各 target 的跨平台路径与环境变量由独立 resolver 计算，实施时以对应工具当前正式约定和现有环境做
   freshness check，不在多个调用方硬编码路径；
3. UI 展示解析后的脱敏路径、目录是否存在及工具是否可检测，但只有用户勾选后才写入；
4. 不自动把外部工具目录加入 ChatSpeed `SkillScanner`；ChatSpeed runtime 默认只消费自己的目录和既有通用
   `.agents/skills` 规则；
5. 每个选中目标独立判断同名冲突：普通已存在目录返回 `skipped_existing`，不覆盖、不合并、不删除；
6. 同名目标若是 symlink、特殊文件、大小写歧义或越界路径，返回 `blocked`，不能伪装为安全的 skip；
7. 一个目标已存在不阻止其他缺失目标安装，最终结果必须逐目标报告。

### 4.4 Deterministic Skill checker

`SkillSecurityChecker` 完全不调用 LLM，输出：

```text
verdict = pass | blocked | inconclusive
findings[] = {rule_id, severity, file, evidence_kind, message}
permissions = {files, commands, network_domains, package_managers, sensitive_reads}
```

至少检查：

- 归档路径穿越、绝对路径、symlink/hardlink/device、文件数/单文件/总大小、压缩比；
- 必需 manifest、名称合法性、目录名/manifest name 一致性、重复 name；
- 二进制、可执行位、脚本、安装脚本、package manager 调用；
- shell/process、`eval`/`exec`、动态下载、编码/混淆、高熵 payload；
- 外部网络域名/IP、凭据/token 请求、浏览器 cookie/session；
- `~/.ssh`、`~/.aws`、密钥链、系统目录及其他敏感路径读写；
- 对 ChatSpeed home、其他 Agent 工具目录和系统配置的越权写入。

安装只接受 `pass`。`blocked` 或无法完整分析的 `inconclusive` 均不写入任何目标。解释性 LLM review
未来可以附加，但不能把 blocked/inconclusive 改成 pass。

### 4.5 Skill 卸载语义

- 默认只能卸载有 ChatSpeed ownership manifest 的安装；
- 对每个目标重新计算 digest；内容漂移时拒绝删除并返回 `needs_reconcile`；
- 从未安装、因同名而跳过或由其他工具管理的目录不得删除；
- 卸载先原子移动到同文件系统 quarantine，再提交 manifest/journal，完成后清理；失败可恢复；
- 内置 Skill 和保留的 `chatspeed-cli` 永远不可卸载；
- 卸载后触发 rescan，Agent/Workflow 选择器不能继续显示已经不存在的托管 Skill。

### 4.6 MCP desired/runtime state

MCP projection 必须同时显示：

- persisted desired state：registered + enabled/disabled；
- observed runtime status：starting/connected/running/stopped/error；
- `last_checked_at`、redacted last error；
- tools freshness 与工具数量；
- 是否存在 DB/cache/runtime drift。

安装默认注册为 disabled。只有 smoke 与 `list_tools` 成功后，用户显式 enable 或使用 `install --enable`
才能进入 running。卸载顺序为 disable → stop/confirm → delete persistence/cache；无法确认 stop 时保留 disabled
记录并标记 `needs_reconcile`，不能先删 DB 再留下未知进程。

## 5. 公共命令与桌面端产品面

### 5.1 CLI 命令

```text
cs skill targets
cs skill list [--target <id>]
cs skill inspect <name> [--target <id>]
cs skill check (--source <github-ref> | --path <path> | --installed <name>)
cs skill install (--source <github-ref> | --path <path>) [--target <id>]...
cs skill uninstall <name> [--target <id>]...
cs skill status <operation-id>

cs mcp list
cs mcp inspect <id|name>
cs mcp install (--preset <id> | --file <descriptor.json>) [--enable]
cs mcp uninstall <id|name>
cs mcp status [<id|name>] [--check]
cs mcp tools <id|name> [--refresh]
cs mcp enable|disable <id|name>

cs doctor capabilities [--reconcile <operation-id>]
```

- `--output human|json|jsonl` 继续复用现有全局契约；machine field/error code 不本地化。
- `skill check` 是用户要求的独立、无需 LLM 的检查命令；`skill install` 内部必须调用同一个 checker，
  不能维护一套较弱的安装检查。
- MCP `tools` 只返回 server 声明的可用工具、描述、input schema 与 disabled 状态，不执行工具。

### 5.2 HTTP/Tauri adapter

建议新增 additive `/control/v1` 资源：

```text
GET    /control/v1/skill-targets
GET    /control/v1/skills
POST   /control/v1/skills:check
POST   /control/v1/skills:install
POST   /control/v1/skills/{name}:uninstall
GET    /control/v1/capability-operations/{operation_id}
POST   /control/v1/capabilities:doctor

GET    /control/v1/mcp-servers
POST   /control/v1/mcp-servers:install
GET    /control/v1/mcp-servers/{id}
POST   /control/v1/mcp-servers/{id}:enable
POST   /control/v1/mcp-servers/{id}:disable
POST   /control/v1/mcp-servers/{id}:check
GET    /control/v1/mcp-servers/{id}/tools
POST   /control/v1/mcp-servers/{id}/tools:refresh
POST   /control/v1/mcp-servers/{id}:uninstall
```

所有 mutation 必须 bearer auth + `Idempotency-Key`；IDs 用 string；secret 字段不回显。现有 Tauri command 名和
camelCase wire 保持兼容，但 command body 下沉调用相同 service，避免桌面端与 CLI 行为分叉。

### 5.3 桌面端

**Skills**：在设置中新增明确命名的“Agent Skills/系统 Skills”管理区，与现有 prompt 型“AI Skills”分开。
提供来源输入、检查结果、目标选择、逐目标安装结果、已安装列表、风险/来源/digest、卸载与 doctor。默认只勾选
ChatSpeed；其他 target 由用户主动勾选。同名 skip 必须以非错误状态展示。

**MCP**：复用现有 `Mcp.vue` 页面与 Pinia store 的交互布局，改为共享 service DTO；保留手动/预设入口、
启停、删除与工具展开，并补充明确的 installing/checking/needs_reconcile、last checked、错误脱敏与刷新结果。
安装成功但未 enable 时显示“已安装/已停用”，不能显示成 running。

所有新增用户文案进入 i18n，en/zh-Hans/zh-Hant 结构一致且 key 排序。

## 6. 验收契约

### Acceptance Criteria

- **AC-1 — 单一共享管理路径**：Skills/MCP 的 Tauri 与 HTTP handler 都委托同一 Rust service；CLI 不直接写
  DB/配置/目标目录或启动进程；旧桌面行为通过 characterization test 保持兼容。
- **AC-2 — Operation 与恢复可观测**：每个 mutation 有稳定 operation ID、结构化终态与逐 effect 结果；重复
  idempotency key 无重复 effect；doctor 能识别 staging、journal、manifest、DB/cache/runtime drift。
- **AC-3 — 默认只装 ChatSpeed**：未指定 target 的 Skill 安装只写 ChatSpeed Skills 目录，对 `.agents`、
  Claude Code、Codex、OpenCode、Cursor、Windsurf、Cline、Trae 均零写入。
- **AC-4 — 显式多目标与同名跳过**：用户可选择 `agents`、`claude-code`、`codex`、`opencode`、
  `cursor`、`windsurf`、`cline`、`trae`；每目标独立安装；同名普通目录 `skipped_existing` 且内容不变，
  可继续安装其他缺失目标。
- **AC-5 — 下载后、安装前 fail closed 检查**：远端内容只进入隔离 staging；path/archive/manifest/
  permission/security 检查全部通过后才能写目标；blocked/inconclusive 在所有目标上零提交。
- **AC-6 — 独立非 LLM 检查命令**：`cs skill check` 和桌面检查入口不调用 LLM，可检查来源、本地内容或
  已安装 Skill；报告规则、verdict、权限与 findings；安装复用同一 checker/version。
- **AC-7 — 安全卸载**：可卸载由 ChatSpeed 安装且未漂移的 Skill；不能删除内置、保留、跳过、非托管或
  已漂移目录；失败可由 doctor 收敛。
- **AC-8 — Skills 桌面可用**：用户能在桌面端完成来源解析→检查→目标选择→安装→查看→卸载；Workflow/
  Agent selector 的 rescan 结果与安装终态一致。
- **AC-9 — MCP 安装**：可从 preset 或 strict descriptor 安装 stdio/streamable HTTP server；配置验证、
  secret redaction、默认 disabled、smoke 与显式 enable 语义成立，重复请求不产生同名重复记录/进程。
- **AC-10 — MCP 卸载**：卸载先停止并确认 runtime，再删除 persistence/cache；停止结果未知时保留 disabled
  记录并进入 `needs_reconcile`，无孤儿进程或“DB 已删但仍运行”的成功假象。
- **AC-11 — MCP 状态与工具列表**：CLI/桌面可获取 desired + observed 状态，执行有界 status check，刷新并
  列出可用工具；disabled/error/offline 有稳定结构化结果，列工具不执行工具。
- **AC-12 — MCP 桌面/CLI parity**：同一 server 经桌面与 CLI 交错安装、查询、启停、刷新、卸载后，DB、
  ConfigCache、ToolManager 与两个客户端的最终投影一致。
- **AC-13 — 安全与脱敏**：Skill/MCP token、env value、下载凭据、敏感路径和工具参数不进入普通日志、CLI
  machine 输出、operation、journal 或模型上下文；MCP argv 无 secret，shell 字符串不被执行。
- **AC-14 — 范围不越界**：本阶段不新增数据导出/导入、automation、外部实验数据、benchmark 扩展、
  marketplace、Skill update/overwrite、MCP tool execution 或第二 runtime owner。

### Protected Invariants

- **INV-1**：`MainStore`/ConfigCache/`ToolManager`/MCP child 仍由主进程唯一持有。
- **INV-2**：旧 Tauri command 名、参数形状与现有 MCP/Workflow 页面契约不做破坏性修改。
- **INV-3**：文件型 Agent Skills 与数据库 prompt 型 `ai_skill` 是不同产品对象，绝不共用 ID/存储/删除逻辑。
- **INV-4**：程序化 checker 是唯一安装安全门；LLM 不能放行、降低或覆盖 verdict。
- **INV-5**：默认 Skill target 永远只有 `chatspeed`；外部 target 每次 mutation 都需显式选择。
- **INV-6**：同名冲突永不覆盖；非托管内容永不由卸载隐式删除。
- **INV-7**：MCP registered/enabled/runtime/tool availability 分层，不把异步 intent 报成已完成事实。
- **INV-8**：所有 mutation 可幂等恢复；无法证明 effect 是否发生时进入 `needs_reconcile`，不盲目重试。
- **INV-9**：run-scoped experiment capability 继续隔离，不写入全局 Skills/MCP registry。
- **INV-10**：无新增通用数据导出/导入；既有历史 artifact/评测能力不因本阶段被删除或改写。

## 7. 三个小步快跑切片

### 3A — 共享 service、只读 inventory 与 doctor

**目的**：先收敛 authority，桌面端与 CLI 能查看同一 Skills/MCP inventory 与诊断结果，再开放 mutation。

**执行单元**：

- **U-1**：定义 `CapabilityApplicationService`、`CapabilityOperationV1`、稳定 error、target ID、redaction 与
  journal/manifest contract；建立 crash/reconcile 纯状态机。
- **U-2**：抽 `McpApplicationService`，把 list/inspect/status 与既有 Tauri MCP query 委托到 service；状态同时
  投影 DB desired state、ToolManager observed state 和 tools freshness。
- **U-3**：建立 Skill inventory，区分 builtin、ChatSpeed managed、通用/外部 discovered、drifted；`SkillScanner`
  与 target registry 共用 canonical path resolver。
- **U-4**：新增只读 HTTP/CLI `skill list/targets`、`mcp list/status`、`doctor capabilities`；桌面设置页展示相同
  inventory/doctor 事实。

**切片验收**：AC-1、AC-2 的只读部分、AC-3 的 target 默认、AC-11 的只读状态、AC-13、AC-14；
不包含 install/uninstall mutation。

### 3B — Skills check/install/uninstall 端到端

**目的**：形成第一个完整可用 mutation：用户在桌面端下载到 staging、看检查报告、选择目标、安装并卸载。

**执行单元**：

- **U-5**：实现 source resolver/downloader/staging/archive guard 与 deterministic checker；构造恶意 fixture
  覆盖 traversal、link/device、zip bomb、超限、脚本/网络/凭据/敏感路径。
- **U-6**：实现 install plan、per-target collision/atomic apply、managed manifest、rescan 与 rollback；默认只选
  `chatspeed`，显式支持八个外部 target。
- **U-7**：实现 ownership-bound uninstall、drift refusal、quarantine/recovery 与 doctor reconcile。
- **U-8**：新增 Skill HTTP/Tauri/CLI 与桌面“Agent Skills”管理区；完成 `check/install/uninstall/status` 和逐目标
  状态 UI。

**切片验收**：AC-2..AC-8、AC-13、AC-14；完成后做真实临时 HOME + 桌面 smoke，再进入 3C。

### 3C — MCP install/uninstall/status/tools 端到端

**目的**：把现有 MCP 页面和 command 收敛到同一安全生命周期，并向 CLI 开放用户要求的四项能力。

**执行单元**：

- **U-9**：将 add/update/enable/disable/restart/refresh/delete transition 下沉 service；定义 intent/observation、
  per-server serialization、compensation 与 reconcile，消除 command 先成功、后台失败却无结构化终态的问题。
- **U-10**：实现 preset/descriptor install：validate → register disabled → smoke → list tools → optional enable；
  过滤 shell/argv/env/URL，所有 projection 脱敏。
- **U-11**：实现安全 uninstall、bounded status check、tools list/refresh；覆盖 offline、timeout、disabled、
  duplicate name、start/stop/list-tools failure 与 crash recovery。
- **U-12**：新增 MCP HTTP/CLI；让现有 `Mcp.vue`/Pinia store 使用 service DTO 与 operation 状态；完成桌面/CLI
  交错 smoke。
- **U-13**：全阶段范围审计、focused 回归、文档 implementation record 与 smoke 记录；核对 AC/INV/V。

**切片验收**：AC-1、AC-2、AC-9..AC-14；通过后 Phase 3 完成，下一入口是单独的 3D 计划。

## 8. 验证策略与矩阵

### Verification

- **V-1 — Service/adapter parity**：直接 service、Tauri wrapper、HTTP handler 对同一请求得到等价 DTO/error；
  CLI 无 DB/MainStore/ToolManager/直接文件 mutation 依赖。
- **V-2 — Operation/idempotency/crash**：每个 commit point 前后 fault injection；同 key 重试零重复 effect；
  ambiguous effect → `needs_reconcile`；doctor roll-forward/rollback 收敛。
- **V-3 — Skill checker security**：恶意 archive/tree fixture 覆盖 traversal、absolute、symlink、hardlink、device、
  bomb、文件/大小限制、binary/script、download/IP、credential/sensitive path、obfuscation；blocked/inconclusive 零安装。
- **V-4 — Skill target matrix**：环境注入的跨平台 home/config resolver；默认仅 ChatSpeed；八个外部 target 显式
  选择；同名 skip、可疑 collision block、部分 skip + 部分 install、case sensitivity。
- **V-5 — Skill lifecycle integration**：临时 HOME/data-dir 完成 local/GitHub fixture staging→check→install→scan→
  drift→uninstall/拒绝→doctor；验证内置/`chatspeed-cli` 不可覆盖/删除。
- **V-6 — MCP service state machine**：mock MCP client/ToolManager 覆盖 register/start/stop/status/list-tools/refresh
  的成功、timeout、error、重复、并发、取消和重启恢复；DB/cache/runtime 最终一致。
- **V-7 — MCP local fixture integration**：使用无网络、无 secret 的确定性 stdio MCP fixture，真实完成 install
  disabled→smoke→tools→enable/status→disable→uninstall；确认没有执行任何工具。
- **V-8 — HTTP/CLI contract**：auth、Origin、Idempotency-Key、body limit、snake_case、string ID、redaction、
  human/json/jsonl、exit code、blocked/skip/needs_reconcile golden tests。
- **V-9 — Frontend focused verification**：受影响 Pinia/component/composable tests、`pnpm` type/build 或最窄现有
  检查；验证 loading/empty/error/blocked/skipped/reconcile/disabled/running 与响应式布局。
- **V-10 — 真实 desktop smoke 与范围审计**：桌面与 CLI 操作同一 Skill/MCP；重启后状态一致；扫描日志与
  operation 无 secret；diff 不含数据导出/导入、automation、外部 dataset/benchmark 或无关重构。

### Acceptance Matrix

| Requirement | Units | Verification |
|---|---|---|
| AC-1 | U-1..U-4,U-8,U-9,U-12 | V-1,V-8,V-9 |
| AC-2 | U-1,U-4,U-6,U-7,U-9,U-11,U-13 | V-2,V-5,V-6,V-10 |
| AC-3 | U-1,U-3,U-6,U-8 | V-4,V-5,V-8,V-9 |
| AC-4 | U-3,U-6,U-8 | V-4,V-5,V-9,V-10 |
| AC-5 | U-5,U-6,U-8 | V-3,V-5,V-8,V-10 |
| AC-6 | U-5,U-8 | V-3,V-5,V-8,V-9 |
| AC-7 | U-6,U-7,U-8 | V-2,V-4,V-5,V-9 |
| AC-8 | U-3,U-5..U-8 | V-5,V-9,V-10 |
| AC-9 | U-9,U-10,U-12 | V-6,V-7,V-8,V-9 |
| AC-10 | U-9,U-11,U-12 | V-2,V-6,V-7,V-9,V-10 |
| AC-11 | U-2,U-4,U-9,U-11,U-12 | V-1,V-6,V-7,V-8,V-9 |
| AC-12 | U-2,U-4,U-9..U-13 | V-1,V-6,V-7,V-9,V-10 |
| AC-13 | U-1,U-5..U-13 | V-3,V-6,V-7,V-8,V-10 |
| AC-14 | U-1,U-4,U-8,U-12,U-13 | V-1,V-8,V-10 |
| INV-1..INV-10 | U-1..U-13 | V-1..V-10 |

## 9. Decision / Uncertainty Ledger

### Confirmed Decisions

- **D-1**：3A/3B/3C 放在同一个 Phase 3 开发计划中，但按垂直切片逐步交付与验证。
- **D-2**：Skill 默认只安装到 `chatspeed`；所有外部 target 都是 opt-in。
- **D-3**：同名 Skill 按目标跳过，不覆盖；可疑 path/link collision 直接阻止。
- **D-4**：Skill 先下载/复制到隔离 staging，再执行非 LLM checker；只有 `pass` 可安装。
- **D-5**：提供独立 `cs skill check`，安装复用同一 checker。
- **D-6**：提供 ownership-bound Skill uninstall；不删除非托管或 drifted 内容。
- **D-7**：MCP 安装默认 disabled；smoke/list-tools 成功后才允许显式 enable。
- **D-8**：MCP 必须支持 install、uninstall、status check、list tools；工具列表操作不执行工具。
- **D-9**：3D 本地自动化和 3E 外部实验评测分别作为后续两次独立实现。
- **D-10**：不新增数据导出/导入功能。

### Implementation-time freshness checks（不阻塞计划批准）

- **F-1**：逐个核对 Claude Code、Codex、OpenCode、Cursor、Windsurf、Cline、Trae 当前用户级 Skills 目录、
  环境变量与平台差异；若某工具没有稳定正式目录，该 target 返回 `unsupported`，不得猜测路径。
- **F-2**：核对现有 MCP preset 的 command/args/package runner，决定 v1 adapter allowlist；不支持的 preset 保留
  手动 descriptor，不自动执行任意 install command。
- **F-3**：核对 Rust 现有依赖能否安全处理选定归档格式与下载；若需要新增依赖，按项目规则单独确认。
- **F-4**：核对 capability journal 使用 app-data 文件还是 additive DB schema 的最小实现。只要 operation contract、
  crash recovery 与单一 owner 不变，可作为局部实现决策；不得因此绕过 MainStore/runtime authority。

### Stop Conditions

命中以下条件时停止对应 unit 并询问用户：

- 需要默认写入 ChatSpeed 之外的 Skills 目录或覆盖同名内容；
- 需要允许 blocked/inconclusive Skill 安装、使用 LLM 放行或提供跳过检查的 force；
- 需要删除非托管/drifted Skill；
- 需要用 shell 拼接 MCP 安装/启动命令、在 argv/log 中传 secret 或安装后直接执行 MCP tool；
- 需要 CLI 直开 SQLite、直接修改目标目录或建立第二 ToolManager/runtime owner；
- 需要加入数据导出/导入、automation、外部实验数据/benchmark、marketplace 或无关架构重构；
- 需要破坏既有 Tauri command/wire、普通 workflow 或 run-scoped experiment capability 契约。

## 10. 后续两次独立实现

### 3D — 本地自动化（下一次独立计划）

前置：3A–3C 完成且 capability operation/doctor 稳定。

目标：复用现有 Workflow Automation scheduler/application service，补齐 `cs automation` typed facade 与桌面端
统一观测；支持 list/get/draft/apply/create/update/enable/disable/run/runs/delete。目标解析只能生成受约束计划，
不能因为自然语言自动获得新的 shell、路径、网络、MCP 或 Skill 权限。该阶段只处理本机 automation，不接入外部
实验数据，不新增导出/导入。

3D 必须单独起草 Acceptance Criteria、Protected Invariants、Execution Units 与 verification matrix；不在 Phase 3
实现中预埋未验证 mutation。

### 3E — 外部实验数据与公开评测扩展（再下一次独立计划）

前置：3D 完成，且 Skills/MCP 安装链路已通过实际使用验证。

目标：在 Phase 2 已有本地 artifact/evaluator/Harbor contract 上，单独评估并接入需要下载或维护外部数据的
Aider/SWE-bench/Terminal-Bench/private holdout 等任务集，固定 dataset/version/digest/split、资源预算、许可、
污染与 verifier 边界。先做小型 smoke，再决定完整评测。

3E 的“外部数据”仅指版本锁定的 benchmark/dataset acquisition，不等于通用用户数据导入/导出；本路线仍不
开放配置、Agent、Skills、MCP 或实验数据的一键导入/导出。

## 11. 实施完成回写规则

每个切片完成后在本文件末尾追加 Implementation Record，不改写历史事实：

```text
### 3A|3B|3C Implementation Record (as built)
- 日期 / 阶段推进
- 实际文件、service、routes、commands 与 UI
- operation/schema/migration（如有）
- focused tests 与最后一次修改后的结果
- desktop/CLI smoke 及 operation/server/skill ID（脱敏）
- secret/范围审计
- 未验证项、环境限制和剩余风险
- 下一切片入口
```

Phase 3 完成前必须逐项核对 AC-1..AC-14、INV-1..INV-10、U-1..U-13、V-1..V-10；没有 pending
remediation 才可把当前阶段指针推进到 3D。

### 3A Implementation Record (as built)

- 日期 / 阶段推进：2026-09-18。推进 3A（U-1、U-2、U-3 完成）与 3B 的服务层（U-4 Skill
  source/staging/ZIP/checker、U-5 plan/installer/ownership、U-6 uninstaller/quarantine 完成并
  有 focused 测试，但**尚未经任何 adapter 暴露**）。3B 剩余 U-7、3C（U-8..U-12）**未开始**，
  U-13 仅完成本记录，Phase 3 未完成，阶段指针不推进到 3D。

实际文件、service、routes、commands 与 UI：

- 新增 `src-tauri/src/capability/`：
  - `mod.rs`：`CapabilityApplicationService`（唯一公开入口），持有 `CapabilityRepository`、
    `ResourceLocks`、`TargetEnvironment` 与 `Arc<dyn McpRuntimePort>`；提供 skill targets /
    skill inventory / MCP projection / doctor / operation 读写与 `recover_interrupted_operations`。
  - `types.rs`：`CapabilityKind`、`OperationState`(planned|staging|checking|applying|completed|
    blocked|failed|needs_reconcile)、`EffectIntent`、`EffectOutcome`、`CapabilityOperation`、
    `OperationEffect`、`OperationRequest`/`OperationBegin`、`SkillInstallation*`。
  - `error.rs`：稳定 code 表（含 `idempotency_key_required`/`idempotency_key_conflict`/
    `unsupported_target`/`check_blocked`/`needs_reconcile`/`interrupted_before_effect`/
    `effect_state_unknown` 等）+ redacted `Display` + `StoreError`/io/serde 转换。
  - `operation.rs`：canonical JSON（键排序）→ SHA-256 `request_hash`、`content_digest`、
    `new_operation_id`(`op-skill-*`/`op-mcp-*`)、`require_idempotency_key`、`ResourceLocks`。
  - `redaction.rs`：分段密钥键识别（`bearer_token` 命中、`author` 不命中）、`env`/`headers`
    整块脱敏、URL userinfo 与 `token=` 内联脱敏、`bounded_redacted_json`。
  - `repository.rs`：`capability_operations` / `capability_operation_effects` /
    `skill_installations` 读写；`begin` 按 `(capability, actor_scope, idempotency_key)` 唯一约束
    做 replay/conflict，冲突时重读权威行；effect intent/outcome 分两个短事务写入。
  - `targets.rs`：固定 target registry（`chatspeed` + 8 外部），路径只在核对到官方约定后
    `supported`，否则 `unsupported` + `path_not_verified`；默认集合恒为 `chatspeed`。
  - `skill_inventory.rs`：复用 `SkillScanner::scan_detailed` 解析，分类 builtin/managed/
    managed_drifted/discovered，保留 `chatspeed-cli` 保留名与 builtin 保护；drift 由
    `skill_installations` 的文件 manifest 校验得出。
  - `skill/manifest.rs`：目录内容清单（SHA-256 + size），symlink/special file 直接拒绝。
  - `mcp_service.rs`：`desired` / `runtime`(observed + 时间戳) / `tools`(count + freshness) 分层
    投影；`config_fingerprint` 只对脱敏后的配置取哈希；secret 仅以布尔存在位表达。
  - `mcp/runtime.rs`：`McpRuntimePort` 只读端口 + `UnavailableRuntimePort` +
    `ToolManagerRuntimePort`（只读 `ToolManager`，`McpStatus::Error` 只报 `error` 不带消息）。
  - `doctor.rs`：report-only 的 journal/ownership/runtime/staging 漂移报告与稳定 finding codes。
- `src-tauri/src/db/sql/migrations/v20.rs`（additive，仅新增表/索引）。
- `src-tauri/src/workflow/react/skills.rs`：新增 `ScannedSkill`、`scan_detailed()` 与
  `SkillScanner::with_search_paths()`（未改动既有 `scan()` 的优先级语义）。
- `src-tauri/src/workflow/react/application.rs`：`WorkflowApplicationService` 创建并持有唯一
  `capability` 实例（注入 `ToolManagerRuntimePort`），以 `capability()` 暴露给 Tauri 层。
- `src-tauri/src/lib.rs`：`pub mod capability;`、State 12 注册 + **启动恢复门**（先分类遗留
  operation，再启动控制面）。
- `src-tauri/src/commands/capability.rs`：5 个只读 Tauri 命令（`capability_skill_targets`、
  `capability_skill_inventory`、`capability_mcp_servers`、`capability_operation`、
  `capability_doctor`），错误以脱敏 `{"code","message"}` JSON 字符串返回。
- `src-tauri/src/workflow/react/client/http/{server.rs,dto.rs}`：新增只读路由
  `GET /control/v1/skill-targets|skills|mcp-servers|capability-operations/{id}|capability-doctor`
  （沿用既有 bearer / body-limit 中间件）与 `dto::capability_error_response` 状态码映射。
- `src-tauri/src/bin/cs.rs`、`bin/cs/{args.rs,capability.rs,skill.rs,mcp.rs}`：`cs skill
  targets|list`、`cs mcp list|status <name>`、`cs doctor capabilities`（`doctor` 改为可选子命令，
  裸 `cs doctor` 语义与退出码不变）。
- i18n：`src-tauri/i18n/{en,zh-Hans,zh-Hant}.yml` 的 `cs:` 段新增 7 个键（三语同构）。
- 桌面 Vue 渲染：**未实现**（Agent Skills 页面属 U-7）。Tauri 只读 DTO 已就绪，但本切片没有
  可见 UI，故 AC-8 未满足、V-9 未执行。
- U-4（Skill 安全门，已实现并有测试，但尚未经 adapter 暴露）：
  - `capability/skill/source.rs`：`SkillSource` 严格 tagged enum（`local_directory`/`local_zip`/
    `github`/`installed`），`deny_unknown_fields`；GitHub owner/repo/ref/path 校验（含 traversal、
    绝对路径、控制字符）；下载 URL 只能由 `GITHUB_DOWNLOAD_HOST` 常量拼出；`validate_skill_name`
    实现 `^[a-z0-9]+(-[a-z0-9]+)*$`；`redacted_ref()` 只保留本地路径最后一段。
  - `capability/skill/staging.rs`：`StagingArea`（app-data 私有 0700 目录、同一 operation 目录已
    存在即 `busy`、operation id 过滤成单段、Drop 自动清理、`keep()` 保留给 doctor 诊断）。
  - `capability/skill/archive.rs`：ZIP 专用安全解包。路径（绝对/drive/UNC/反斜杠/`..`/NUL/控制字符/
    Windows 保留名/结尾空格点）、条目数 2048、单条目 16MiB、展开总量 64MiB、压缩比 200、
    case-fold 与归一化重名、符号链接与特殊文件（`classify_entry_kind`）全部 fail closed；任一
    条目不合格即整包拒绝，不做部分解包。
  - `capability/skill/checker.rs`：`SKILL_CHECKER_VERSION = "skill-checker.v1"`；verdict
    `pass|blocked|inconclusive`、severity `block|inconclusive|warn`、permissions profile、
    findings（rule/severity/path/detail）、`content_digest`（基于文件清单 SHA-256）、
    `file_count`/`total_bytes`。规则：缺失/非法 manifest 与非法 name、凭据文件访问、敏感路径写入、
    提权、混淆载荷（base64 blob + eval/atob）→ blocked；非法 UTF-8/未知二进制、无法读取 →
    inconclusive；普通脚本/网络/下载/eval 使用 → warn 并记入 permissions。
    `SkillSourceResolver::check()` 负责 staging→解包→检查→清理；GitHub 下载失败/非 2xx/超限一律
    inconclusive（无网络即 fail closed）。
- 迁移/表：U-4 不新增表。
- U-5（安装计划与多目标落地，服务层）：
  - `capability/skill/ownership.rs`：`OwnershipMarker`（schema_version/installation_id/skill_name/
    target_id/checker_version/content_digest/manifest_digest/nonce/source_*），`write_marker`/
    `read_marker`/`has_proof`；marker 文件为安装目录内 `.chatspeed-skill.json`，**不计入内容
    manifest**（`manifest::is_marker_path` 同时在 compute/verify 两处跳过）；损坏 marker 一律
    `refused`，绝不当成「无归属」。
  - `capability/skill/plan.rs`：`SkillInstallPlan` 冻结 plan_id/skill_name/source/checker_version/
    verdict/content_digest/file manifest/target 选择/frozen_root；`build()` 要求 verdict=pass、
    checker_version 一致、target 非空且不重复，并把已检查内容复制进私有 frozen 目录（逐个文件
    边复制边校验 SHA-256），冻结后的 digest 必须与 check 报告一致；`verify_frozen()` 在 apply 前
    复查，源在 check 后被改写即 `refused`。
  - `capability/skill/installer.rs`：`SkillInstaller::apply()` 逐 target 处理，每个 target 先
    `record_effect_intent`、后 `record_effect_outcome`（effect 表对 operation 有外键，无法绕过
    journal）。同名已存在：有本机 ownership row + marker + manifest 全匹配 → `already_installed`；
    否则（含未托管、已漂移、无 marker）→ `skipped_existing`，内容零改动；symlink/非目录 → `blocked`；
    无法核实目录的 target → `unsupported`；提交用同目录 sibling temp（`.<name>.cs-install-<nonce>`）
    复制+逐文件校验，marker 写入 → upsert ownership row(installing) → rename → 置 installed；失败
    清理 temp。默认 target 只有 `chatspeed`。
- U-6（卸载与 quarantine 恢复，服务层）：
  - `capability/skill/uninstaller.rs`：`SkillUninstaller::uninstall()` 逐 target journal 后执行；
    删除前必须同时满足：保留名不在 `RESERVED_SKILL_NAMES`、target 可解析、存在 ownership row 且
    `row.install_path` 与目标路径一致、marker 与 row 完全匹配、内容 manifest 未漂移；任一不满足
    即 `refused` 且零删除。通过后 rename 到同 target 根下的 `.<name>.cs-quarantine-<nonce>`
    （同文件系统、原子，不依赖 app-data 与目标同盘），再把 row 置 `quarantined`，最后 `removed`
    并删除 quarantine；row 置 quarantined 失败会把目录 rename 回原位。row 已是 quarantined 时走
    `finish_quarantine()` 收敛残留，绝不二次删除；row 已是 removed 时返回 `not_found`；目录已被
    人工删除时只把 row 置 `removed`（`finalized`）。
- 迁移/表：U-5/U-6 不新增表（复用 v20 的 `skill_installations` / `capability_operation_effects`）。

operation/schema/migration（如有）：
- 迁移 v20（additive）：`capability_operations`、`capability_operation_effects`、
  `skill_installations` 及 3 个索引；不触碰任何既有表或行。
- fresh install 直接安装最新 schema（version 20）；v19 数据库增量升级到 v20 且保留既有行。

focused tests 与最后一次修改后的结果（全部在最终代码上运行）：

- `cd src-tauri && cargo test --lib -- --test-threads=1 capability::` → **85 passed, 0 failed**
  （其中 `capability::skill` 56 项：checker 9、archive 9、installer 7、plan 5、uninstaller 8、
  source 5、staging 3、ownership 2、manifest 2 等）。
- `cd src-tauri && cargo test --lib -- --test-threads=1 db::sql::migrations` → 17 passed，
  **1 failed**：`db::sql::migrations::v18::tests::creates_table_and_seeds_presets_once`
  （断言 seeded presets 11，实际 10）。已核对为**既有失败**：断言与 seed 源都位于
  `v18.rs`/chathub preset 提交（`a0b59f75`），本切片未触碰这些文件，且不属 Phase 3 范围，
  因此未修改，仅记录。
- `cd src-tauri && cargo test --lib -- --test-threads=1 workflow::react::skills` → 9 passed。
- `cd src-tauri && cargo test --bin cs -- --test-threads=1` → **128 passed, 0 failed**。
- `cd src-tauri && cargo check --lib` 与 `cargo check --bins` → 通过；唯一 warning 为既有
  `src/mcp/client/types.rs:175`（`McpClientInternal` 私有 trait 泄漏），非本次引入。
- 迁移套件中另有一处**既有**陈旧断言（`v17_database_upgrades_to_v18_...` 断言版本 18）已在本
  次改动中最小适配为 `latest_migration_version()`，因为它直接位于被扩展的迁移测试文件内。

desktop/CLI smoke 及 operation/server/skill ID（脱敏）：

- 桌面 smoke **未执行**：沙箱环境无法运行 GUI，也无法运行构建产物（宿主 glibc 低于构建环境）。
- `cs` 二进制**无法运行**（`GLIBC_2.38 not found`），因此没有真实 CLI 端到端 smoke。替代证据：
  HTTP 路由在真实 axum server + 真实 SQLite 上通过集成测试（含 401 与结构化 404）；CLI 侧用
  clap `Cli::command().debug_assert()` 与解析测试锁定命令树。
- 本切片没有 mutation 入口，未产生真实 operation ID；测试内 ID 形如 `op-skill-<uuid>`。

secret/范围审计：

- 静态检查：`src/bin/` 无 `MainStore`/`ToolManager`/`rusqlite`/`ChatState` 引用；`src/capability/`
  无自建数据库连接（只经共享 `MainStore`）；新 adapter 无直接 mutation primitive 调用。
- canary 测试：journal payload、MCP 投影 DTO、redaction 单测均断言 `canary` 不出现。
- 范围：未新增导出/导入、automation、外部实验数据、benchmark、marketplace、Skill
  overwrite/update 或 MCP tool execution；`work/` 既有改动保持原样。

未验证项、环境限制和剩余风险：

- **未开始**：U-7（Skills HTTP/Tauri/CLI 与 Agent Skills 桌面页）、U-8..U-12（MCP
  characterization/迁移/install/lifecycle/HTTP/CLI/UI）。
- U-4/U-5/U-6 已实现但**尚无 adapter 调用方**：`cs skill check`、check/install/uninstall 路由、
  Tauri 命令与桌面 Agent Skills 页都在 U-7，因此 AC-6、AC-8 尚未满足；安装/卸载目前只能由 Rust
  测试驱动，尚无用户可执行入口（AC-5、AC-7 在服务层成立，但未在桌面/CLI 暴露）。
- 因此以下 AC 仍未满足：AC-4 的桌面/CLI 侧、AC-6、AC-8、AC-9、AC-10、AC-12；AC-5/AC-7 仅在
  服务层与测试中成立；AC-13 仅在已实现面（journal / DTO / 日志 / 错误）成立。
- 未执行验证项：V-2 的 fault injection 部分、V-3 的 HTTP 面、V-5 的 rescan/doctor 收尾、
  V-6、V-7、V-8、V-9、V-10。
- 第一个 checker 已具备完整恶意 fixture 矩阵（traversal/absolute/drive/UNC/link/device/上限/
  case-fold 重名/凭据/敏感路径/提权/混淆/二进制）；ZIP 符号链接与设备条目用改写 central directory
  external attributes 的 fixture 覆盖（`zip` writer 会屏蔽类型位）。
- 外部 target 中 `codex`/`cursor`/`windsurf`/`cline`/`trae` 返回 `unsupported`
  (`path_not_verified`)：本环境未能核到官方 skills 目录约定，按 D-5 fail closed。

下一切片入口：

- 从 U-7 开始：把已完成的 `SkillSourceResolver::check()` / `SkillInstallPlan` /
  `SkillInstaller` / `SkillUninstaller` 接到 `CapabilityApplicationService` 的
  `skill.check|install|uninstall` 操作（含 idempotency key、operation 状态与 rescan），再加 HTTP
  route、Tauri 命令、`cs skill check|install|uninstall` 与独立 Agent Skills 桌面页；完成后 3B
  才算可验收。注意 `SkillSourceResolver::check()` 目前会在返回前清理 staging，安装需要先补一个
  保留 staging 的 `materialize()` 入口。

## 11. As-built Implementation Record — 3A / 3B / 3C（追加于 2026-09-19）

本节是 3A、3B、3C 的 as-built 记录，**取代第 10 节末尾与"下一切片入口"中的旧缺口清单**
（那些清单描述的是 U-6 完成时点的状态）。历史小节不做改写。

### 11.1 交付摘要

- **3A（capability contract + journal + 只读面）**：完成。`CapabilityApplicationService` 是
  Skills/MCP 的唯一 mutation service；additive SQLite v20 保存
  `capability_operations` / `capability_operation_effects` / `skill_installations`；只读
  inventory、target registry、MCP desired/runtime/tools 投影与 doctor 已通过 Tauri、HTTP、
  `cs` CLI 与桌面暴露。
- **3B（文件型 Agent Skills）**：完成。source resolver（local dir / ZIP / 受约束 GitHub）、
  私有 staging、ZIP guard、非 LLM deterministic checker、immutable install plan、多目标
  atomic apply、ownership manifest、ownership-bound uninstall、doctor recovery，以及
  `cs skill check|install|uninstall` 与独立 Agent Skills 桌面页。
- **3C（MCP）**：服务层、旧 Tauri command 迁移、HTTP、CLI、桌面投影完成。install 严格
  descriptor、默认 disabled、无 runtime/network effect；uninstall 为 desired disabled →
  有界确认 stop → 删除 persistence；status/tools/refresh 分层且有界；旧命令名与 wire 保留，
  command-owned detached `tokio::spawn` 与直接 store/runtime mutation 已全部删除。

### 11.2 关键实现位置

Rust（新增）：

- `src-tauri/src/capability/{mod.rs,types.rs,error.rs,operation.rs,repository.rs,redaction.rs,
  doctor.rs,skill_inventory.rs,targets.rs,mcp_service.rs}`
- `src-tauri/src/capability/skill/{mod.rs,source.rs,staging.rs,archive.rs,manifest.rs,checker.rs,
  plan.rs,installer.rs,ownership.rs,uninstaller.rs,orchestrator.rs}`
- `src-tauri/src/capability/mcp/{mod.rs,repository.rs,runtime.rs,descriptor.rs,orchestrator.rs,
  tests.rs}`
  - `mcp_install` 141、`mcp_install_config` 164、`mcp_enable` 255、`mcp_disable` 405、
    `mcp_uninstall` 473、`mcp_refresh_tools` 591、`mcp_tools` 685、`mcp_tool_declarations` 695、
    `mcp_status` 730、`mcp_status_all` 737、`mcp_restart` 749、`mcp_update` 861、
    `mcp_set_tool_disabled` 968、`stop_and_confirm` 1135
- `src-tauri/src/commands/capability.rs`（只读 Tauri adapter，`capability_mcp_servers` 55）
- `src-tauri/src/db/sql/migrations/v20.rs`
- `src-tauri/src/bin/cs/{skill.rs,mcp.rs,capability.rs}`

Rust（修改）：

- `src-tauri/src/commands/mcp.rs`：11 个命令中 9 个改为 facade 委托；`list_mcp_servers` 与
  `run_mcp_tool` 保持原实现（读投影与既有工具执行契约，INV-2/U-12）。
- `src-tauri/src/tools/tool_manager.rs`：删除 `ops_in_progress`（被 service resource-key lock
  取代；删除前确认全仓无其他使用者）。
- `src-tauri/src/workflow/react/{application.rs,skills.rs}`、
  `src-tauri/src/workflow/react/client/http/{server.rs,dto.rs}`（MCP 路由 459-466，handler
  1362+）、`src-tauri/src/bin/{cs.rs,cs/args.rs}`、`src-tauri/src/lib.rs`（State 12）、
  `src-tauri/src/db/sql/migrations/{mod.rs,manager.rs}`。

前端：`src/components/setting/AgentSkills.vue`、`src/stores/capability.js`、
`src/libs/capability.js`、`src/components/setting/Mcp.vue`（runtime-facts 行）、
`src/stores/mcp.js`（列表刷新同时刷新投影）、`src/views/Settings.vue`、
`src/i18n/locales/{en,zh-Hans,zh-Hant}.json`、`src-tauri/i18n/{en,zh-Hans,zh-Hant}.yml`。

### 11.3 Acceptance Criteria 状态

| AC | 状态 | 证据 |
|---|---|---|
| AC-1 | 满足 | `capability::mcp::tests::adapters_never_call_an_mcp_mutation_primitive_directly`（源码级 guard：commands/HTTP 无 mutation primitive 调用、无 `tokio::spawn`；CLI 无 MainStore/ToolManager/rusqlite/db 依赖）；Tauri+HTTP 同一 facade（`application.rs` State 12 单一实例） |
| AC-2 | 满足 | `capability::mcp::tests::install_replays_one_key_and_conflicts_on_a_changed_request`、`startup_recovery_classifies_an_mcp_operation_with_an_unproven_effect`、`db::sql::migrations::manager::tests`（fresh/v19→v20/rollback/ahead-version）；HTTP 层 `mcp_routes_install_disabled_replay_and_never_expose_a_secret` |
| AC-3 | 满足 | `capability::skill::*` target 矩阵测试 + `libs/capability.test.js`（默认选择只有 ChatSpeed）+ AgentSkills 契约测试 |
| AC-4 | 部分满足 | registry 固定 8 个 ID，逐目标结果、同名 `skipped_existing`、未核实路径 `unsupported(path_not_verified)` 均有测试；但本环境只能核到 `agents`/`claude-code` 的正式约定，`codex`/`cursor`/`windsurf`/`cline`/`trae` 目前返回 `unsupported`（按 D-5 fail closed，未猜路径）。需在这些工具官方约定可核实后再启用 |
| AC-5 | 满足 | `capability::skill::checker` 恶意 fixture 矩阵 + `a_blocked_skill_source_is_refused_over_http`（HTTP 面 blocked 零安装） |
| AC-6 | 满足 | `POST /control/v1/skill-check`、`cs skill check`、桌面 Check 按钮共用 `SkillChecker`（`checker_version = skill-checker.v1`），安装复用同一 checker |
| AC-7 | 满足 | `capability::skill::uninstaller` 与 `capability::skill::*` drift/refusal 测试；HTTP `skill-uninstall` 只删 ownership 已证明内容 |
| AC-8 | 满足 | `src/components/setting/AgentSkills.vue` + `AgentSkills.contract.test.js`（与 prompt 型 `stores/skill` 分离、状态齐全、i18n 三语一致） |
| AC-9 | 满足 | `descriptor.rs` 11 个 strict 解析测试；`install_registers_disabled_and_performs_no_runtime_effect`；HTTP 路由测试断言 disabled、零记录重复、SSE 422 |
| AC-10 | 满足 | `uninstall_disables_then_stops_and_only_then_deletes`、`uninstall_keeps_the_record_when_the_stop_cannot_be_confirmed`、`an_unconfirmed_stop_leaves_the_disabled_record_in_needs_reconcile`；真实子进程面 `a_real_stdio_child_that_cannot_handshake_is_never_reported_running` |
| AC-11 | 满足 | `listing_tools_never_starts_or_invokes_anything`、`a_server_the_runtime_does_not_know_reports_a_stable_empty_result`、`a_failed_refresh_keeps_the_last_known_snapshot_and_marks_it`、`a_refreshed_list_is_reported_fresh_afterwards`；`cs mcp status/tools/refresh` |
| AC-12 | 满足 | 同一 service/DTO：桌面 `Mcp.vue` 读 `capability_mcp_servers` 投影，CLI 读 `/control/v1/mcp-servers`；`Mcp.contract.test.js` 锁定投影来源与旧 wire 并存；`stores/mcp.js` 刷新列表时同步刷新投影 |
| AC-13 | 满足 | `the_journal_never_stores_a_descriptor_secret`、`a_secret_never_appears_in_a_refusal_message`、`env_must_be_pair_arrays_not_a_bare_object`；HTTP 测试直接扫描 `capability_operations/effects/skill_installations` 行文本无 canary；测试日志 canary 扫描无命中；无 shell 拼接、无 argv secret |
| AC-14 | 满足 | `the_capability_surface_does_not_grow_an_export_or_automation_path` + 全量 diff 审查：`run_mcp_tool`/`list_mcp_servers` 未改动，`Cargo.toml` 未改动，无 export/import、automation、benchmark、marketplace、Skill overwrite/update 改动 |

### 11.4 Protected Invariants

- INV-1：`MainStore`/ConfigCache/`ToolManager`/MCP child 仍只在桌面主进程；CLI 是 HTTP client（guard 测试）。
- INV-2：11 个旧命令名与参数保留；`Mcp.contract.test.js` 断言页面仍调用同名命令。
- INV-3：Agent Skills 与 `ai_skill` 无共享代码路径（`AgentSkills.contract.test.js` 禁止 `stores/skill`）。
- INV-4：install 只接受 `verdict == pass`；无 force/LLM 旁路参数。
- INV-5：默认 target 集合恒为 `chatspeed`；外部 target 需显式传入。
- INV-6：同名普通目录只 skip；非托管/drifted 内容零删除。
- INV-7：desired/observed/freshness 分层；`an_answered_observation_proves_a_missing_server_is_stopped` 与 `desired_and_observed_are_reported_separately` 固定"未观察 ≠ 已停止"。
- INV-8：无法证明的 effect 一律 `needs_reconcile`，doctor 收敛；不盲重试。
- INV-9：Phase 2 run-scoped experiment capability（`workflow/react/engine.rs`）未改动。
- INV-10：无新增导出/导入，未删除既有 artifact/评测能力。

### 11.5 验证执行记录

| V | 命令 | 结果 |
|---|---|---|
| V-1 | `cargo test --lib -- --test-threads=1 capability` | 139 passed |
| V-2 | `cargo test --lib -- --test-threads=1 db::sql::migrations::v20 db::sql::migrations::manager` | 6 passed（含 v19→v20 增量、失败回滚、ahead-version ensure） |
| V-3 | `cargo test --lib -- --test-threads=1 workflow::react::skills capability::skill` | 72 passed |
| V-4 | 同上（target matrix 测试在 `capability::skill::*` / `capability::targets`） | passed；`codex`/`cursor`/`windsurf`/`cline`/`trae` 断言为 `unsupported` |
| V-5 | 同上 + HTTP `capability_mutation_routes_...`、`a_blocked_skill_source_is_refused_over_http` | passed（临时 CHATSPEED_HOME 下 check→install→scan→uninstall→doctor） |
| V-6 | `cargo test --lib -- --test-threads=1 capability::mcp` | 41 passed（fake repository/runtime：timeout、error、duplicate、并发串行化、重启恢复、freshness revision） |
| V-7 | `cargo test --lib -- --test-threads=1 mcp_routes_install_disabled a_real_stdio_child` | 2 passed。**部分覆盖**：见 11.6 |
| V-8 | `cargo test --lib -- --test-threads=1 workflow::react::client::http`、`cargo test --bin cs` | 39 passed；140 passed |
| V-9 | `pnpm test:capability`、`pnpm build` | 21 passed；build 成功（chunk-size 提示为既有） |
| V-10 | canary 扫描 `dev_data/logs/*` 无命中；journal 行扫描无命中；diff 范围审查通过。**真实桌面 GUI 交错 smoke 未执行** | 部分完成，见 11.6 |

回归确认：`cargo test --lib -- tools::tool_manager commands:: db::mcp` → 103 passed（删除
`ops_in_progress` 与迁移命令后无回归）。`cargo check --lib` / `--bin cs` 仅剩 1 个与本阶段无关的
既有 privacy warning。

### 11.6 与计划的偏差、限制与剩余风险

偏差（局部实现细节，未改变策略/验收）：

1. U-10/U-11 计划的 `installer.rs` / `lifecycle.rs` 合并进 `capability/mcp/orchestrator.rs`
   （模块划分属 U-1 允许范围）；`capability/mcp_service.rs` 承担投影与 drift 分类。
2. MCP 未新增 `capability/mcp/service.rs` 文件名，改用 `orchestrator.rs`；`mcp_service.rs` 为
   read projection。
3. 桌面 install 入口复用旧 `add_mcp_server`/`update_mcp_server` 命令（其 body 已委托 service），
   因此未新增桌面 install Tauri 命令名；legacy 表单继续用 `check_form` 校验，strict descriptor
   仅用于 HTTP/CLI 新入口，避免对既有页面收紧 URL 规则造成破坏性变更。
4. 开机时 desired-enabled 的启动仍在 `ToolManager` 初始化路径（同一唯一 owner、`!disabled`
   过滤），未改写为 journaled operation，以免每次启动产生噪声 operation；用户发起的 mutation
   才进入 journal。
5. 删除了 `ToolManager::ops_in_progress`（迁移后全仓无使用者）。

未验证 / 限制：

- **V-7 部分**：仓库内没有可作为子进程运行的真实 MCP server fixture，因此"成功启动一个真实
  MCP 子进程并列出其工具"这一路径未验证。已验证的是：install 零进程、禁用态 tools 稳定为空、
  真实 runtime port 下握手失败的子进程被判为失败且投影报 `desired_enabled_not_running`、
  且仍可卸载；成功启动路径由 scripted runtime 的状态机测试覆盖。补齐需要一个 test-only MCP
  fixture bin（新增 Cargo 目标），需另行确认。
- **V-10 部分**：本环境无显示会话，未执行真实桌面 GUI 与 CLI 交错操作/重启 smoke。
  HTTP/CLI/service 三层已在真实 SQLite + 真实文件系统（临时 CHATSPEED_HOME）下验证。
- **AC-4 部分**：`codex`/`cursor`/`windsurf`/`cline`/`trae` 目录约定未能在此环境核实，按 fail
  closed 返回 `unsupported`，未写入任何外部目录。
- 预存在、非本阶段引入的失败：`db::sql::migrations::v18::tests::creates_table_and_seeds_presets_once`
  （commit `a0b59f75` 改了 chat hub 预设种子数量但测试常量仍为 11）、`tools::fs::tests::test_edit_file_*`
  （依赖缺失的本地 fixture 文件）。
- 预存在的 i18n 结构缺口：`zh-Hans` 缺 `workflow.newWorkflowDialog.defaultTitle`（不在本阶段
  diff 内，未修改）。
- 观察项：headless 测试中 `ToolManager` 广播 MCP 状态时会记
  `Failed to broadcast ...: channel closed`（无 webview 订阅者，非本阶段引入，桌面场景有订阅者）。

结论：3A 与 3B 达到可验收状态；3C 达到功能与安全性可验收状态，但**在补齐 V-7 成功启动 fixture
与 V-10 真实桌面 smoke 之前，不应把 Phase 3 记为最终关闭**。下一入口：3D automation（须待上述
两项验证补做或在记录中明确豁免后开始）。

## 12. 审查整改轮次记录（追加于 2026-09-19）

本节记录针对最终审查两项 major 整改（不改动第 11 节历史结论，只追加）。

### 12.1 整改一：MCP 公共读取 DTO 全链路脱敏（AC-13）

- `commands/mcp.rs::list_mcp_servers` 不再返回含原值的 `Vec<Mcp>`，改为委托共享 service 的
  `mcp_records_redacted()`；`add`/`update`/`update_tool_status` 返回经 `mcp_record_redacted()`
  脱敏的记录。`redact_record_secrets` 去除 `bearer_token` 与 env 原值，仅保留可编辑非敏感字段。
- `mcp_update` 实现 presence/显式替换语义：省略的 secret 保留、显式提供的才替换；前端编辑表单
  只表达 secret/env 存在性与显式替换提示（`Mcp.vue`、`stores/mcp.js`）。
- 命令实际序列化路径回归测试：`capability::mcp::tests::the_desktop_read_path_serializes_no_secret_canary`
  通过真实 `CapabilityApplicationService`（fake repository）注入 `CANARY-BEARER`/`CANARY-ENV`，
  调用 `mcp_records_redacted()` 并 `serde_json::to_string` 断言两处 canary 均不出现；
  `commands/mcp.rs` characterization 断言 raw 输入形状仍含字段而 read model 不含。
- 桌面 status 叠加脱敏（补充）：`list_mcp_servers` 会把 ToolManager 的实时 `McpStatus` 覆盖到每条
  记录上，而 `McpStatus::Error` 的原始 message 由 client 用配置值（bearer/env/URL userinfo）拼接，
  属公共 DTO 泄漏。新增 `capability/mcp_service.rs::public_runtime_status`，把 `Error(_)` 的 message
  替换为稳定非敏感码（保留 `{"error":...}` 线形），并把叠加逻辑抽成 `commands/mcp.rs::overlay_redacted_status`
  纯函数；回归测试 `commands::mcp::characterization::the_desktop_list_response_carries_no_secret_in_the_overlaid_status`
  对真实命令的 `Vec<Mcp>` 序列化断言 config + URL userinfo + inline token + bare bearer 多重 canary 均不出现。

### 12.2 整改二：durable、service-owned 的 doctor/reconcile 收敛（AC-2、AC-7）

- 新增 `capability/reconcile.rs`：`CapabilityApplicationService::reconcile()`，证据驱动、幂等、
  单向收敛。顺序为 journal 分类（复用 `recover_interrupted_operations`，幂等）→ Skill 残留 →
  私有 staging 残留 → operation 效果收敛。`CapabilityReconcileReport` 以稳定 snake_case 输出
  `quarantines_finalized`/`installs_recovered`/`staging_residue_removed`/`mcp_effects_recovered`/
  `still_needs_reconcile`/`findings`。
- 收敛规则（仅在可证明时推进，否则保持 `needs_reconcile`，绝不盲重试或删除）：
  1. `Quarantined` 行 → 删除 ChatSpeed 移入的同级 quarantine 目录并把行记为 `Removed`
     （`SkillUninstaller::reconcile_owned_directory`）；
  2. `Installing` 行且内容+ownership marker+manifest 仍证明安装 → 记为 `Installed`；漂移/未知零删除；
  3. 私有 staging 中无存活 operation 归属且超过 600s grace 的目录才清理；
  4. MCP `start/stop/delete` 效果，须持久化与 runtime 观测共同证明（`delete` 还须记录确已消失、
     runtime 非运行；timeout 视为 unknown 而非已停止）；无法证明的效果保留 `needs_reconcile`。
  5. 一个 operation 的所有效果被证明后由 `needs_reconcile` 收敛为 `Completed`。
- 仅对 `resolve_targets()` 判定 supported 的 target 做文件系统探测（INV-5/D-5）。
- 支持钩子：`StagingArea::sanitize_operation`（跨重启稳定 staging 目录名关联）。

### 12.3 四端接入（AC-1）

- Tauri：`commands/capability.rs::capability_reconcile`（与只读 `capability_doctor` 并列，注册于
  `lib.rs` invoke_handler）。
- HTTP：`POST /control/v1/capability-doctor/reconcile`，Bearer + Origin + 强制 Idempotency-Key，
  经 `with_idempotency` 复用统一 mutation 契约，输出稳定 snake_case。
- CLI：`cs doctor reconcile [--idempotency-key]`（`DoctorCommand::Reconcile`），缺省自动 mint key；
  human/json/jsonl 渲染 `human_reconcile`。只读 `cs doctor capabilities` 与裸 `cs doctor` 行为不变。
- 桌面：Agent Skills 页新增“收敛能力漂移”入口 + 结果展示，`stores/capability.js::reconcile()`
  调用同一命令并刷新 inventory 与 doctor；三语 locale（Rust yml 与前端 json）同构、key 有序。
- 开机后在异步线程一次性执行 `reconcile()`（`lib.rs`），可收敛崩溃遗留 quarantine/staging/MCP drift；
  无法证明的效果仍保留 `needs_reconcile`。

### 12.4 新增 focused 测试

- `capability::reconcile::tests`（5）：quarantine finalize 并完成对应 operation（成功收敛）、
  interrupted install 恢复、漂移 install 不删除/不收敛、staging 残留清理与近期目录保留、空闲 journal 为 no-op。
- `capability::mcp::tests` reconcile（4）：proven start/stop 收敛为 Completed、record 仍在时 delete
  保留 needs_reconcile 且 delete_calls=0、runtime 仍 running 时 stop 保留 needs_reconcile。
- CLI：`cs doctor reconcile` 解析与 `--idempotency-key`、`human_reconcile` golden 渲染；
  前端 `AgentSkills.contract.test.js` 增加 reconcile 经由唯一 service 的断言。

### 12.5 整改轮验证记录

| 检查 | 命令 | 结果 |
|---|---|---|
| capability 全量 | `cargo test --lib -- --test-threads=1 capability` | 151 passed |
| MCP service（含新 reconcile + 命令面 canary） | `cargo test --lib -- --test-threads=1 capability::mcp::tests` | 29 passed |
| reconcile（Skill） | `cargo test --lib -- --test-threads=1 capability::reconcile` | 5 passed |
| HTTP 控制面（含 reconcile 路由） | `cargo test --lib -- --test-threads=1 workflow::react::client::http` | 39 passed |
| CLI 全量 | `cargo test --bin cs` | 141 passed |
| MCP command characterization | `cargo test --lib -- --test-threads=1 commands::mcp` | 8 passed（含新增 desktop status 叠加 canary） |
| 前端 focused | `node --test`（capability/AgentSkills/Mcp contract） | 22 passed |
| 前端构建 | `pnpm build` | 成功（仅既有 chunk-size 提示） |
| 编译 | `cargo check --lib --bin cs` | 仅剩 1 个与本阶段无关的既有 privacy warning |

### 12.6 剩余限制（不变项仍适用）

- 两项 major 整改已在服务端/CLI/桌面/HTTP 四端接通并有 focused 证据。第 11 节列出的 V-7 成功启动
  真实子进程 fixture、V-10 真实桌面 GUI 交错/重启 smoke 仍未执行；在其补齐前 Phase 3 不记为最终关闭。

## 12.7 审查整改轮次记录二：`mcp_update` 停止未确认不得 swap 新配置（AC-10 / AC-12 / INV-8）

- 缺陷：`capability/mcp/orchestrator.rs` 的 `mcp_update` 忽略 `stop_and_confirm` 的 `false` 返回值。停止
  超时/未知时仍写入新配置并在 desired enabled 时 `start`，可能旧进程存活 + 新进程重复、DB/ConfigCache/
  ToolManager 投影分裂，且 operation 被误报完成；reconcile 无法处理该阶段。
- 修复（对齐 `mcp_restart` 既有正确门控）：持久化 desired（`mcp.update`=Applied）后、启动新配置前强制
  检查停止确认；未确认时**不启动新配置**、`require_reconcile("update_stop_unconfirmed")` 并返回结构化
  `NEEDS_RECONCILE`，operation 保持 `needs_reconcile` 而非 `Completed`。journal 留下 `mcp.stop`=Unknown 的
  可证明证据。
- reconcile 收敛（按 re-review 修正）：`reconcile_operations` 对 `mcp.update` 走专用
  `reconcile_update_operation`，不再只观察。它先证明旧 runtime 已停止（`mcp.stop` 观测非 running），再
  以**持久化记录（含真实未脱敏 secret，非 journal 的 redacted 副本）为准**驱动 desired，绝不伪装完成：
  - desired enabled：新配置未运行则记录 `mcp.start` intent 并有界 `start` 后观察；证明 running 才
    `Completed`（report 记 `mcp.start` recovered）。启动**确定性失败** → `mcp.start`=Failed 且 operation 终止为
    结构化 `Failed`；启动**未知/超时/不可观察** → `mcp.start`=Unknown 并 `mark_needs_reconcile`
    （`update_reconcile_start_unconfirmed` / `update_reconcile_start_not_observable`），保留 `needs_reconcile`。
  - desired disabled：确认停止后直接 `Completed`，从不启动。
  - 无已证明的 stop 时不推进 swap（避免同一记录两个活客户端），保留 `needs_reconcile`。幂等：已
    Applied/Skipped 或已 Failed 的 `mcp.start` 不重复发起。start 只经唯一 `mcp_effects()` owner（INV-1）。
- 新增 focused 测试（`capability::mcp::tests`，本轮累计 +6，用既有 `stubborn_stop`/`start_result`/
  `silent_start` fake 机制覆盖"之前已启用/运行"这一此前从未进入 stop 分支的场景——既有 update 测试都用
  `disabled=true` 且不重命名，故缺陷此前未被捕获）：
  - `an_update_does_not_start_the_new_config_when_the_old_stop_is_unconfirmed`：live 返回 `NEEDS_RECONCILE`、
    calls 含 `stop:weather` 不含任何 `start:`、state=`NeedsReconcile`、reason=`update_stop_unconfirmed`、未证明
    effect 计数=1。
  - `an_update_confirms_the_old_stop_before_starting_the_new_config`：live happy path calls 严格为
    `["stop:weather","start:weather"]`、`Completed`。
  - `reconcile_converges_an_update_once_the_old_runtime_is_proven_stopped`：旧进程消失后 reconcile 先证
    `mcp.stop`、再**主动启动**新配置并观察到 running，`mcp_effects_recovered=[mcp.stop,mcp.start]`、`Completed`。
  - `reconcile_converges_an_update_to_a_disabled_desired_without_starting`：desired disabled → 收敛为停止且无 start。
  - `reconcile_fails_an_update_when_the_new_config_cannot_start`：start 失败 → `Failed`，非 `Completed`。
  - `reconcile_keeps_an_update_needs_reconcile_when_the_start_is_unobservable`：start 应答但不可观察 →
    保持 `NeedsReconcile`、reason=`update_reconcile_start_not_observable`，不误报完成。
- 本轮 focused 验证（追加证据，不改写第 11 / 12.5 历史）：`capability::mcp` 54 passed（含上述 6 项）；
  `capability::` 全量 147 passed；`commands::mcp` 8 passed（含 desktop overlaid status secret-canary）；
  `workflow::react::client::http` 39 passed；`--bin cs` 141 passed；前端 `pnpm test:capability` 22 passed；
  `pnpm build` 成功（仅既有 chunk-size 提示）；`cargo check --lib --bin cs` 仅剩 1 个位于
  `mcp/client/types.rs`、与本阶段无关的既有 privacy warning。
- 验证期间发现的既有无关失败：`db::sql::migrations::v18::tests::creates_table_and_seeds_presets_once`
  断言 11 而 seed 实为 10（`chat_hubs` 网页预置，与 Skills/MCP 能力无关，文件相对 HEAD 未改动，属基线
  既有问题）。未纳入本次整改范围，留待其 owner 处理。
- 剩余限制：第 5 轮审查的 update 生命周期缺陷与 re-review 的"reconcile 未启动新配置却误标完成"均已修复
  并回归（enabled 主动启动+观察、失败/未知不误报、disabled 不启动、无证明 stop 不推进 swap）。V-7 真实子进程
  stdio fixture 与 V-10 真实桌面 GUI 交错/重启 smoke 仍未执行，Phase 3 最终关闭仍需补齐这两项。

## 12.8 审查整改轮次记录三：live `mcp_update` start 阶段必须记录 durable `mcp.start` 并以 running 观察为完成门槛（AC-2 / AC-10 / AC-12 / INV-7 / INV-8）

- 缺陷：上一轮虽已修好"停止未确认不得 swap 新配置"，但 live `mcp_update` 在停止确认后对新配置调用
  `mcp_effects().start(...)` 时，只执行 `self.observe(&updated.name).await.ok().flatten()` 并**忽略观察结果**、也**未写入
  `mcp.start` intent/outcome**。因此即使 `start` 返回 `Ok(())` 但新 runtime 实际不可观察、已退出或状态查询失败，
  代码仍构造 `status:"updated"` 并把 operation 标为 `Completed`——DB desired/config 已更新而 ToolManager/runtime 可能
  未运行，违反 AC-2 的 effect 证据要求、AC-12 的 DB/cache/runtime 一致性与 INV-8 的不得伪装完成。该分支此前只有
  reconcile 路径被覆盖，live 分支无测试。
- 修复（`capability/mcp/orchestrator.rs`，约 984–1062 行）：将 start 分支改为完全镜像 live `mcp_enable` 的有界证据语义：
  `set_state("start")` 后先 `record_effect_intent(EFFECT_START)`；用 `with_timeout(effect_timeout, start())` 有界启动；
  - 启动**未回答**（`with_timeout` `Err`）：记 `mcp.start`=`Unknown`，`require_reconcile("update_start_timed_out")`，返回结构化
    `NEEDS_RECONCILE`，绝不伪装完成；
  - 启动**确定性失败**（`Ok(Err)`）：记 `mcp.start`=`Failed`，`finish_operation(Failed, status:"updated_but_start_failed")`
    并返回该错误（desired 已持久化，由 doctor 报告为 drift，而非 running）；
  - 启动 `Ok(Ok)`：单次 `observe` 判定 `is_running(state)`；**仅 running/connected 才**记 `mcp.start`=`Applied` 并
    `Completed`；否则记 `Unknown` 并 `require_reconcile("update_start_not_observable")`、返回 `NEEDS_RECONCILE`。
  这三条与 reconcile 的 update roll-forward 形成闭环：live 停留在 `needs_reconcile` 的新配置启用，后续由
  `reconcile_update_operation` 依 journal 内更新后 desired 再启动并观察，只有证明 running 才收敛为 `Completed`。
- 新增 focused 测试（`capability/mcp/tests.rs`，直接驱动 live `mcp_update`，非仅 reconcile）：
  - `an_update_that_cannot_observe_the_new_runtime_needs_reconcile`：`silent_start`，live 返回 `NEEDS_RECONCILE`、
    reason=`update_start_not_observable`、`mcp.start` effect=`Unknown`、calls 严格 `["stop:weather","start:weather"]`；
  - `an_update_start_that_never_answers_needs_reconcile`：新增 `start_hang` fake 钩子仅挂起 start，live 返回
    `NEEDS_RECONCILE`、reason=`update_start_timed_out`、`mcp.start`=`Unknown`（旧 stop 仍被确认，证明 gate 不误伤停止阶段）；
  - `an_update_start_that_fails_is_reported_failed_not_complete`：`start_result=Some(...)`，live 返回 `INTERNAL`、
    `list_needing_reconcile` 为空、operation=`Failed`、`mcp.start`=`Failed`、result status=`updated_but_start_failed`，
    确保确定性失败不被降级为"未知 reconcile"或伪装完成。
  - happy path `an_update_confirms_the_old_stop_before_starting_the_new_config` 仍严格 `["stop:weather","start:weather"]`
    且 `Completed`，证明新门槛只在真实 running 时放行、未回退正常流程。
- 本轮 focused 验证（追加证据，不改写第 11 / 12.5 / 12.7 历史）：`capability::mcp` 57 passed（含上述 3 项新测）；
  `capability::` 全量 150 passed；`commands::mcp` 8 passed（含 desktop overlaid status secret-canary）；
  `workflow::react::client::http` 39 passed；`--bin cs` 141 passed。`db::sql::migrations` 仍仅第 12.7 记录的既有无关
  v18 seed 计数失败（10 vs 11，文件相对基线未改动），其余 18 项含 v20 通过。
- 剩余限制（不变）：live update start 阶段现已具备 durable intent/outcome 且只在证明 running 后 `Completed`，re-review
  的该项 major 已闭环。V-7 真实子进程 stdio fixture 与 V-10 真实桌面 GUI 交错/重启 smoke 仍未在本环境执行，
  Phase 3 最终关闭仍需补齐这两项。

## 12.9 审查整改轮次记录四：refresh 未证明终态不再伪装成功 + reconcile 补齐 refresh 收敛与冷启动轮询（AC-2 / AC-10 / AC-11 / AC-12 / INV-7 / INV-8）

- 缺陷 1（L1/L2）：`mcp_refresh_tools` 在 effect 超时或确定性失败时仍返回 `Ok`（HTTP 200 / CLI exit 0），
  超时路径还直接 `finish_operation(NeedsReconcile)` 而不写 `reconcile_reason`；只按状态码或退出码分支的调用方会把
  一次未确认甚至失败的刷新当成成功，而 enable/disable/update/restart 在同等情况下都返回结构化 `NEEDS_RECONCILE`。
- 缺陷 2（M1）：reconcile 的通用 MCP effect 证明只识别 `mcp.start|mcp.stop|mcp.delete`，`mcp.tools.refresh` 的
  `Unknown` effect 永远无法被证明，operation 会**永久**停留在 `needs_reconcile`，每轮 doctor/reconcile 都报同一噪声。
- 缺陷 3（M2）：reconcile 的 `mcp.update` roll-forward 在启动新配置后只用**单次** `observe` 判定，未复用 live 路径的
  `wait_until_running` 有界轮询；冷启动（进程已被接受但尚未握手）会被误判为不可观察，收敛要等下一轮。

修复：

- `capability/mcp/orchestrator.rs`（refresh 尾段，约 634–717 行）：超时 → `mcp.tools.refresh`=`Unknown` +
  `finish_operation(NeedsReconcile, 结果)`（journal 保留 `kept_last_known` 与 `freshness=stale`）+
  `require_reconcile("refresh_timed_out")` + 返回 `NEEDS_RECONCILE`；确定性失败 → effect=`Failed` + 结构化失败结果 +
  返回底层错误；成功路径不变（`Completed` + `freshness=fresh`）。三种终态都不再向调用方伪装成功，最后已知快照仍可从
  operation 记录读到。
- `capability/reconcile.rs`：
  - 新增 `reconcile_refresh_operation`（574–687 行），并在 `reconcile_operations`（255–265 行）为 `mcp.tools.refresh`
    走专用 roll-forward：已持久化终态直接收敛；运行时被证明 running 则有界重发 `refresh_tools`（列工具从不执行工具），
    应答成功才记 `Applied` 并 `Completed`（此时 freshness 才是真的新鲜）；被证明未运行记 `Failed`（前提已消失，
    不可能再生效）；运行时无应答保持 `needs_reconcile` + reason `refresh_reconcile_unconfirmed`，绝不猜测。
  - `reconcile_update_operation` 的启动分支改用 `wait_until_running`（532 行），与 live 路径同一有界确认语义。
- `capability/mcp/orchestrator.rs:1375`：`wait_until_running` 由私有提为 `pub(crate)` 以共享（仅可见性变化，
  实现未改）。

新增 focused 测试（`capability/mcp/tests.rs`）：

- `a_refresh_that_never_answers_is_refused_and_needs_reconcile`（1678）：超时返回 `NEEDS_RECONCILE`、
  reason=`refresh_timed_out`、effect=`Unknown`、未证明 effect 计数=1、journal 结果 status=`refresh_unconfirmed`。
- `a_failed_refresh_is_refused_and_keeps_the_last_known_snapshot`（1634）：把原先断言 `Ok` 的用例改为错误契约，
  并断言 journal 仍保留 `kept_last_known` 与 `freshness=failed`。
- `reconcile_converges_a_refresh_once_the_runtime_proves_the_server_running`（1955）、
  `reconcile_fails_a_refresh_whose_server_is_not_running`（1987）、
  `reconcile_keeps_a_refresh_needs_reconcile_when_the_runtime_will_not_answer`（2018）。
- `reconcile_polls_until_the_updated_config_is_proven_running`（828）：`start_state=starting` +
  `running_after_observations=4`，证明一次 reconcile 就能收敛冷启动。

失败-通过证据（临时回退验证后已还原为修复实现）：把 reconcile update 启动分支临时改回单次 `observe`，
`reconcile_polls_until_the_updated_config_is_proven_running` 失败（`still_needs_reconcile` 非空）；把
`mcp.tools.refresh` 分支临时禁用、退回通用证明器，三项 refresh reconcile 测试全部失败。还原后全部通过。

本轮 focused 验证（在最终代码上运行）：`capability::mcp::tests` 45 passed；`capability::` 全量 167 passed；
`workflow::react::client::http` 39 passed；`commands::mcp` 8 passed；`--bin cs` 141 passed；
`pnpm test:capability` 22 passed；`cargo check --lib --bin cs` 仅剩既有无关 privacy warning；`git diff --check` 通过。

剩余限制（不变）：V-7 真实成功 stdio 子进程 fixture 与 V-10 真实桌面 GUI 交错/重启 smoke 仍未在本环境执行，
Phase 3 最终关闭仍需补齐这两项；AC-4 的 `codex`/`cursor`/`windsurf`/`cline`/`trae` 仍按 fail closed 返回
`unsupported(path_not_verified)`。

## 12.10 审查整改轮次记录五：reconcile 与实时 MCP mutation 共享锁域（AC-2 / AC-10 / AC-12 / INV-8）

- 审查发现：`reconcile()` 会执行 MCP `start`/`refresh_tools` 等 runtime effect，但此前未取得与实时
  `enable`/`update`/`refresh` 相同的 per-resource lock；恢复操作可能与桌面或 CLI mutation 交错，导致重复
  effect、运行时配置错配或将其他操作造成的 running 误判为当前 operation 已收敛。
- 审查发现：`mcp_update` 改名时只锁旧 name，另一调用可按新 name 进入并发 mutation；跨名操作也存在锁顺序不一致
  时的死锁风险。
- 修复：`mcp/orchestrator.rs` 新增 `lock_mcp_resources()`，按排序后的 MCP name 去重并统一转换为
  `mcp:<name>` journal lock key；`mcp_update` 同时锁旧名与新名；`reconcile_operations` 对 MCP operation
  复用同一锁域，Skill reconcile 保持原有路径不变。这样恢复与实时 mutation 不能在同一 MCP runtime 上交错，
  跨名锁获取顺序也稳定。
- 健壮性调整：`CapabilityApplicationService::mcp_servers()` 对全量 runtime observation 增加
  `status_timeout` 有界等待；删除无调用者的未脱敏 `mcp_records()` 和无调用者的 `mcp_status_all()`，避免
  后续 adapter 误用 secret-bearing API；保留桌面专用 `mcp_records_redacted()` 与共享 `mcp_servers()` 投影。
- 新增回归：`reconcile_serializes_runtime_effects_with_a_live_refresh` 验证恢复 refresh 与实时 refresh 的
  runtime effect 最大并发数为 1；已有改名/启动/停止/刷新状态机测试继续通过。
- 最终验证：`capability::mcp::tests` 与 `capability::reconcile` 共 51 passed；`commands::mcp` 与
  `workflow::react::client::http` 共 47 passed；`cargo test --bin cs -- --test-threads=1` 为 141 passed；
  `cargo check --lib --bin cs` 通过；`git diff --check` 通过。仅保留既有 `Message`/`McpClientInternal`
  privacy warning，未引入新的编译错误。
- 计划状态：AC-2、AC-10、AC-12、INV-8 的本轮并发整改已完成；V-7 真实成功 stdio fixture、V-10
  真实桌面 GUI 交错/重启 smoke 及 AC-4 中未核实的外部 target 仍是既有未完成验证/限制，不能据此把 Phase 3
  标记为最终关闭。

## Phase 3D as-built 记录（本机 Automation Facade）

本节只追加 3D 的实施事实，不改写上方 3A–3C 历史记录。

### 交付范围

建立 transport-neutral 的 typed `AutomationApplicationService` 作为本机 automation 的唯一 facade，
Tauri、HTTP、`cs` CLI、scheduler 与桌面 store 全部经它完成 `list/get/draft/apply/create/update/
enable/disable/run/runs/delete`，未引入第二 runtime owner 或第二状态机。

### 关键改动

- 新增 `src-tauri/src/workflow/automation/application.rs`：facade、plan/apply、revision/CAS、
  权限 non-escalation、durable receipt 复用、run lifecycle 结构化投影（`project_run`/`reconcile_all`）。
- 新增 `src-tauri/src/workflow/automation/errors.rs`：canonical snake_case 错误码
  （`invalid_request/not_found/conflict/revision_conflict/plan_expired/permission_expansion/busy/
  confirmation_required/needs_reconcile/internal`）。
- `types.rs` 增补 `AutomationSpec/AutomationDraftInput/AutomationPlanV1/AutomationApplyRequest/
  AutomationView/AutomationRunView/AutomationMutationResult/AutomationDispatchResult`，内部统一 snake_case。
- 新增 additive `migrations/v21.rs`（经 `manager.rs`/`mod.rs` 注册，head=21）：automation `revision`、
  run `trigger`/`dispatch_key` 与 scheduled slot 去重、跨 transport `automation_mutations` receipt；
  历史行使用安全默认值，未改写既有迁移。
- `db/automation.rs` 增加 revision CAS、原子 due-slot claim（单事务 claim+推进+插入 run）、
  mutation receipt reserve/replay/conflict、blocking-run 检测与 snapshot 读取。
- `scheduler.rs` 收敛为仅调用 `automation_dispatch_due`，删除自身 list→advance→run 并行编排。
- `commands/workflow_automation.rs` 保留旧 command 名与 camelCase wire（`compat_save`/`run_automation_now`
  等），并 additive 增加 `draft/apply/run_views`；`delete` 增加 `confirm` 参数。全部经 `svc.automation()`。
- `client/http/{server.rs,dto.rs}` additive 增加 `/control/v1/automation*` 路由：读免 key，写强制
  bearer/Idempotency-Key，统一 `automation_error_response` 状态映射，snake_case 输出、delete 需 `confirm`。
- `bin/cs/automation.rs` + `args.rs` + `cs.rs`：`cs automation` 全子命令，仅经认证 loopback HTTP，
  复用 CapabilityClient 风格 idempotency 生成与 human/json/jsonl 渲染，附源码 guard 测试。
- 前端 `stores/workflowAutomation.js` + `WorkflowAutomationEditor.vue` + `WorkflowSidebar`/`Workflow.vue` +
  三语 `i18n/locales`：新增 draft/apply 预览（plan hash、base revision、permission summary、warnings）、
  结构化 run 生命周期（含 `needs_reconcile`）、显式 destructive delete 确认；旧 save/list/run 行为保持。

### 执行内核单一化证据

`create_manual_run`（`service.rs`）是唯一 shell+workflow 执行内核；facade `automation_run`、
scheduler `automation_dispatch_due` 与旧 `run_automation_now` command 均路由到它，未出现第二执行路径。

### 验证证据

- `cargo test --lib -- --test-threads=1 automation`：32 passed（facade 单元、db CAS/claim/receipt/
  blocking-run、`runs` 结构化投影、HTTP automation 路由契约、既有 schedule/service 兼容测试）。
- `cargo test --bin cs -- --test-threads=1`：148 passed（含 `cs automation` 渲染、idempotency 生成、
  以及证明 CLI 无 MainStore/SQLite/ToolManager/进程执行 的源码 guard）。
- HTTP 契约测试覆盖：bearer、缺 key 拒绝、draft 无副作用且 hash 稳定、create→get→runs→
  未确认 delete 返回 `confirmation_required` 且不级联、确认 delete 成功并 404，读为 snake_case。
- 前端 `pnpm run build` 通过（仅既有 chunk-size warning）；三语 `automation` key 集一致（各 62 key），
  JSON 全部可解析。
- `cargo check --lib` 仅余既有 `McpClientInternal` privacy warning（非 3D 引入，先前 `get_run_row` 死代码已删除）；
  `git diff --check` 干净，改动文件全部在 3D 范围内，未触及 export/import、外部 benchmark 或第二 owner。

### 限制与既有问题

- 宿主无图形界面/真实 stdio MCP 环境：未运行 V-10 真实桌面 GUI 交错/重启 smoke；已用真实 SQLite +
  认证 HTTP + temp 文件集成测试替代，不将其记为 GUI smoke。
- 既有基线失败：`db::sql::migrations::v18::tests::creates_table_and_seeds_presets_once` 期望 chat_hubs
  11 条 preset，实际 10 条。该测试在内存库上仅应用 v18 自身 SQL，且 `v18.rs` 与 `chat_hubs` 均不在 3D diff 内，
  属 3D 之前的既有问题，不计入 3D 回归。
- 兼容取舍：桌面仍以 Tauri compatibility adapter（`compat_save`/`run_automation_now`/`set_enabled`）为写路径，
  但其内部一律归一到同一 facade，符合 INV-2；未在 3D 将桌面整体切到 HTTP。
- 阶段指针：3D 本机 automation facade 已实现并通过上述 focused 验证；是否把 Phase 3 记为最终关闭仍需人工复核
  上述限制项。

## 13. Phase 3D 审查整改轮次记录一：原子活动-run guard 与 run-now facade 归一（AC-1/AC-6/INV-2）

本节记录 3D final review 拒绝后的整改，只追加事实，不改写第 12 节 3A–3C 与上方 3D 初版记录。

### 拒绝结论与根因

final review 判定 3D 未达完成门槛，提出三条 major：

1. `create_manual_run` 先查 `automation_has_blocking_run` 再在另一条写操作中插入 pending run，
   两次并发手工请求可同时通过检查并创建两个活动 run（AC-6 不成立）。
2. `automation_dispatch_due` 的 `claim_due_automation_slot` 事务只校验 enabled/revision/next_run_at，
   未在 claim 事务内检查活动/未知 run，手工 run 存在时 scheduler 仍可再启一个（AC-6）。
3. 兼容命令 `workflow_automation_run_now` 直接调用 `service::run_automation_now`/`create_manual_run`，
   绕过 typed facade 写路径（AC-1/INV-2）。

### 整改实现

- **原子手工 run claim（DB 层）**：新增 `MainStore::claim_manual_run(automation_id, run_id, scheduled_for)`，
  在单个 `write_blocking` 事务内先 `COUNT` 活动/未知 run（`pending/starting/running/needs_reconcile`），
  仅当为 0 时插入 pending manual run 并 commit，否则 `Busy`。借助 SQLite 单写事务，两个并发手工请求
  最多一个成功（`db/automation.rs`）。
- **scheduler due-slot 活动-run guard**：`claim_due_automation_slot` 事务调整为「CAS 推进 → 同事务内活动-run
  计数 → 命中则不 commit 直接返回 `ClaimOutcome::ActiveRunExists`」。CAS 先判保证 stale 仍返回 `NotEligible`
  （既有去重测试不变），overlap 时回滚 next_run_at/revision，automation 保持 due 供下一 tick。
- **facade 归一 run-now**：删除 `service::run_automation_now`；Tauri 命令改为 `svc.automation_run_compat(...)`。
  `automation_run_compat` 的 mutation 只经 typed facade `self.automation_run(...)`（其内部拥有唯一
  `create_manual_run` 内核），再经 facade 读方法 `get_row`/`run_rows` 用返回的 dispatch view 重建旧 camelCase
  `WorkflowAutomationRunNowResult`（INV-9）。compat 不再直接调用 service kernel，命令层不再持有 automation 写逻辑，
  满足 AC-1/INV-2「所有 Tauri mutation 归一到同一 typed facade」。
- **dispatch_due**：新增匹配 `ActiveRunExists` → 记 `Skipped` 结果，scheduler 与手工/前序 run 双向不重叠。
- **清理**：`create_manual_run` 改用 claim，`db/mod.rs` 移除随之未使用的 `WorkflowAutomationRunInsert` re-export，
  测试引用改指 `crate::db::automation::WorkflowAutomationRunInsert`。

### 验证证据

- 新增 focused DB 测试全部通过：
  `db::automation::tests::manual_claim_is_atomic_and_busy_on_overlap`（第二并发手工请求 `Busy`，run 数=1；
  终态后可再 claim）、`db::automation::tests::scheduler_claim_skips_when_active_run_exists`
  （存在 running 手工 run 时 due claim 返回 `ActiveRunExists`，next_run_at/revision 未推进、run 数=1）；
  既有 `scheduled_slot_claim_is_atomic_and_unique` 仍 `NotEligible`，证明 stale 语义未回归。
- `cargo test --lib -- --test-threads=1 automation`：35 passed（含上述新增与 HTTP/前端 facade 契约）。
- `cargo test --bin cs -- --test-threads=1`：148 passed。
- 源码 wiring guard `workflow::automation::application::tests::legacy_run_now_routes_through_typed_facade`：
  断言 Tauri 命令仅调用 `svc.automation_run_compat`、不含 `run_automation_now`/`create_manual_run`；
  `automation_run_compat` 仅经 `.automation_run(&automation_id)` typed facade 且不含 `create_manual_run`；
  `automation_run` 是唯一持有 `create_manual_run` 内核之处。防止第二条公共 mutation path 回归。
- 源码 guard：`create_manual_run` 仅由 `application.rs` 的 `automation_run` 调用；
  `workflow_automation_run_now` 仅调用 `svc.automation_run_compat`；`run_automation_now` 已无引用。
- `cargo check --lib` 仅余既有 `McpClientInternal` privacy warning（本轮整改曾引入的
  `WorkflowAutomationRunInsert` unused re-export 警告已清除）；`git diff --check` 干净，改动限定在
  `db/automation.rs`、`db/mod.rs`、`workflow/automation/{service.rs,application.rs}`、
  `commands/workflow_automation.rs`，未触及普通 workflow、run-scoped experiment 与 3A–3C capability 契约。

### 剩余限制

- 仍以 SQLite 单写事务作为并发 authority；若未来出现多主进程写同一 DB，需升级为 lease/owner 设计（A-4）。
- GUI/真实重启交错 smoke 仍受宿主限制未运行，以真实 SQLite 事务级测试替代，不记为桌面 smoke。
