# ChatSpeed `cs` CLI 第二期实施计划（评测与受控自我改进）

> 本文件是第二期（Phase 2）的**正式路线文档**与**逐阶段实施计划**。
> 第二期目标：在第一期已交付的 `cs` CLI、独立 loopback `/control/v1` HTTP/JSON + SSE control plane
> 和与 Tauri 共用的 workflow runtime authority 之上，建立"可验证的评测与受控自我改进"基础。
>
> 事实来源：`work/agent-cli-evaluation-self-improvement-design.md`、
> `work/agent-cli-evaluation-self-improvement-design-supplement.md`、
> `work/agent-cli-phase-1-implementation-plan.md` 及当前仓库实际代码。
> 命名映射：第一期文档使用 `agent-cli-phase-1-*`，本第二期沿用 `agent-cli-phase-2-*` 前缀。

## 0. 当前阶段指针（Current Stage Pointer）

- **2A 状态**：代码、离线验证与真实 desktop-owned CLI smoke **已完成**（51 个 `cs` focused tests +
  双 binary 无 warning + 一期 HTTP/前端回归 + `pnpm tauri dev` 实例验证）；详见 `## 9` 的
  `2A Implementation Record` 与 `work/agent-cli-phase-2-smoke-test.md`。
- **下一个入口**：`2C`（Experiment run，受控单次试验）。2B（Budget & effect admission ledger）已完成，
  Q-3 已在 2B 计划批准时确认（本地货币、token/resource-only、全资源 hard cap、默认不 retry）。
- **规则**：每个阶段完成后，必须在本文件末尾的 `## Implementation Record` 追加该阶段的实施记录，
  并把"当前阶段指针"推进到下一阶段。**下一阶段开始前不得清空或改写已完成阶段的 Implementation Record。**
- 只有当某阶段的 Acceptance Criteria、Protected Invariants、Execution Units 与 Verification 全部有证据，
  且没有 pending/in-progress 工作时，才允许把该阶段标记为完成并推进指针。

## 1. Problem Statement

### 1.1 第二期总体目标

第二期不允许直接从"运行 workflow"跳到"自动评测 / 自动候选 / 自动晋级"。必须先建立一条可审计、
可重放、可离线验证、有预算边界、有隔离 owner 的证据链，再在其上叠加评测与自我改进语义。
因此第二期被拆成 2A–2I 九个有明确进入/退出门槛的阶段，按依赖顺序推进。

### 1.2 分期总览（2A–2I）

| 阶段 | 主题 | 一句话目标 | 前置依赖 |
|---|---|---|---|
| **2A** | Artifact / Provenance + offline inspect/replay | 把已存在的 workflow session 固化为不可变、可离线验证、脱敏的 artifact bundle | 第一期 control plane（已完成） |
| **2B** | Budget & effect admission ledger | 建立 per-request / per-trial / per-candidate / per-campaign 的 reserve/commit/release 预算与 effect admission | 2A（artifact 契约稳定） |
| **2C** | Experiment run（受控单次试验） | 在预算 admission 之下，提供会触发 LLM/tool 的 experiment run，产出符合 2A 契约的 artifact | 2A、2B |
| **2D** | Correctness evaluator（只读打分） | 对 2A artifact 做确定性检查与可选 LLM judge，产出 correctness 事实，不做 promotion | 2A、2C |
| **2E** | Benchmark adapter + independent verifier | 接入 Harbor/Aider/SWE-bench/Terminal-Bench/private holdout，固定版本/digest/split，独立 verifier 产出分数 | 2A、2B、2C、2D |
| **2F** | Candidate / campaign / proposer | GEPA/DGM 式候选生成与 campaign 编排，仍受预算与 verifier 约束 | 2B、2C、2D、2E |
| **2G** | Isolated execution owner | worktree/patch apply、container 编排、MCP/skill 安装的隔离执行 owner | 2C、2F |
| **2H** | Headless / daemon + 独立实验 data domain | 非交互 headless owner、独立实验数据库/命名空间、CI 级可重放运行 | 2B、2C、2E、2G |
| **2I** | Promotion & self-improvement loop | verdict→apply→受控灰度→审计的端到端自我改进闭环 | 2D、2E、2F、2G、2H |

### 1.3 各阶段进入/退出门槛（Entry / Exit Gates）

- **2A**
  - 进入：第一期 control plane 提供认证 `meta`、权威 snapshot、分页 durable events 与 terminal usage。
  - 退出：`cs experiment capture/inspect/replay` 可用；artifact schema v1 固定；离线 verifier fail closed；
    脱敏与原子性有 focused tests；真实固定模型 smoke 有文档记录。**不引入任何新的外部 effect。**
- **2B**
  - 进入：2A 完成。
  - 退出：预算 reserve/commit/release 与 effect admission 有类型化 API、持久化与一致性测试；未定价/超限明确拒绝。
- **2C**
  - 进入：2B 完成（无预算 admission 不得注册 experiment effect）。
  - 退出：单次 experiment run 产出可被 2A verifier 验证的 artifact，且受预算约束；无 promotion。
- **2D**
  - 进入：2A、2C 完成。
  - 退出：evaluator 对 artifact 产出结构化 correctness 事实；judge 有版本与 prompt hash；无 promotion verdict。
- **2E**
  - 进入：2A–2D 完成，且 benchmark 版本/digest/split/verifier/资源预算已单独确认（见 Open Questions）。
  - 退出：至少一个 benchmark adapter + independent verifier 端到端产出分数，绑定到具体 artifact。
- **2F**
  - 进入：2B–2E 完成。
  - 退出：candidate/campaign 编排在预算与 verifier 约束下可复现；proposer 有边界。
- **2G**
  - 进入：2C、2F 完成。
  - 退出：worktree/patch/container/MCP 安装具备隔离 owner 与回滚。
- **2H**
  - 进入：2B、2C、2E、2G 完成。
  - 退出：headless/daemon owner 与独立实验 data domain 可运行，非交互可重放。
- **2I**
  - 进入：2D、2E、2F、2G、2H 完成。
  - 退出：promotion verdict→apply→灰度→审计闭环，且有完整 provenance。

### 1.4 本次执行范围（只展开 2A）

本次**只实现 2A**。2B–2I 仅在本路线文档登记，不进入本次代码、不注册、不声称支持。
2A 的完整 contract、执行单元与验证见本文件 `## 2` 之后的 2A 详细计划与 `## Implementation Record`。

## 2. 2A 范围边界与非目标（Non-Goals）

本次 2A **不实现、不注册、不声称支持**：

- `experiment run` 或任何新的 LLM/tool effect admission（真实 smoke 只复用一期已存在的 `cs workflow run`；capture 只读）；
- per-request/per-trial/per-candidate/per-campaign budget reserve/commit/release（属 2B）；
- benchmark adapter、Harbor、Aider、SWE-bench、Terminal-Bench、private holdout（属 2E）；
- correctness evaluator、LLM judge、独立 benchmark verifier、分数比较或 promotion verdict（属 2D/2E/2I）；
- candidate、campaign、GEPA/DGM、自动 proposer、worktree、patch apply（属 2F/2G）；
- headless/daemon、独立实验 data domain、容器编排、MCP/skill 安装（属 2G/2H）；
- 数据库 schema migration、实验索引表、Tauri command/event wire 修改、旧 HTTP/static/ccproxy router 修改。

## 3. 2A 目标行为与验收契约

### 3.1 目标行为

1. `cs experiment capture <session-id> --artifact-dir <dir>` 通过 `GET /control/v1/meta`、
   `GET /control/v1/workflows/{session_id}` 和带 `after`/`limit` 的 durable events 分页查询读取证据；
   不创建 session、不启动 workflow、不发 signal、不改服务端状态。
2. CLI 在目标目录同级 staging 目录生成 artifact，完成文件写入与 manifest/hash 校验后 atomic rename 发布；
   目标已存在、越界、symlink、大小/数量超限或写入失败时不发布可被当作有效的 artifact。
3. artifact 默认只保留结构化状态、ID/name、脱敏配置投影、hash、usage/cost 元数据与事件类型/顺序；
   prompt、响应、工具参数值、工具结果、环境变量、API key、Bearer token、private holdout 与敏感绝对路径不以原文写入。
4. `inspect`/`replay` 在无主进程、无 discovery、无数据库、无网络、无 LLM key 环境下工作；
   篡改文件、改动事件顺序、改变 session ID、缺少终端事件、manifest/hash 不一致、schema 不兼容或 provenance 不完整时 fail closed。
5. artifact 能区分 `complete`/`incomplete`/`invalid`/`incompatible`，报告结构化终态与 usage/cost 状态；
   未定价或 partial usage 显式为 `cost_status=unknown`，不生成 correctness 分数或 promotion 结论。

### 3.2 Acceptance Criteria

- **AC-1 — 路线拆分文档可续接**：新增本文件，明确 2A–2I 顺序、依赖、进入/退出门槛、当前阶段与回写要求；2B–2I 不进入本次实现。
- **AC-2 — 受控 capture 生成 artifact**：对现有 session 执行 capture 能读取权威 snapshot 与全部分页 durable events，
  生成 v1 固定布局 `run.json`/`snapshot.json`/`events.jsonl`/`result.json`/`artifacts/manifest.json`；非终态明确标记 `incomplete`。
- **AC-3 — provenance 与隐私边界可验证**：artifact 记录 session/agent/protocol/schema、状态、配置/模型/工具/策略/工作区/prompt 的 hash 或脱敏投影、
  durable event ID、事件版本、来源与 usage；不含 Bearer token、API key、完整环境变量、private holdout、未获允许的原始 prompt/response/transcript 或敏感绝对路径。
- **AC-4 — 离线 inspect/replay fail closed**：`inspect`/`replay` 在无主进程/无网络下验证 manifest、canonical hash、event hash chain、
  session binding、durable ID 顺序、终态事件与 usage 一致性；任何篡改/缺失/混入其他 session/不兼容 schema 返回稳定非零错误与机器可判定状态。
- **AC-5 — usage/cost 只做事实投影**：复用已有 `WorkflowUsageSummary`/model breakdown；正确报告 token、duration、pricing、partial/unpriced；
  `unpriced_tokens > 0` 或缺可重算依据时 `cost_status=unknown`，不当作零成本，不产生 correctness/promotion verdict。
- **AC-6 — 不产生新的 workflow effect**：capture/inspect/replay 不创建/修改 workflow，不调用 LLM/工具/MCP/signal/stop 或数据库写入；
  一期 `cs workflow`、Tauri UI、HTTP/SSE、snapshot、durable events、usage 语义保持不变。
- **AC-7 — 可执行验证与真实 smoke 有记录**：focused Rust tests、格式/双 binary 检查、前端 workflow 契约测试通过；
  必要时后台 `pnpm tauri dev` + `cs@free:ds-v4-flash` 短 workflow 完成 capture/inspect/replay 与篡改负向测试，回写 smoke 文档。
- **AC-8 — 第二期范围不越界**：本次 diff 不引入 benchmark/evaluator/verifier score、budget admission、headless/daemon、
  candidate/campaign/promotion、worktree、DB migration、Tauri wire 变化或旧 static/ccproxy router 改动。

### 3.3 Protected Invariants

- **INV-1 — 一期外部契约不变**：Tauri command 名、camelCase 参数/响应、`workflow://event/{session_id}`、`GatewayPayload`、
  `/control/v1` 既有 routes 与 CLI 既有 workflow 命令保持兼容。
- **INV-2 — 单一 runtime/data authority**：CLI 只通过 control plane 访问 `MainStore`/`WorkflowManager`/executor/`WorkflowRuntimeHub`；
  不打开 SQLite、不创建第二套 lifecycle/input/run loop。
- **INV-3 — 本阶段无新的外部 effect**：2A 只捕获已有 workflow；没有预算 admission 就不新增会触发 LLM/tool 的 experiment run/campaign/自动重试入口。
- **INV-4 — secret/private data fail closed**：token、API key、secret value、private holdout、完整环境变量与敏感原文不得进入 artifact；
  无法可靠脱敏时拒绝发布，不降级为原文写入。
- **INV-5 — artifact 原子性与不可伪造完成**：staging/最终目录分离；manifest/hash 未完成或校验失败的目录不能被标为 `complete`；文件与事件 hash 链不一致必须失败。

## 4. 2A 架构与数据流

### 4.1 选定方案

**CLI-local artifact capture + offline verifier/replay**：不新增 HTTP route、不新增主进程写盘服务、不改数据库 schema。
一期 control plane 已能提供认证 `meta`、snapshot、durable events，usage summary 已在 terminal boundary 持久化；
把 artifact 写入放在 CLI 本地可避免新增 runtime owner、数据库表、主进程生命周期与新权限边界。

### 4.2 数据流

```text
cs experiment capture
  ├─ discovery + Bearer auth
  ├─ GET /control/v1/meta
  ├─ GET /control/v1/workflows/{session_id}
  ├─ GET /control/v1/workflows/{session_id}/events?after=<id>&limit=500  (loop until exhausted)
  ├─ redact + project structured fields
  ├─ canonical JSON + SHA-256/domain-separated hash chain
  └─ staging directory -> verify manifest -> atomic rename -> artifact directory

cs experiment inspect/replay
  └─ local artifact only -> schema/manifest/chain/session/terminal/usage checks -> JSON/JSONL/human projection
```

注意：HTTP wire 上 snapshot/events 经 `to_snake_case_keys` 递归转换（含 `event_data` 内部键），CLI 侧一律按 snake_case 读取；
`after` 是 durable event ID（整数），SSE cursor 与 durable ID 严格分离。

### 4.3 Artifact schema v1（固定文件布局）

- `run.json`：`schema_version`、`artifact_kind`、`run_id`（2A 默认等于 workflow session ID）、capture timestamp、
  control-plane protocol/schema、server instance ID、agent ID、workflow session ID、terminal/artifact status、
  prompt/config/model/tool/policy/workspace 的 hash、source/provenance labels。
- `snapshot.json`：脱敏后的 workflow identity、status、wait reason、liveness、agent identity、redacted config projection、
  structured counts；不复制 messages、完整 `executionContext`、prompt/response 原文或工具结果原文。
- `events.jsonl`：每行字符串化 durable event ID、session ID、event type/version、created timestamp、
  `provenance=workflow_runtime`、安全结构化字段投影、`payload_hash`、`prev_hash`、`record_hash`；自由文本只保留 hash/长度/类型投影。
- `result.json`：artifact status、structured terminal status/event、usage summary projection、
  `cost_status=known|unknown`、`correctness_status=not_evaluated`、`promotion_status=not_applicable`、校验摘要。
- `artifacts/manifest.json`：相对文件路径、字节大小、SHA-256、schema/algorithm version、manifest status；不含 token、绝对 DB 路径或 secret。

脱敏规则、canonical JSON/hash、event hash chain、原子写入与限制见代码模块 `src-tauri/src/bin/cs/artifact.rs` 的实现记录。

### 4.4 状态与 authority

- workflow status/wait_reason/terminal event 来自 control-plane snapshot/durable events；不解析 assistant 文本决定终态。
- `WorkflowUsageSummary`/`task_completed.usage_summary` 是 usage authority；无 summary、`is_partial=true`、unpriced tokens 或不一致时报 `cost_status=unknown`。
- provenance 区分 `workflow_runtime`/`control_plane_snapshot`/`control_plane_durable_event`/`derived_redaction`；2A 不创建 `verifier`/`runner` correctness provenance。
- `inspect` 验证内部一致性；`replay` 只输出 timeline/status/usage 投影，不重放执行、不写库、不产 benchmark score。

## 5. 2A Decision / Uncertainty Ledger

### 5.1 Confirmed Decisions

- **D-1**：本次只实现 2A；2B–2I 只进入本路线文档。
- **D-2**：artifact 写入在 CLI 本地完成；不新增 control-plane endpoint、主进程 artifact writer 或 DB 表。
- **D-3**：artifact schema 使用 canonical snake_case JSON；跨 Rust/JSON 的 ID 与 durable event ID 均使用 string；SSE cursor 与 durable ID 分离。
- **D-4**：默认隐私策略是不保存原始 prompt/response/transcript、secret、token、完整环境变量、private holdout 与敏感绝对路径。
- **D-5**：usage/cost 只做事实投影；没有 request-level reserve/commit/release 就不提供 experiment effect admission，不把 unknown cost 当作零。
- **D-6**：真实 smoke 使用现有 `cs workflow run`，模型固定 `cs@free:ds-v4-flash`；`experiment capture` 本身只读。
- **D-7**：阶段完成后必须在本文件 `## Implementation Record` 追加记录（实际文件/符号、提交或未提交状态、验证命令/结果、真实 smoke、剩余风险）。

### 5.2 Assumptions

- **A-1**：现有 snapshot/events endpoint 能提供 2A 所需的 identity、structured state、terminal event 与 usage；字段缺失时优先标记 provenance/cost incomplete，不复制 runtime helper。
- **A-2**：现有 `sha2`/`uuid`/`tempfile` 满足 canonical hash、artifact ID 与测试目录需求；若必须引入新依赖，先停止并确认，不自动加依赖。
- **A-3**：默认 artifact 只需脱敏结构与 hashes，不需要原始 transcript/patch/benchmark verdict。
- **A-4**：允许在当前 desktop-owned control plane 上做开发机真实 smoke，但不把 smoke 结果包装成 headless/CI 保证。

### 5.3 Open Questions（后续阶段，不阻塞 2A）

- **Q-1（2E 前）**：benchmark 选择 Aider Polyglot / SWE-bench / Terminal-Bench/Harbor / 私有 holdout，需单独确认版本/digest、split、verifier 与资源预算。
- **Q-2（隐私扩展前）**：是否允许未来 artifact 保存原始 prompt/response、patch 或 transcript，需隐私与 retention 决策明确后单独扩展 schema。
- **Q-3（2B 前）**：预算单位、hard caps、未定价模型策略与资源限制需在任何新 experiment effect 之前确认。

### 5.4 Blockers / Stop Conditions

无未决 blocker。命中以下任一条件即停止相应 unit 并要求确认：需要新增 DB schema/Tauri wire/既有 control-plane endpoint 语义/static-ccproxy router/新 runtime authority；
需要把 capture 变成创建/启动 workflow、发 signal、调用 LLM/tool/MCP 或在无预算 admission 时注册 experiment effect；无法安全脱敏却准备写入原文；
需要引入新依赖/headless/container/worktree/benchmark/verifier/score/candidate/promotion；snapshot/durable events/structured state 不一致只能靠 assistant 文本补足；
覆盖既有未提交 `EnvGuard::_lock` 修改或无法隔离其他用户改动。

## 6. 2A Execution Map

- **U-1**：写入本路线文档并固化 2A 边界（AC-1、AC-7、AC-8；INV-1）。
- **U-2**：实现 artifact schema、脱敏、canonical hash、event chain、manifest、限制、staging/atomic writer 与 usage/cost projection（AC-2、AC-3、AC-5、AC-6；INV-2、3、4、5）。
- **U-3**：接入 `cs experiment capture|inspect|replay`；capture 复用既有认证 GET，inspect/replay 在 discovery 分流前离线执行（AC-2、AC-4、AC-6、AC-8；INV-1..5）。
- **U-4**：完成 focused verification、真实固定模型 smoke 并回写本文件 Implementation Record 与 `work/agent-cli-phase-2-smoke-test.md`（AC-1..8；INV-1..5）。

依赖：U-1 → U-2 → U-3 → U-4。

## 7. 2A Verification Strategy

- **V-1**：路线文档与范围审计（AC-1、AC-7、AC-8、INV-1）。
- **V-2**：artifact schema/redaction/hash golden 与 negative tests（AC-2、AC-3、AC-4、AC-5、INV-4、INV-5）。
- **V-3**：offline inspect/replay 无外部依赖验证（AC-4、AC-5、AC-6、INV-2、3、4、5）。
- **V-4**：CLI/build 质量与一期回归（AC-2、AC-4、AC-6、AC-8、INV-1、2、3）。
- **V-5**：真实 desktop-owned smoke 与阶段记录（AC-1..8、INV-1..5）。

### Acceptance Matrix

| Requirement | Units | Verification |
|---|---|---|
| AC-1 | U-1,U-4 | V-1,V-5 |
| AC-2 | U-2,U-3,U-4 | V-2,V-3,V-4,V-5 |
| AC-3 | U-2,U-4 | V-2,V-5 |
| AC-4 | U-2,U-3,U-4 | V-2,V-3,V-4,V-5 |
| AC-5 | U-2,U-3,U-4 | V-2,V-3,V-5 |
| AC-6 | U-2,U-3,U-4 | V-3,V-4,V-5 |
| AC-7 | U-1,U-4 | V-1,V-5 |
| AC-8 | U-1,U-3,U-4 | V-1,V-4,V-5 |
| INV-1 | U-1,U-3,U-4 | V-1,V-4,V-5 |
| INV-2 | U-2,U-3,U-4 | V-3,V-4,V-5 |
| INV-3 | U-2,U-3,U-4 | V-3,V-4,V-5 |
| INV-4 | U-2,U-3,U-4 | V-2,V-3,V-5 |
| INV-5 | U-2,U-3,U-4 | V-2,V-3,V-5 |

## 8. 阶段完成回写模板（Implementation Record Template）

每阶段完成后，在下方 `## Implementation Record` 追加一节，至少包含：

```text
### <阶段> Implementation Record (as built)
- 日期 / 当前阶段推进：<2A -> 2B>
- 实际文件与符号：<新增/修改文件、关键函数/结构>
- 提交/工作区状态：<是否 stage/commit；保留的既有未提交修改>
- 验证命令与结果：<cargo fmt/check/test、pnpm test:workflow、真实 smoke 命令与输出摘要>
- 真实 smoke：<session id、artifact 路径、模型、capture/inspect/replay/负向结果>
- 未验证项 / 环境限制 / 剩余风险：<明确列出>
- 下一阶段入口与前置：<2B 需要确认的 Q-3 等>
```

## 9. Implementation Record

（以下按阶段追加，已完成阶段记录不得清空。）

### 2A Implementation Record (as built)

- 日期 / 当前阶段推进：2026-09-14；2A 代码与离线验证完成，真实 desktop smoke 环境阻塞（见下）；下一入口 **2B**。
- 实际文件与符号：
  - 新增 `src-tauri/src/bin/cs/artifact.rs`：`SCHEMA_VERSION=1`、`ArtifactStatus{Complete,Incomplete}`、
    `TerminalStatus`、`code::*` 稳定 machine code、`ArtifactError`、`canonical_json`/`canonical_hash`/`domain_hash`/`blob_hash`、
    `redact_value`（SAFE_KEYS 白名单 + secret key/value + path/text 投影）、`build_event_chain`（durable id 升序、
    genesis prev、record_hash 绑定）、`terminal_status_from_event`（事件类型→terminal status）、
    `project_usage`/`project_totals`/`project_breakdowns`（cost_status known/unknown）、
    `project_snapshot`/`project_hashes`、`construct_bundle`、`build_manifest`、
    `write_bundle`（total/event/payload 限制 + staging + re-verify + atomic rename）、`verify_bundle_dir`
    （schema→manifest 固定集→session 绑定→chain→result 交叉绑定→**事件链派生状态 + run/result 严格交叉校验**）、
    `parse_artifact_status`/`parse_terminal_status`、`inspect_projection`/`replay_projection`/`error_projection`。
  - **verifier 加固（final review 修复）**：`verify_manifest` 强制 v1 固定文件集（`REQUIRED_FILES` 恰为
    `run.json`/`snapshot.json`/`events.jsonl`/`result.json`，拒绝缺项/重复/额外/绝对路径/`..` 穿越），
    使任何必需文件都无法被从 manifest 中移除以绕过完整性检查；`terminal_status_from_event` 显式映射
    `workflow_failed→failed`、`workflow_cancelled→cancelled`、完成类事件→completed，durable 终端事件优先于
    snapshot 状态，`complete` 仅在存在真实终端事件时成立（stale completed snapshot 无终端事件→`incomplete`）。
  - **verifier 状态交叉校验加固（re-review 修复）**：`verify_bundle_dir` 现从**已验证的 durable event chain**
    派生权威 terminal/artifact 状态（不再信任 `result.terminal_status`），并用 `parse_artifact_status`/
    `parse_terminal_status` 严格解析 `run.json` 与 `result.json` 的状态声明——缺失/`unknown`/非法值一律
    `Err(code::INVALID)` 非零退出；再交叉校验 run 与 result 必须互相一致、且必须与事件链派生值一致
    （含 failed/cancelled），任何不一致在返回 `Ok` 前拒绝；`cost_status` 仅接受 `known`/`unknown`。
    移除不再产生的 `ArtifactStatus::Invalid`、`TerminalStatus::Running` 变体与 `code::MISSING_TERMINAL`（防伪造完成）。
  - 新增 `src-tauri/src/bin/cs/experiment.rs`：`capture`（仅 `client.get` meta/snapshot/events，分页 `after`+`limit=500`
    直到取尽，durable id 不作 SSE cursor）、`inspect`/`replay`（离线，discovery 前分流）、human/json/jsonl 渲染、
    `fetch_all_events`、`to_cli_error`（保留 machine code 前缀）。
  - 修改 `src-tauri/src/bin/cs.rs`：新增 `mod artifact;`/`mod experiment;`、`run()` 在 `load_discovery` 前分流
    `Experiment::Inspect/Replay`，`Experiment::Capture` 在 client 分支处理。
  - 修改 `src-tauri/src/bin/cs/args.rs`：新增 `Command::Experiment` 与 `ExperimentCommand{Capture,Inspect,Replay}`；
    既有 workflow/agent/doctor 命令与参数不变（INV-1）。
  - 新增 `work/agent-cli-phase-2-implementation-plan.md`（本文件）、`work/agent-cli-phase-2-smoke-test.md`。
- 提交/工作区状态：**已提交**（用户显式要求，两个独立 commit）：
  - `615962ef` `feat(cs): add experiment artifact capture/inspect/replay (phase 2A)`（`cs.rs`、`args.rs`、
    `artifact.rs`、`experiment.rs` + 本文件与 `work/agent-cli-phase-2-smoke-test.md`）；
  - `57d30351` `fix(workflow): name EnvGuard lock field to clear test dead_code warning`（既有 `server.rs`
    `_lock` 测试辅助修改，作为无关的独立提交保留）。
  - 提交后工作区干净，无越界文件。
- 验证命令与结果：
  - `cargo fmt --all -- --check` → 干净（先 `cargo fmt --all` 归一）。
  - `cargo check --bin chatspeed --bin cs` → Finished，**无 warning**。
  - `cargo test --bin cs` → **51 passed; 0 failed**（含新增整体 usage/cost 一致性、snapshot 身份绑定、
    verifier 资源上限与 durable events 分页边界回归；以及 manifest
    固定集/绝对路径/重复负向、failed/cancelled 终端映射、事件链派生状态、run/result 状态交叉校验、
    缺失/非法 status 与非法 cost_status fail-closed、stale-completed→incomplete）。
  - `cargo test --lib workflow::react::client` → **36 passed; 0 failed**（含 server `_lock` 测试）。
  - `pnpm test:workflow` → **62 passed; 0 failed**。
  - 真实 CLI 进程：`cs experiment --help` 正常；`cs experiment inspect <missing> --output json` →
    `artifact_status=invalid, code=missing_file, correctness_status=not_evaluated, promotion_status=not_applicable`，**exit=1**，
    且未加载 discovery（离线）。
  - 结构证据：capture 仅 `client.get`；`artifact.rs`/`experiment.rs` 无 `MainStore`/SQLite/`WorkflowManager`/executor/
    `crate::workflow`/`crate::db`/`crate::commands` import。
- 真实 desktop smoke（V-5，2026-09-14）：已启动 `pnpm tauri dev`，`cs doctor` 认证连接
  `127.0.0.1:39361`（protocol v1，instance `5ac23af75d8809a83e0c0bebd15ab3b0`）。使用
  `builtin:coding`、`cs@free:ds-v4-flash`、`Reply with exactly: OK` 完成 session `0rm2f4v4r0400`；
  只读 capture 得到 complete artifact 和 8 条 durable events。以不存在的 discovery 文件运行
  inspect/replay 均成功，真实 artifact 隐私扫描无 prompt、Bearer/API key 或 workspace 绝对路径命中，
  capture 前后 workflow snapshot 字节一致。真实 usage 含 legacy breakdown 与 unpriced tokens，
  artifact 正确投影为 `cost_status=unknown`。详情见 smoke 记录。
- 未验证项 / 环境限制 / 剩余风险：
  - host CLI 生成的真实 artifact 临时目录在普通 shell 中以隔离的只读挂载可见，无法复制后对其进行
    手工篡改；真实 artifact 的离线正向/隐私/无副作用检查已完成，篡改 fail-closed 路径由 `cs` focused
    tests 覆盖（含刷新 manifest 后的 result usage 篡改）。
  - capture 的 snapshot 与分页 events 非同一事务快照；实施按 capture_timestamp + instance_id + event range 记录，
    读取期间 session 变化导致终态/usage 不一致时由 verify 的 result 交叉绑定/终端检查判 `incomplete`/`invalid`，不伪造一致。
  - 全库 clippy 既有 baseline 未扩大处理（本次仅保证新 CLI 模块无 warning）。
  - 脱敏为保守结构投影（默认不保存原文 transcript/patch/verdict，D-4/A-3）；如需原文留存走 Q-2 单独扩展 schema。
  - manifest_hash 为无密钥 canonical hash：2A 的离线 verifier 保证**内部一致性/防意外损坏与跨文件不一致**
    （强制固定文件集、路径穿越/绝对路径拒绝、event chain 与 result 交叉绑定、session 绑定、终端事件要求），
    但**不抵抗掌握整个目录并能一致重算所有 hash 的攻击者**（需签名/外部 trust anchor，属后续阶段单独引入）。
- 下一阶段入口与前置：**2B（Budget & effect admission ledger）**。进入前需确认 Q-3（预算单位、hard caps、
  未定价模型策略、资源限制）；在 2B 预算 admission 之前不得注册任何会触发 LLM/tool 的 experiment effect（INV-3）。

### 2B Implementation Record (as built)

- 日期 / 当前阶段推进：2026-09-14；2B 代码与 focused 验证完成；真实 experiment smoke 未执行
  （2B 不创建 experiment run，无 backend-owned experiment fixture 可触发真实 effect，见限制）；下一入口 **2C**。
- 实际文件与符号：
  - 新增 `src-tauri/src/budget/`（crate 级 canonical contract，`pub mod budget`）：
    - `types.rs`：`ScopeKind`（request/trial/candidate/campaign）、`ScopeChain`（opaque string ID + 校验）、
      `ResourceDimension`（11 维含 money）、`CapLimit::{NotApplicable,HardCap}`（禁止 omission=unlimited）、
      checked-integer `BudgetVector`（u64 + SQLite INTEGER range guard `ensure_sqlite_range`）、
      `ResourceCaps::check_admission`（committed+reserved+requested<=cap）、`PricingSnapshot`（整数 micro 计价 + source hash）、
      `MoneyMode::{TokenResourceOnly,Money}`（money cap 与 currency/pricing 成组出现）、`BudgetEnvelope::validate`
      （required dimension 必须有 hard cap、max_attempts>=1）、`ReservationState` 状态机（unknown 不可 release）、
      `ReserveEffect`/`Reservation`/`CommitReceipt`/`ReleaseReceipt`/`UnknownReceipt`/`InfraReceipt`。
    - `errors.rs`：`AdmissionErrorCode` 九个稳定 machine code（budget_exceeded/unpriced_model/missing_bound/
      scope_paused/invalid_scope_chain/idempotency_conflict/invalid_transition/admission_persistence_failure/
      resource_unobservable）+ `AdmissionError`（带可选 dimension）。
    - `pricing.rs`：`pricing_snapshot_from_config`（f64 per-million → 整数 micro，deterministic round-up，
      非有限/负价 fail closed）、`worst_case_money_micros`（u128 内部运算、reasoning 取 reasoning/output 价高者、
      multiplier round-up）、`build_llm_estimate`（money mode 要求 provider/model/currency 匹配 snapshot，
      output cap 需显式 bound 否则 missing_bound；token-only 不要求价格且 money=0）。
    - `resource.rs`：tool/process effect admission 边界。session 的 backend-created request scope（scope_id==session_id）
      存在时才 gate；disk/network 无可靠 owner instrumentation，envelope 硬 cap 这两维时返回
      `resource_unobservable` 拒绝（A-4 fail closed）；estimate=tool_calls 1 + processes（按工具是否 spawn 进程）+
      concurrency 1；`commit_tool_effect`（实际 tool_calls/wall time）/`mark_tool_effect_unknown`/`release_tool_effect`。
    - `recovery.rs`：`InfraFailureKind` 分类（transport/timeout/rate_limit/stream_failure 为 infra；
      auth_failure/model_not_found 为 correctness 不计入 infra threshold）、`recover_expired_reservations`
      （lease 过期的 reserved 保守冻结为 unknown，幂等，日志仅 opaque ID）。
  - 新增 `src-tauri/src/db/budget.rs`（MainStore 原子 ledger，全部走 DbRuntime dedicated writer 单事务）：
    `create_budget_scope`（冻结 envelope + cap 列物化 + parent chain/kind/status 校验）、`reserve_effect`
    （campaign→candidate→trial→request 顺序校验 active/parent/cap，idempotency_key 幂等重放/冲突拒绝，
    reservation 行 + 四级 reserved 余额 + append-only entries 同事务）、`commit_reservation`（actual 向量结算，
    仅对 envelope 硬 cap 维判定 overrun，overrun 同事务 pause campaign）、`release_reservation`（仅 Reserved 可 release，
    unknown 拒绝）、`mark_reservation_unknown`（预算冻结不释放）、`record_infra_failure`（operation_id 幂等计数，
    达 threshold 同事务 pause）、`reconcile_expired_reservations`、读投影 `get_budget_scope_status`/
    `get_budget_scope_envelope`/`get_budget_reservation`/`recompute_scope_balance_from_ledger`（audit 重算）。
  - 新增 `src-tauri/src/db/sql/migrations/v18.rs`：`experiment_budget_scopes`（33 个 cap/committed/reserved 整数列 +
    kind/parent/status CHECK + 非负 CHECK）、`experiment_budget_reservations`（idempotency_key UNIQUE、
    operation_id UNIQUE、state CHECK、estimate/actual 列）、`experiment_budget_ledger_entries`（append-only，
    operation CHECK）+ 4 个索引；注册于 `migrations/mod.rs`、`manager.rs`（v17→v18）。
  - 新增 `src-tauri/src/ccproxy/admission.rs`：`ADMISSION_HEADER`（x-cs-experiment-admission）、
    `AdmissionContext`（opaque scope chain/effect/idempotency/attempt）、`admission_context_from_headers`
    （仅 trusted internal request，双重信任校验）、`admit_before_send`（load envelope → worst-case estimate →
    reserve，spawn_blocking 包裹阻塞写）、`AdmissionLease`（commit / mark_unknown_with_infra_failure）、
    `AdmissionSettlement`（streaming 终端边界 blocking commit/unknown）、`rejection_message`（协议安全，仅 machine code）。
  - 修改 `src-tauri/src/ccproxy/handler/{chat_handler,direct_handler,responses_handler,embedding_handler}.rs`：
    五个 outbound 发送边界（含 /v1/responses 的 direct 与 unified fallback 两条）在 `send_with_retry` 前统一调用
    admission gate；opted-in 请求强制 `retry_config.max_retries=0`（默认单次 attempt）；发送前拒绝 → 协议安全错误、
    不发送；transport 错误 → mark unknown + infra failure；provider 错误响应 → commit 零用量；非流式成功 →
    按实际 usage commit；流式 → lease 进入 `StreamStatGuard` 终端边界。普通请求（无 context）路径零改动。
  - 修改 `src-tauri/src/ccproxy/helper/stat_guard.rs` + `stream_handler.rs`：`StreamStatGuard` 新增
    `admission: Option<AdmissionSettlement>`，Drop 终端边界 commit 实际 usage / stream_failed 时 mark unknown + infra。
  - 修改 `src-tauri/src/ccproxy/router.rs`：`strip_untrusted_workflow_attribution_headers` 增加
    `x-cs-experiment-admission`（外部伪造 header 剥离）。
  - 修改 `src-tauri/src/workflow/react/engine.rs` `execute_tools`：audit/approval/postpone 判定之后、物理执行之前
    调用 `admit_tool_effect`（仅 backend-created scope 的 session 被 gate）；拒绝 → 结构化 tool error observation
    （不执行）；执行完成按实际 tool_calls/wall time commit；取消 → mark unknown；postponed 未派发 → release。
    approval 等待不消耗预算。
  - 修改 `src-tauri/src/lib.rs`：`pub mod budget`（crate 级 contract）+ 启动时后台 best-effort
    `recover_expired_reservations`（不新增公开命令）。
- 提交/工作区状态：全部改动保留在工作区未 stage/commit（遵守项目规则）；基线仅有一个未跟踪的
  `work/.cs2a-smoke-artifact/`（2A 遗留，未触碰）。
- 验证命令与结果（`cd src-tauri`，全部通过，无 warning）：
  - V-1 `cargo test --lib budget::` → 51 passed（domain/machine code/overflow/envelope/状态机）。
  - V-2 `cargo test --lib migration` → 15 passed（fresh install 直接 v18、v1→v18 增量、apply 失败回滚、
    v18 三表存在断言）。
  - V-3 `cargo test --lib budget_pricing` → 13 passed（round-up deterministic、cache/reasoning/multiplier、
    token-only 免费模型、缺价/负价/NaN/模型不匹配/货币不匹配/无 output bound 全部 fail closed）。
  - V-4 `cargo test --lib budget_ledger` → 6 passed（reserve→commit 全四级余额 + audit 重算一致、幂等重放/
    冲突拒绝、任一级 cap 不足全量回滚、unknown 不可 release、overrun pause）。
  - V-5 `cargo test --lib budget_concurrency` → 2 passed（8 线程并发 reserve 恰好 5 个通过、无部分余额）。
  - V-6 `cargo test --lib budget_recovery` → 4 passed（infra 分类、过期冻结 unknown、幂等、live 不受影响）。
  - V-7 `cargo test --lib admission` → 11 passed（外部伪造 header 不可 mint admission、malformed 忽略、
    trusted reserve→commit、缺 scope fail closed、router 剥离断言、mock upstream 集成：admitted→provider 被调用
    且 usage commit、rejected→provider 零调用、forged→普通路径不变）。
  - V-8 `cargo fmt --all -- --check` 通过；`cargo check --bin chatspeed --bin cs` 零 warning；
    `cargo test --lib workflow::react::client` → 36 passed；`cargo test --bin cs` → 51 passed（2A 离线命令无回归）；
    `cargo test --lib ccproxy::handler` → 25 passed（既有 handler/stats/header 回归）。
- 真实 smoke：未执行。2B 按计划不注册 experiment run（INV-3），当前不存在能产生 backend-owned
  admission context 的调用方（2C 交付后才有）；真实 opt-in effect smoke 留待 2C 以 typed fixture/真实 run 验证。
- 未验证项 / 环境限制 / 剩余风险：
  - `workflow::react` 全量测试中有 9 个失败（security/path-guard csignore、prompts 文案、context 语言检测），
    经 pristine HEAD baseline worktree 复现确认均为**既有环境相关失败**（沙箱 locale/git 环境差异），
    与 2B 改动无关（2B 未触碰 security.rs/prompts.rs/context.rs）。
  - direct/responses/embedding 三个边界的负向集成测试未单独编写（与 unified 共用同一 `admit_before_send`
    helper 与相同 wiring 模式，unified 边界已有三条 mock-upstream 集成测试覆盖 gate 行为）。
  - disk/network 维度无 owner instrumentation：envelope 硬 cap 这两维时 tool effect 一律拒绝（A-4 预期行为），
    OS 级 enforcement 不在 2B 声称完成。
  - 并发 tool 的 concurrency 维按 reservation 占用/释放执行（engine semaphore 之外的第二层硬 cap），
    未做 OS 进程级限制（属 2G sandbox 范围）。
  - 基线验证遗留物：`.cs-2b-target/`（baseline worktree 的 cargo target 目录，untracked，可删除）。
  - 全库 clippy 既有 baseline 未扩大处理（新模块以 `cargo check` 零 warning 为准）。

#### 2B Final Review Round 2 Fix (as built, 2026-09-15)

第二轮 final review 提出 3 个 major 发现，已全部修复并补充回归测试：

1. **tool settlement 不再吞错**（AC-2/AC-7）：`budget/resource.rs` 的 `mark_tool_effect_unknown`（改为原子
   unknown+infra）、`release_tool_effect`、`release_proven_not_dispatched` 全部返回 `Result<(), AdmissionError>`
   并向调用方传播 machine-readable 错误；`engine.rs` 所有调用点显式处理：commit 失败 → error 日志 +
   保守 mark unknown+infra（若也失败则记录 reservation 保持 reserved 供 lease-expiry recovery）；
   unknown/release 失败 → error 级 machine code 日志并注明 reservation 保持 reserved 可被 recovery 收敛。
2. **流式无输出不再零结算**（AC-6/INV-5）：`AdmissionSettlement` 恢复携带 reserve estimate，新增
   `commit_conservative_input_blocking`：流终止但无输出时按保留 input/cache 上界 + snapshot 货币保守结算
   （output=0 有结构证据：无生成内容），envelope 不可用或 commit 失败时 fail closed 冻结 unknown；
   `stat_guard.rs` 的 `!has_output` 分支改用该结算；新增回归测试
   `stream_guard_settles_conservative_input_when_no_output`（money 模式下断言 committed input=100、
   money=100 micros、output=0、reserved 归零）。
3. **tool unknown 计入 infra threshold**（AC-6）：`mark_tool_effect_unknown` 改用
   `MainStore::mark_reservation_unknown_with_infra_failure` 原子操作（unknown + 幂等 infra 计数 + 阈值暂停
   同一 writer 事务）；engine 所有取消/失败路径传入 campaign id 与 failure kind（cancelled/tool_failure/
   settlement_failure）；新增测试 `tool_unknown_counts_infra_and_pauses_at_threshold`（threshold=2 时两次
   tool unknown 后 campaign paused 且后续 admission 返回 scope_paused）。

修复后验证（`cd src-tauri`，全部通过、双 binary 零 warning）：`budget::` 67 passed、`migration` 15 passed、
`budget_pricing` 13 passed、`budget_ledger` 6 passed、`budget_concurrency` 2 passed、`budget_recovery` 4 passed、
`budget_resource` 9 passed、`admission` 13 passed、`stat_guard` 4 passed、`ccproxy::handler` 28 passed、
`workflow::react::client` 36 passed、`cargo test --bin cs` 51 passed、`cargo fmt --all -- --check` 通过。

#### 2B Final Review Round 3 Fix (as built, 2026-09-15)

第三轮 final review 指出：tool 执行返回 `Err(ToolError)`（spawn/execution/owner failure）时仍被无条件
commit，未进入 unknown+infra 原子路径。已修复：

- `engine.rs` 并行与顺序两条 tool terminal settlement 均改为按实际工具结果分类：
  `Ok(_)`（可靠完成、有 owner 实际计量）→ `commit_tool_effect`（失败时保守 unknown+infra）；
  `Err(_)`（execution/spawn/owner failure，effect 可能已发生）→ 单事务
  `mark_tool_effect_unknown`（unknown + 幂等 infra 计数 + 阈值暂停），错误以 error 级 machine code
  日志传播，失败时 reservation 保持 reserved 供 lease-expiry recovery。
- 新增 engine 级回归测试 `tool_execution_failure_settles_reservation_as_unknown_with_infra`
  （recovery_tests 模块）：注册一个始终失败的测试工具（`ToolDefinition` + `ToolManager::register_tool`）、
  创建以 session id 为 request scope 的四级预算链、经真实 `execute_tools` 派发失败工具，
  断言 reservation 进入 `unknown`、ledger 中 infra_failure entry 恰好 1 条、campaign
  `infra_failure_count == 1` 且未暂停、request scope 的 tool_calls hold 保持冻结（unknown 不释放）。

修复后验证（`cd src-tauri`，全部通过、双 binary 零 warning）：`budget::` 67 passed、`migration` 15 passed、
`admission` 13 passed、`stat_guard` 4 passed、`workflow::react::engine::recovery_tests` 61 passed（含新增
engine 级失败路径测试）、`ccproxy::handler` 28 passed、`workflow::react::client` 36 passed、
`cargo test --bin cs` 51 passed、`cargo fmt --all -- --check` 通过。

#### 2B Final Review Round 4 Fix (as built, 2026-09-15)

第四轮 final review 指出：tool 终态结算仅按 `Ok/Err` 二分，确定未派发的失败（MCP policy/security 拒绝、
tool lookup miss）也会被错误冻结为 unknown 并计入 infra failure。已修复：

- `engine.rs` 并行与顺序 tool future 显式返回 `dispatched` 标志（physical owner 是否被真正调用：
  MCP policy 拒绝在调用 owner 前构造 `ToolError::Security`，`dispatched=false`）；terminal settlement
  改为三分类：`Ok` → commit（失败时保守 unknown+infra）；`Err` 且已 dispatch（execution/spawn/owner
  failure，含 `FunctionNotFound` 之外的 owner 内部错误）→ 单事务 `mark_tool_effect_unknown`
  （unknown + 幂等 infra 计数 + 阈值暂停）；`Err` 且 `FunctionNotFound`（tool lookup miss，owner 未调用）
  或 `dispatched=false`（policy 拒绝）→ `release_tool_effect`（release，不计 infra）。
- 新增 engine 级回归测试：
  - `tool_not_dispatched_error_releases_reservation_without_infra`：调用未注册工具（lookup miss），
    断言 reservation 进入 `released`、infra_failure entry 为 0、request scope reserved 归零；
  - `tool_infra_failures_pause_campaign_at_threshold_through_engine`：threshold=1 时一次执行失败即
    campaign paused，后续 admission 返回 `scope_paused`；
  - 既有 `tool_execution_failure_settles_reservation_as_unknown_with_infra` 继续覆盖
    execution failure → unknown + infra 计数路径。

修复后验证（`cd src-tauri`，全部通过、双 binary 零 warning）：`budget::` 67 passed、`migration` 15 passed、
`admission` 13 passed、`stat_guard` 4 passed、`workflow::react::engine::recovery_tests` 63 passed（含
not-dispatched release、threshold pause、execution failure 三条 engine 级路径）、`ccproxy::handler` 28 passed、
`workflow::react::client` 36 passed、`cargo test --bin cs` 51 passed、`cargo fmt --all -- --check` 通过。

#### 2B Final Review Round 5 Fix (as built, 2026-09-15)

第五轮 final review 指出：`dispatched` 标志在 `ToolManager::tool_call` 入口即置 true，而非 physical
owner（`ToolDefinition::call`）边界；disabled-MCP/security 预校验拒绝与 semaphore 排队中的取消仍会被
误分类为 unknown+infra。已修复：

- `tool_manager.rs` 新增 `tool_call_with_dispatch(name, params, owner_entered)`：在 registry lookup、
  disabled-MCP 预校验之后、`tool.call` 进入之前才设置 owner-entry 标志并返回权威 `dispatched` 事实；
  `native_tool_call` 委托该方法，行为不变。
- `engine.rs` 并行与顺序路径改用 tracked call；每个 tool 持有 `Arc<AtomicBool>` owner-entry 标志并
  存入 `tool_dispatch_flags`；terminal settlement 的 `proven_not_dispatched` 完全由 owner 边界事实决定
  （移除 `FunctionNotFound` 启发式）。
- 取消路径按 owner-entry 标志分类：并行 cancelled arm 与顺序 cancelled arm 中，flag=true（owner 已进入，
  可能产生 effect）→ 单事务 mark unknown + infra；flag=false（仍在 semaphore 排队/未进入 owner）→
  release，不计 infra。
- 新增回归测试：
  - manager 级 `tool_call_with_dispatch_reports_owner_entry_fact`：lookup miss → `dispatched=false`；
    owner 进入（即使工具失败）→ `dispatched=true` 且标志置位；
  - engine 级 `tool_mcp_policy_rejection_releases_reservation_without_infra`：MCP 工具不在 allowlist
    （空 `available_tools`）→ 预 owner Security 拒绝 → released、infra 0；
  - engine 级 `tool_cancelled_while_owner_running_settles_unknown_with_infra`：owner 进入后取消
    （BlockingTestTool 信号 + pending）→ unknown + infra 1；
  - engine 级 `tool_cancelled_before_owner_entry_releases_reservation`：占满 3 个 semaphore permit 使
    tool future 排队后取消 → released、infra 0。

修复后验证（`cd src-tauri`，全部通过、双 binary 零 warning）：`budget::` 67 passed、`migration` 15 passed、
`admission` 13 passed、`stat_guard` 4 passed、`workflow::react::engine::recovery_tests` 66 passed（新增
MCP policy 拒绝 release、owner 运行中取消 unknown+infra、排队中取消 release 三条 engine 级路径）、
`tool_manager` dispatch-fact 测试 1 passed、`ccproxy::handler` 28 passed、`workflow::react::client` 36 passed、
`cargo test --bin cs` 51 passed、`cargo fmt --all -- --check` 通过。

#### 2B 真实桌面 smoke 结果（2026-09-15 已执行）

用户指出其配置下 `pnpm tauri dev` 可本地执行；实测确认可用（此前"沙箱无 pnpm/DISPLAY"的判断不成立：
Bash 工具的 host 路由使该命令在宿主机桌面会话中正常运行）。已执行并确认：

1. `pnpm tauri dev` 启动成功：Vite dev server 就绪（localhost:1420），cargo run 编译并启动桌面应用，
   前端连接正常（workflow snapshot 请求、MCP server、51 个 skills 均正常加载）。
2. 迁移验证：日志输出 `Database is already up to date at version 18.`；dev 数据库
   （`dev_data/chatspeed.db`）`db_version` 表含 1..18 全部版本记录，v18 于 2026-09-14 应用；
   `experiment_budget_scopes` / `experiment_budget_reservations` / `experiment_budget_ledger_entries`
   三表均存在。
3. 启动恢复验证：整个运行期日志中 `[Budget]` 行数为 0——recovery 完全静默，无遗留 reservation 处理。
4. 普通 workflow 无回归（INV-2）：应用正常处理既有 workflow 会话；三张预算表保持 0 行
   （普通请求不进预算路径）。
5. 运行期错误审计：日志中 10 条 error 均为既有环境问题（全局热键已被既有实例注册 ×9、
   updater 网络检查失败 ×1），与 2B 无关。
6. 负向信任边界（伪造 admission header 实测）：执行代理的沙箱网络与宿主机隔离，无法直接探测
   ccproxy 端口；该边界由自动化测试覆盖（router 剥离伪造 header + admission fake-owner 集成测试）。
   桌面环境如需人工复核，可对 ccproxy 端口发送携带伪造 `x-cs-experiment-admission` header 的请求，
   应按普通路径处理且三张预算表无新增行。

预算 gate 本身的端到端触发仍留待 2C 的 backend-owned experiment context（INV-3：2B 不注册 experiment run）。

#### 2B 真实桌面 smoke 步骤（原始计划，供复现）

本沙箱无 DISPLAY/X server 且无 pnpm，无法启动 `pnpm tauri dev`；且按 INV-3，2B 不注册 experiment run，
普通 workflow 不携带 admission context，桌面 smoke 无法触发预算 gate 本身（gate 触发需 2C 的
backend-owned experiment context）。桌面环境可执行以下 smoke 验证 2B 的启动/迁移/恢复/无回归面：

1. `pnpm tauri dev` 启动应用（账号/模型用法参考 `work/agent-cli-phase-1-smoke-test.md`：
   loopback 控制面 + 免费模型 `cs@free:ds-v4-flash`）。
2. 迁移验证：应用数据目录 `chatspeed.db` 的 `db_version` 应为 18，且存在
   `experiment_budget_scopes` / `experiment_budget_reservations` / `experiment_budget_ledger_entries` 三表。
3. 启动恢复验证：应用日志无 `[Budget][recovery]` error（无遗留 reservation 时应完全静默）。
4. 普通 workflow 无回归（INV-2）：按阶段一方式 `cs workflow run --agent builtin:coding
   --model "cs@free:ds-v4-flash" --prompt ... --follow` 应正常完成；随后检查
   `experiment_budget_scopes` / `experiment_budget_reservations` 仍为空（普通请求不进预算路径）、
   `ccproxy_stats` 照常记录。
5. 负向信任边界（可选）：对 ccproxy 端口以外部请求携带伪造 `x-cs-experiment-admission` header，
   应被 router 剥离、请求按普通路径处理且不产生任何预算数据。
- 下一阶段入口与前置：**2C（Experiment run，受控单次试验）**。2C 需通过 backend-owned 路径创建四级 scope chain
  （`MainStore::create_budget_scope`，request scope id = workflow session id 的 2B 约定）并在 trusted internal
  request 上携带 `x-cs-experiment-admission` context（或等价 backend context）触发 LLM gate；tool gate 按
  session scope 自动生效。2C 不得绕过 admission contract，不得假设 USD（货币由 experiment profile 声明）。

#### 2B Final Review Fix Round (as built, 2026-09-15)

Final review 提出 4 个 major 发现，已全部修复并补充回归测试：

1. **结算不再清零关键维度**（AC-3/AC-4/AC-6）：
   - `budget/pricing.rs` 新增 `settlement_money_micros`：按冻结 pricing snapshot 从实际 token 用量计算
     结算货币（与 reserve 相同的确定性 round-up）；money-budget 模式下结算绝不提交零货币。
   - `ccproxy/admission.rs`：`AdmissionLease` 携带 `request_scope_id` 与 reserve estimate；新增
     `commit_usage`（实际 token + snapshot 货币）与 `commit_provider_error_response`（provider 已处理但
     无生成内容的错误响应：input 按保留的 estimate 上界、output 为 0 的有证据保守结算，绝不 commit 零向量）；
     所有 handler 成功路径改用 `commit_usage`，错误响应路径改用 `commit_provider_error_response`；
     `AdmissionSettlement::commit_usage_blocking` 在终端边界同样按 snapshot 计算货币，commit 失败时
     fail closed 冻结为 unknown 并记录 error 级 machine code。
   - `budget/resource.rs` `commit_tool_effect` 提交真实 `processes`（按工具是否 spawn 进程），
     仅 concurrency 在终态释放；新增顺序请求跨 money cap 的 overrun 测试与 process cap 累计测试。
2. **required dimension 按 effect owner 校验**（AC-3/A-4）：`budget/resource.rs` 新增
   `check_owner_observability`：LLM/embedding owner 可观测集为 token/wall time/concurrency/money，
   tool/process owner 为 tool_calls/processes/wall time/concurrency；envelope `required_dimensions`
   含 owner 不可观测维度时在 physical effect 前返回 `resource_unobservable`（LLM 在 `admit_before_send`、
   tool 在 `admit_tool_effect` 各自校验），并附 LLM（required processes）与 tool（required network bytes）
   拒绝测试。
3. **pre-dispatch 清理所有权**（AC-6/AC-7）：engine `execute_tools` 对 Stage 1 已 reserve、尚未 dispatch 的
   reservation 建立显式清理：partition `Err` 早退与 dispatch 前 stop check 走
   `release_proven_not_dispatched`（证明无 effect → release 而非冻结）；parallel/sequential 非取消失败对
   已开始执行的工具 mark unknown、未派发的 release；turn 末尾 stop check 对任何未结算残留保守 mark unknown；
   新增 `release_proven_not_dispatched` 批量 release 测试。
4. **unknown + infra failure 原子化**（AC-2/AC-6）：`MainStore::mark_reservation_unknown_with_infra_failure`
   在同一 writer transaction 完成 mark_unknown、幂等 infra 计数与 threshold pause；
   `AdmissionLease::mark_unknown_with_infra_failure` 与 `AdmissionSettlement::mark_unknown_blocking`
   均改用该原子操作；异步路径传播持久化失败（error 级 machine code 日志），stream/Drop 边界记录
   error 日志且 reservation 保持 reserved 供 lease-expiry recovery 收敛；新增原子性/幂等/阈值暂停测试。

修复后验证（`cd src-tauri`，全部通过、双 binary 零 warning）：`budget::` 66 passed、`migration` 15 passed、
`budget_pricing` 13 passed、`budget_ledger` 6 passed、`budget_concurrency` 2 passed、`budget_recovery` 4 passed、
`budget_resource` 8 passed、`admission` 13 passed、`ccproxy::handler` 28 passed、`workflow::react::client`
36 passed、`cargo test --bin cs` 51 passed、`cargo fmt --all -- --check` 通过。

