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

- **2A/2B/2C/2D+2E/2F 状态**：均已完成（代码、focused 验证与记录见 `## 9` 对应 Implementation Record；
  2A/2C/2F 含真实 desktop smoke，2B 预算 gate 的端到端触发由 2C/2F smoke 覆盖；2D+2E 含离线 CLI 进程
  smoke 与真实 desktop 固定免费模型全链 smoke；2F 含三个独立 campaign 的真实免费模型
  baseline+candidate 交付、负向与退出审计，见该记录与 `work/agent-cli-phase-2-smoke-test.md` `## 6`）。
- **下一个入口**：无（Phase 2 的 2A–2I 已全部完成）。`2I` 已于 2026-09-17 完成并推进指针：
  promotion/apply/受控灰度/审计闭环（代码 patch 候选、自动 policy、实验分支本地 commit 不 push、
  paired canary、audit bundle）的 AC-1..AC-9 与 INV-1..INV-10 均有证据（见 `## 12` 的 2I
  Implementation Record 与 `work/agent-cli-phase-2-smoke-test.md` `## 8`）。2G+2H 的 Implementation
  Record 按规则**不改写**。
- **后续交付口径**：2C 之后的实现按 `2C → 2D+2E → 2F → 2G+2H → 2I` 的顺序成组交付。该口径只用于
  规划后续阶段的打包与推进顺序，属于准备工作，不作为任何阶段的功能验收证据；各阶段仍须各自满足其
  Acceptance Criteria、Protected Invariants、Execution Units 与 Verification 后才可推进指针。
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
- **2D**（本期与 2E 成组交付；首版仅 deterministic evaluator）
  - 进入：2A、2C 完成。
  - 退出（首版）：离线 deterministic evaluator 对已通过 2A verifier 的 artifact 产出独立、版本化
    evaluation sidecar（绑定 `run_id/session_id/chain_head`，含确定性检查事实与
    `pass|fail|not_evaluable` 稳定状态）；LLM judge（版本/prompt hash/预算绑定）延后到后续阶段；
    无 promotion verdict。
- **2E**（本期首版：checked-in `chatspeed-smoke@2` contract smoke）
  - 进入：2D 首版完成；本期 fixture 身份固定为 `dataset_id=chatspeed-smoke`、`dataset_version=2`、
    `split=smoke`，task manifest/verifier identity/resource profile 在实现时生成并写回本文件与
    golden tests（见 `## 10`）；不宣称 Aider/SWE-bench/Terminal-Bench 官方成绩。
  - 退出（首版）：至少一个 benchmark adapter + independent verifier 端到端产出绑定具体 artifact 的
    分数事实；真实 Harbor installed-agent 接入延后到 2G+2H，本期只固定 adapter/verifier 合约。
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

### 1.4 本次执行范围（历史口径：2A 期；当前 active scope 见 `## 10`）

2A 期口径：本次**只实现 2A**。2B–2I 仅在本路线文档登记，不进入本次代码、不注册、不声称支持。
2A 的完整 contract、执行单元与验证见本文件 `## 2` 之后的 2A 详细计划与 `## Implementation Record`。
（当前 active scope 已推进到 2D+2E，见 `## 10`；本节保留为 2A 历史口径。）

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

### 2C Implementation Record (as built)

- 日期 / 当前阶段推进：2026-09-15；2C 代码与确定性验证完成；真实 desktop 固定模型 smoke 受既有桌面实例
  占用 dev 端口（1420）阻塞（见"剩余风险"）；下一入口 **2D+2E**。

#### 交付范围（In scope 1–13）

- **U-1**：新增 `src-tauri/src/workflow/react/experiment.rs`——strict `ExperimentRunSpecV1`
  （`deny_unknown_fields`，固定 `schema_version="experiment_run_spec.v1"`）、`ExperimentRunRequest`/
  `ExperimentRunResult`/`ExperimentScopeRefs`、`ExperimentSpecError` 稳定 machine codes
  （`unsupported_spec_version`/`empty_prompt`/`invalid_config`/`invalid_budget`/`max_attempts_not_one`/
  `resource_unobservable`/`currency_mismatch`/`child_agent`）。`to_envelope()` 在 effect 前强制
  `max_attempts==1`、拒绝 disk/network hard-cap/required、复用 2B `BudgetEnvelope::validate`。
  `db/budget.rs` 新增 `NewExperimentWorkflowRow`、`canonical_experiment_chain`（request=session id，
  其余 `:trial`/`:candidate`/`:campaign` 后缀，与 2B tool/LLM owner 完全一致）、`create_experiment_run_atomic`
  （单 writer transaction 插入 workflow 行 + 四级 scope，任一失败整体回滚，无新 migration）。
  `commands/workflow.rs` 抽出共享 `build_resolved_workflow_config`（普通 create 与 experiment 共用 config
  resolver），新增 `run_experiment_core`：backend TSID 生成 session/run id → 原子创建 → 安装 session key →
  复用 `workflow_start_core`；experiment 以非空确定性 title 创建，使复用的 start kernel 的 title helper
  不触发额外 LLM effect；普通 create/title 行为不变。`application.rs` 暴露唯一 facade `experiment_run`。
- **U-2**：`db/budget.rs` 新增只读 `get_budget_scope_chain(request_scope_id)`（无 request scope→None；
  半链/错链/非 canonical→`StoreError::InvalidData` fail closed）。`ai/chat/openai.rs` 在统一内部 header
  构造点、custom headers **之后**由 backend 覆盖写入 `x-cs-experiment-admission` AdmissionContext
  （fresh effect/idempotency id、attempt=1）；chain 读取/序列化失败在 localhost send 前返回
  `AiError::InitFailed` fail closed，绝不降级普通请求。budgeted session 关闭应用层重试：
  `llm.rs` ReAct `max_retries=0`（新增 `budgeted` 字段，构造期读 durable chain）、
  `intelligence.rs` language helper 最多 1 次、`compression.rs` 压缩最多 1 次；smart approval 单次调用且
  已带 attribution。ccproxy 层 retry 已为 0（既有 `x-cs-retry-max-count:0`）。tool/process 复用 2B
  `admit_tool_effect`/settlement，未新增第二 gate。`ccproxy/mod.rs` 仅把 `admission` 模块放宽为
  `pub(crate)`（供 openai 注入使用）。**Final review 修复**：`ccproxy/admission.rs` 把 header 解析契约改为
  `extract_admission_context`——受信内部请求携带**存在但 malformed** 的 `x-cs-experiment-admission` 现返回稳定
  `invalid_scope_chain` 错误（而非旧的静默 `None`），`admit_before_send` 经 `?` 传播，chat/direct/responses/
  embedding handler 既有 `Err` 分支在 `send_with_retry` 前返回，确保 malformed context 在 provider 零调用下
  fail closed，不降级普通路径（AC-3/INV-2/INV-3）；header 缺失仍为普通路径（INV-4）；更新原
  `malformed_context_is_ignored` 为 `malformed_context_fails_closed_before_send` 负向测试。
- **U-3**：`http/dto.rs` 新增 strict `ExperimentRunHttpRequest`（`deny_unknown_fields`）。
  `http/server.rs` 注册唯一 canonical `POST /control/v1/experiments:run`，强制 bearer（既有 layer）、
  强制非空 `Idempotency-Key`（缺失→400 `missing_idempotency_key`）、复用 `with_idempotency`（同 key/body
  单次、异 body 409）；成功返回 201 `{schema_version,run_id,session_id,scopes,status:"started"}`。
  CLI 新增 `cs experiment run --agent --spec (--prompt|--prompt-file) [--follow] [--artifact-dir]`
  （`args.rs`/`bin/cs.rs`/`experiment.rs`）：读 spec 文件、生成一次 idempotency key、POST；无 artifact-dir
  启动后返回；有 artifact-dir 隐含轮询 durable 终态后复用提取的 render-free `capture_bundle`（避免双 stdout）。
  `error.rs` 新增 `CliError::Budget`（exit 9）；`client.rs` `is_budget_code` 将预算 machine code 映射为 exit 9，
  stdout 仅协议、stderr 仅诊断。offline inspect/replay 分流与 artifact v1 语义未改。
- **U-4**：见下方验证。

#### 验证证据（`cd src-tauri`，双 binary 零 warning，fmt clean）

- `cargo fmt --all -- --check`：通过。`cargo check --bin chatspeed --bin cs`：0 warning。
- `cargo test --lib workflow::react::experiment`：10 passed（strict serde round-trip + 负向矩阵：unknown
  field/version、max_attempts!=1、disk/network hard-cap、required 无 cap、currency mismatch、
  token-only 带 money cap）。
- `cargo test --lib budget::`：66 passed（新增 3 个 `create_experiment_run_atomic` 原子性/回滚/无效 envelope
  零写入 + 3 个 `get_budget_scope_chain` None/正解/非 canonical fail-closed）。
- `cargo test --lib chat::openai`：55 passed（新增 3 个 AdmissionContext 构造：普通 session→None、
  budgeted→attempt=1 canonical、非 canonical→fail-closed）。
- `cargo test --lib workflow::react::client`：41 passed（新增 5 个 endpoint：缺 key→400、无 bearer→401、
  unknown spec field→400 且无 workflow、wrong version→400 且无 workflow、valid run→201 且恰一 workflow +
  四级 scope + 同 key/body replay 不双建 + 异 body 409）。
- `cargo test --lib admission`：16；`stat_guard`：4；`ccproxy::handler`：28（2B 回归，含伪造 external
  admission header 走普通路径）。`cargo test --lib workflow::react::{llm,intelligence,compression}`：
  41/6/59 passed（retry 门控无回归）。`cargo test --bin cs`：56 passed（2A capture 重构无回归 + 新增
  error/client budget exit-9 测试）。根目录 `pnpm test:workflow`：62 passed。

#### 剩余风险 / 限制（如实记录）

- **真实 desktop 固定模型 smoke（V-7）已执行**（用户关闭既有实例后 `pnpm tauri dev` 以 2C 重建二进制启动）：
  completed run（`0rmancd3g0400`，artifact complete、离线 inspect/replay 通过、exit 0）、helper+主 ReAct 两个
  attributed LLM effect 各自 reserve、极小 cap（`0rmap8ny00400`）在 provider 前 `budget_exceeded` 零调用且
  language helper `attempt 1/1`、普通 workflow（`0rmapkhdw0400`）0 条 admission 且 429 指数退避重试
  （1s/2s/4s，attempt n/10）。详见 smoke 文档 §5.2。
  - **smoke 发现并修复的 CLI bug**：`bin/cs/experiment.rs` 的 `wait_for_terminal` 原按 `"failed"` 匹配终态，
    但持久化 `WorkflowState` 序列化为 `"error"`，导致 `--artifact-dir` 在失败 run 上轮询到超时；已改为
    `completed|error|cancelled`，失败 run 正确捕获 artifact 并按结局退出（completed→0 / 其它→1 / 预算拒绝→9）。
  - **已知限制**：LLM-path 执行期预算拒绝目前只在 crash 日志/瞬时 error chunk，未进入 CLI 可读 durable 事件
    （captured events 仅 workflow_started/effective_task_objective_changed/state_changed），故该异步场景 CLI
    退出 1 而非 9；同步 control-plane 预算 machine code 响应仍 exit 9（`is_budget_code`，已测），tool-path 拒绝
    写入 durable tool observation 可被 `budget_rejected_in_events` 命中→exit 9。要让 LLM-path 执行期拒绝稳定
    exit 9 需后端把 admission 拒绝码持久化进 durable 失败事件（runtime 变更，另行评估）。
- 未引入 evaluator/benchmark/promotion/headless/新 migration/第二 runtime owner；2A/2B Implementation
  Record 原样保留。路线口径整理（`2C → 2D+2E → 2F → 2G+2H → 2I`）仅为准备，不作功能验收证据。

### 2D+2E Implementation Record (as built)

- 日期 / 当前阶段推进：2026-09-15；2D+2E 代码与确定性验证完成；离线 CLI 进程 smoke 与**真实 desktop
  固定免费模型全链 smoke（benchmark run→capture→evaluate→verify）均已执行**；下一入口 **2F**。
- 实际文件与符号：
  - 新增 `src-tauri/src/bin/cs/evaluate.rs`（2D deterministic evaluator）：
    `EVALUATION_SCHEMA_VERSION=1`、`EVALUATION_KIND="cs.evaluation.deterministic"`、
    `EVALUATOR_ID/EVALUATOR_VERSION`、`code::TARGET_OVERLAP`、`CheckStatus{Pass,Fail,NotEvaluable}`、
    `CheckClass{Correctness,Infra}`、`run_checks`（10 项确定性检查：schema/manifest/chain/session 绑定、
    artifact 完整性、terminal 已知且 completed、cost known、usage totals、无 budget rejection）、
    `correctness_status`（correctness-class fail→fail；infra fail 或 not_evaluable→not_evaluable；
    否则 pass）、`build_evaluation`（`created_at` 不参与 `evaluation_hash`，重复评估 canonical-equivalent）、
    共享 sidecar writer `SidecarBundle`/`write_sidecar`/`verify_sidecar_dir`（staging + re-verify +
    atomic rename；单文件 manifest、symlink/extra/size/hash fail closed）、`reject_target_overlap`
    （输出目录与 artifact 目录互含即拒绝）、`evaluate()`（discovery 分流前离线执行）。
  - 新增 `src-tauri/src/bin/cs/benchmark.rs`（2E adapter；初始 `chatspeed-smoke@1`/`smoke`，本次
    verifier coverage 语义修订后当前 fixture 为 `chatspeed-smoke@2`）：
    `RUNNER_KIND="local_control_plane"`、fixture 经 `include_str!`（`CARGO_MANIFEST_DIR/../work/
    agent-cli-smoke-benchmark/`）编译期内嵌（无运行时路径可替换）、strict `BenchmarkManifestV1`/
    `SmokeTaskV1`/`SmokeResourceProfileV1`（`deny_unknown_fields`）、`resolve_task`（suite/task 白名单、
    task 文档 swap 拒绝、verifier 身份一致性、instruction hash 校验、`task_digest`/`manifest_digest`
    计算）、`build_run_spec`（profile→`experiment_run_spec.v1` caps，disk/network 永不 cap，
    `max_attempts=1`，token_resource_only）、`adapter_metadata`（transport-neutral，供未来 Harbor
    runner 复用）、`run()`（复用 `experiment::run_with_spec`→既有 `POST /control/v1/experiments:run`）。
  - 新增 `src-tauri/src/bin/cs/verifier.rs`（2E independent verifier）：`VERDICT_KIND=
    "cs.benchmark.verdict"`、`run_verdict_checks`（terminal/cost 与 expected 一致、无 budget rejection、
    input/output/cache/wall-time caps 对 artifact usage 实际值，None cap=not_applicable）、
    `score`（仅全部**独立可验证** pass/not_applicable→1.0；tool/process/concurrency 的
    not_applicable 不构成 pass 证据）、`verifier_digest`（绑定 verifier 身份+expected 契约+resource
    profile）、`build_verdict`（dataset/task/verifier digest + `run_id/session_id/chain_head` 绑定 +
    safety/infra/budget facts；明确区分 admission caps、independently verified caps 与 artifact v1
    未记录的 tool/process/concurrency caps；provenance `artifact/benchmark_adapter/independent_verifier`；
    无 promotion 字段）、`verify()`（离线，discovery 分流前执行）。
  - 修改 `src-tauri/src/bin/cs/args.rs`：`ExperimentCommand` 新增 `Evaluate{artifact_dir,
    evaluation_dir}` 与 `Benchmark{Run|Verify}`（`BenchmarkCommand`）。
  - 修改 `src-tauri/src/bin/cs.rs`：新增 `mod benchmark/evaluate/verifier`；`Evaluate` 与
    `Benchmark::Verify` 在 `load_discovery` 前离线分流；`Benchmark::Run` 走既有 client 分支。
  - 修改 `src-tauri/src/bin/cs/experiment.rs`：抽出 `run_with_spec`（`run` 读文件后委托；benchmark
    adapter 复用同一 POST/idempotency/capture 路径，无第二 runtime）。
  - 修改 `src-tauri/src/bin/cs/artifact.rs`：`ArtifactError::new` 改为 `pub`、新增
    `REQUIRED_FILE_COUNT`（供 sidecar 投影使用；v1 verifier 语义零改动）。
  - 新增 fixture：`work/agent-cli-smoke-benchmark/manifest.json`、`tasks/smoke_reply_ok.json`、
    `tasks/smoke_echo_ping.json`（instruction "Reply with exactly: OK"/"PONG"；expected：terminal
    completed + cost known + 无 budget rejection；resource profile：input 65536/output 128000/
    cache null(not_applicable)/wall 300000ms/tool_calls 0/processes 0/concurrency 1）。
  - `benchmark run` 新增 `--model`（可选 act-phase 模型 override，转发为 2C spec 的
    `workflow.model`；与 `--agent` 一样是运行时参数，不进入 fixture digest）。
- 提交/工作区状态：全部改动保留在工作区未 stage/commit（遵守项目规则）；无越界文件。
  遗留物：`src-tauri/target/cs-2e-smoke/`（离线 CLI smoke 的临时 artifact/sidecar，位于 gitignored
  target 目录，沙箱删除被策略拒绝，可手动删除）。
- 验证命令与结果（`cd src-tauri`，双 binary 零 warning，fmt clean）：
  - `cargo test --bin cs` → **110 passed; 0 failed**（2A 51+2C 5 基线无回归；新增 evaluate 23、
    benchmark 18、verifier 16：正向 pass/fail/not_evaluable 结构化区分、canonical-equivalent 重复
    评估/验证、artifact 不可变性 hash 前后一致、隐私负向断言、篡改/未来 schema/缺文件/重复发布/
    symlink（含父目录 symlink）/overlap/未知 suite/task/字段/重复 task/verifier 身份/instruction
    hash 篡改全部稳定 machine code 且不发布 sidecar、模型自述不影响分数、budget rejection→score 0+
    safety fail、golden digest 锁定、model override 转发）。
  - `cargo test --lib workflow::react::client` → **41 passed**（既有 endpoint/idempotency/budget
    契约不变；benchmark run 复用同一 POST 路径，未新增 route）。
  - `cargo check --bin chatspeed --bin cs` → 0 warning；`cargo fmt --all -- --check` 通过。
  - 根目录 `pnpm test:workflow` → **62 passed; 0 failed**。
- 离线 CLI 进程 smoke（V-8 替代执行，2026-09-15）：当前环境无运行桌面实例（无 discovery 文件），
  在线 `benchmark run` 无法执行；以临时测试写出真实 2A artifact 后用真实 `cs` 二进制完成离线链路：
  - `cs experiment evaluate <artifact> --evaluation-dir <eval>` → exit 0，
    `correctness_status=pass`，sidecar（evaluation.json + artifacts/manifest.json）发布；
  - `cs experiment benchmark verify --suite chatspeed-smoke --task smoke_reply_ok <artifact>
    --verdict-dir <verdict>` → exit 0，`score=1.0, safety=pass, infra=pass`，verdict 绑定
    dataset/task/verifier digest 与 `run_id/session_id/chain_head`；隐私扫描（secret prompt/
    Bearer/sk-/模型名）无命中；
  - 篡改 run.json → evaluate exit 1（`hash_mismatch`）且无 sidecar 发布；未知 task → verify exit 1
    （`unknown_task`）且无 verdict 发布；重复目标 → exit 1（`target_exists`）；
  - 全程无 discovery 文件存在 → 证明 evaluate/verify 离线（无主进程/网络/DB/key）。
- 未验证项 / 环境限制 / 剩余风险：
  - **真实 desktop 固定免费模型全链 smoke（V-8）已执行**（2026-09-15，`pnpm tauri dev` +
    `cs@free:ds-v4-flash`）：`cs experiment benchmark run --suite chatspeed-smoke --task
    smoke_reply_ok --agent builtin:coding --model cs@free:ds-v4-flash --artifact-dir …` → session
    `0rmczhh500400` terminal=completed、artifact=complete、exit 0；离线 evaluate →
    `correctness_status=pass`；离线 benchmark verify → `score=1.0, safety=pass, infra=pass`，
    verdict 绑定 run_id/chain_head/新 digest；篡改 run.json → exit 1 `hash_mismatch` 且无 sidecar
    发布。应用日志可见 `[Budget][admission] reserved … for effect llm:…`（预算 admission 真实生效）。
  - **fixture 资源 cap 校准过程（有价值的负向证据）**：前两次 desktop run 因 admission 按设计
    fail closed 被拒——run `0rmcvtxt80400`（output 投影 8192/128000 > cap 2000）与 run
    `0rmcye9w00400`（input 投影 30697 > cap 20000）均在 provider 调用前 `budget_exceeded`，零
    provider effect；据此把 fixture caps 校准为 input 65536/output 128000（worst-case reserve
    需覆盖 agent 配置的 max tokens 与真实 prompt 规模），digest 同步更新并回写本文档与 golden
    tests。这两次被拒 run 的 artifact 为 incomplete/cost unknown，verifier 如实给不可通过事实。
  - 本期 verifier 是 contract-level candidate-untrusted boundary，不是 2G/Harbor 的 OS/container
    隔离；输出与文档均已标注。
  - fixture 的 response-content oracle（校验模型最终回复内容 hash）依赖 durable 事件携带回复文本，
    当前 control-plane 事件不保证；本期 expected facts 限定为 control-plane-observable 结构化事实
    （terminal/cost/budget/caps），内容级 oracle 留待 2G/2H（对应计划 Q-3）。
  - cache_read/cache_write cap 在 fixture 中为 null（not_applicable）：免费模型缓存行为不可保证，
    hard cap 0 会在 admission 前被 `budget_exceeded` 拒绝；如需收紧须同步 bump dataset version。
  - **2F 前置决策（用户已提出）**：当前 2C `ExperimentWorkflowOverride` 仅支持
    model/allowed_paths/auto_approve_plan/final_audit；benchmark matrix 需要的完整 agent-config
    级 override（models 各 slot、工具集、MCP、skills 等）需扩展 2C 公共契约，留待 2F 入口单独
    确认与实现，本期不悄悄扩。
  - CLI flag 命名偏差（局部实现细节）：计划文本的 `--output <evaluation-dir>` 与既有全局
    `--output`（输出格式）冲突，evaluate 使用 `--evaluation-dir`、verify 使用 `--verdict-dir`；
    语义与计划一致。
  - 全库 clippy 既有 baseline 未扩大处理（新模块以 `cargo check` 零 warning 为准）。
  - verifier v2 coverage clarification（本次风险处置，2026-09-15）：2E fixture 仍将
    `tool_calls=0/processes=0/concurrency=1` 完整下发给 2C admission/runtime；但 2A artifact v1 只携带
    token totals 与 wall time，不能独立重建上述三项实际值。故 `chatspeed-smoke-verifier` 升为 v2、
    verdict document schema 升为 v2：metrics 显式列出三项为 `not_applicable`（`actual=null`，原因
    `not_recorded_in_artifact_v1`），`budget_facts` 分为 `admission_caps`、
    `independently_verified_caps` 与 `unverified_admission_caps`。`score=1.0` 仅说明已独立可验证
    facts 全部通过，**不再暗示**三项 admission-only cap 已由 artifact 独立回验。fixture/verifier
    digests 已按 v2 contract 更新并由 golden tests 固定；本次未修改 2A schema 或 artifact。
  - sidecar 输出路径 TOCTOU（本次风险处置结论）：当前 lexical/canonical overlap 与既存 symlink
    ancestor 拒绝已能防止调用前存在的符号链接重定向，但任意可写父目录中的并发替换仍无法由跨平台
    `std::fs` 路径 API 完全消除。彻底消除需另立跨平台、handle-relative writer/verifier（Unix
    `openat`/`renameat2` 与 Windows directory handle/reparse-point 对等实现），或改变 CLI 以只接受
    攻击者不可写的可信输出根；两者均超出本期最小范围与现有任意 `--evaluation-dir/--verdict-dir`
    契约。本期保留现有 fail-closed 防御并如实记录该边界，不作“已消除 TOCTOU”声明。
- 下一阶段入口与前置：**2F（Candidate / campaign / proposer）**。进入前需确认 candidate 生成是否
  引入新 LLM effect（须走 2B admission）、campaign 编排的预算 scope 复用方式，以及 verdict→candidate
  的消费契约（本期 verdict schema 已固定 provenance/binding，2F 不得改写）。

#### 2D+2E Final Review Fix (as built, 2026-09-15)

Final review 指出一个 major 发现：sidecar 输出路径的**父目录符号链接**可绕过纯 lexical
`Path::starts_with` overlap 检查（如 `<link>` → artifact 目录，输出设为 `<link>/evaluation`），
atomic rename 会把 sidecar 实际写入 2A artifact 目录。已修复：

- `evaluate.rs` 新增共享 `reject_symlink_ancestors`（对输出路径所有**已存在**祖先逐级拒绝 symlink，
  稳定 machine code `symlink_rejected`）与 `canonicalize_output`（canonicalize 最深已存在祖先并
  拼回不存在尾部）；`reject_target_overlap` 升级为 lexical + filesystem-resolved 双重校验（带
  `target_kind` 参数区分 evaluation/verdict 措辞）。
- `evaluate()` 与 `verifier::verify()` 均先做 symlink-ancestor 拒绝再做 overlap 校验；
  `write_sidecar` 内部追加 symlink-ancestor 防御（任何调用方都被保护）；verifier 侧 overlap
  失败保留 verdict 专属 machine code `verdict_target_unsafe`。
- 新增负向测试：`evaluate_rejects_symlinked_parent_targeting_artifact`、
  `verify_rejects_symlinked_parent_targeting_artifact`（断言稳定非零、无 sidecar/verdict 发布、
  artifact 文件 hash 前后不变）与 `evaluate_allows_sibling_output_under_real_parent`（真实父目录
  下的兄弟输出目录仍被允许，防过度拒绝回归）；`evaluate_rejects_symlinked_output_target` 更新为
  断言 `symlink_rejected`。
- 真实 CLI 进程复验：`cs experiment evaluate <artifact> --evaluation-dir <link>/evaluation` 与
  `cs experiment benchmark verify ... --verdict-dir <link>/verdict`（`<link>` → artifact 目录的
  symlink）均 exit 1（`symlink_rejected`），artifact 目录内无任何新增文件。
- 修复后验证：`cargo test --bin cs` → **109 passed; 0 failed**；`cargo test --lib
  workflow::react::client` → 41 passed；`cargo check --bin chatspeed --bin cs` → 0 warning；
  `cargo fmt --all -- --check` 通过。

### 2F Implementation Record (as built)

**范围**：Stage 0 人工单变量 candidate + 确定性规范化/校验 + 不可变 CLI campaign plan +
backend-owned shared campaign budget scope。无 LLM proposer、无 promotion、无 migration、无新依赖。

**实际文件/符号**

- `src-tauri/src/workflow/react/campaign.rs`（新增）：strict `campaign_plan.v1` /
  `candidate_manifest.v1` / `campaign_run_request.v1` / `campaign_summary.v1` 契约
  （`deny_unknown_fields` + snake_case + 稳定 machine code `CampaignSpecErrorCode`）、
  domain-separated canonical hash（plan/candidate/surface/envelope/catalog/campaign-id/
  candidate-scope-id/trial-scope-id）、checked-in prompt catalog（`include_str!` 编译期内嵌，
  拒绝任何 runtime 路径）、`resolve_workflow_prompt_override`（workflow snapshot 只存
  ref/hash/catalog digest，正文只在内存中解析，catalog 漂移 fail closed）。
- `work/agent-cli-candidate-surfaces/catalog.json`（新增，checked-in）：唯一 allowlisted surface
  `smoke-terse-v1`（`prompt_hash` 为 `cs-campaign:candidate-prompt` domain hash，加载时校验）。
- `src-tauri/src/db/budget.rs`：`create_campaign_atomic`（幂等，envelope 不一致 fail closed）、
  `create_campaign_run_atomic`（单 writer 事务内校验 campaign active/envelope 一致、复用或创建
  共享 candidate/trial scope、新建 per-run request scope + workflow row）、`close_budget_scope`
  （幂等，ledger 记 `pause`/`closed:` 前缀）、`list_budget_child_scopes`；
  `get_budget_scope_chain` 改为**按 durable parent linkage** 解析并校验四级 kind/环/完整性
  （v1 单 run 的 suffix 链因本就写入真实 parent 而继续可用，suffix 不再作为 authority）。
- `src-tauri/src/commands/workflow.rs`：抽出唯一 run kernel `create_and_start_budgeted_run`
  （`BudgetedRunSetup`/`BudgetedRunScope::{Standalone,Campaign}`），2C `run_experiment_core` 与
  2F `campaign_run_core` 共用；`campaign_create_core`/`campaign_get_core`/`campaign_close_core`/
  `campaign_run_core`；`build_resolved_workflow_config` 增加仅 campaign facade 使用的
  experiment prompt 参数；`workflow_start_core` 在构造 executor 前解析 workflow-local prompt ref
  （失败即 `experiment_prompt_rejected: <code>`，不降级为 baseline prompt）。
- `src-tauri/src/db/agent.rs`：`AgentConfig` 新增 `experimentAgentPromptRef/Hash` +
  `experimentPromptCatalogDigest`（只存 ref/hash/digest，从不写 Agent 记录）；
  `validated_inherited_agent_config` 剥离这三个字段（`--agent-config`/继承快照无法越权选择
  experiment surface）；`sync_workflow_agent_config_at_tool_boundary` 从本 workflow 自身快照
  回填（tool-boundary 能力同步不丢 frozen run identity）。
- `src-tauri/src/workflow/react/engine.rs` / `orchestrator.rs`：runtime config 重写透传
  experiment prompt identity（来自 preserved snapshot）；child-agent 会话显式置空
  （candidate 不得改写 child agent prompt）。
- `src-tauri/src/workflow/react/application.rs`：`campaign_create/get/run/close` 唯一 facade。
- `src-tauri/src/workflow/react/client/http/{server.rs,dto.rs}`：additive 路由
  `POST /control/v1/campaigns`、`GET /control/v1/campaigns/{id}`、
  `POST /control/v1/campaigns/{id}/runs`、`POST /control/v1/campaigns/{id}/close`
  （bearer + 必填 Idempotency-Key；path campaign id 权威；body 禁止 scope id；
  稳定 machine code 直接上 wire）。**偏差说明**：close 路由用 `/close` 段而非 `{id}:close`，
  因为 axum/matchit 不支持同段内 path parameter + 静态后缀；契约其余不变。
- `src-tauri/src/bin/cs/{campaign.rs,args.rs,cs.rs}`（campaign.rs 新增）：
  `cs experiment campaign create|run|inspect|close`；CLI-local sidecar 布局
  `<out>/campaign/{main,candidates,runs,summary}` 与 `<out>/evidence/<key>-<run_id>/{artifact,
  evaluation,verdict}` 恒为兄弟目录；sidecar 复用 `SidecarBundle`（staging + re-verify +
  atomic rename + symlink/overlap 防御），manifest domain 独立为
  `cs-campaign-sidecar:manifest`；独立 verdict consumer（先 `verify_sidecar_dir`，再重算
  verdict 内容 hash、校验 artifact run/session/chain_head 绑定、fixture 身份/digest、
  safety/infra/budget 事实、拒绝 promotion 字段）；sidecar 只记录 campaign-local
  `evidence_dir_hint`，不写绝对路径。`inspect` 完全离线。
- `src-tauri/src/lib.rs`：`pub mod campaign` 窄导出（仅纯契约类型/函数），使 `cs` 与 backend
  共享同一 parser/hash 实现；`cs` 仍不打开 DB、不启动 runtime/executor。
- `src-tauri/src/bin/cs/{evaluate.rs,verifier.rs,experiment.rs}`：抽出非渲染 seam
  `evaluate_artifact_offline` / `verify_artifact_offline`，`capture_bundle`/`wait_for_terminal`/
  `budget_rejected_in_events` 改 `pub(crate)` 供 campaign runner 复用；`SidecarBundle.file_name`
  改为 `String`；新增 `sidecar_manifest_hash`。

**验证命令与结果（最终态）**

```text
cd src-tauri && cargo fmt --all -- --check        # clean
cargo check --bin chatspeed --bin cs              # 0 warnings
cargo test --lib db::budget::                     # 24 passed（含共享 scope/聚合 cap/close/复用/parent-linkage）
cargo test --lib migration                        # 15 passed（无新 migration，仍为 v18）
cargo test --lib workflow::react::campaign        # 20 passed
cargo test --lib workflow::react::client          # 45 passed（含 4 个 campaign HTTP 路由测试）
cargo test --lib workflow::react::experiment      # 10 passed
cargo test --lib commands::workflow               # 67 passed（含 prompt identity 继承/同步测试）
cargo test --lib workflow::react::llm             # 含 campaign_prompt_override_changes_the_single_assembled_system_prompt
cargo test --bin cs                               # 117 passed（含 7 个 2F campaign/verdict 消费测试）
pnpm test:workflow                                # 62 passed
```

**真实 desktop smoke**：见 `work/agent-cli-phase-2-smoke-test.md` `## 6`。要点：隔离
`CHATSPEED_HOME` 的真实 `pnpm tauri dev` 实例（未触碰用户另一 checkout 的在跑实例）；三个独立
campaign（`p2f-smoke-21/22/23`）各 baseline + 单变量 prompt-ref candidate，6 个 run 全部 durable
`completed`、score 1.0、safety/infra pass、cost known，全部 artifact→evaluate→verify→consume 并
`close`；负向覆盖 campaign cap 在 provider 前拒绝（`rejected before send`，外部 provider 零调用）、
重复消费、close 后 run（`campaign_not_active`）、篡改 sidecar（`hash_mismatch`）；退出审计
TERM 本次进程组后 12 个 PID 全部退出、用户实例未受影响。首选模型 `cs@free:ds-v4-flash` 因免费配额
持续 429 无法完成三 campaign，经用户确认改用同为免费组的 `cs@free:qwen-3.8-flash`（未使用收费模型）。

**已知限制 / 后续**：mid-run admission 拒绝只记录在 server 日志（不在 durable events/snapshot），
CLI 对该类 run 以 `campaign_stopped`(exit 1) 停止而非 exit 9（与 2C 现状一致）；campaign 的
plan/candidate/summary 事实由 CLI-local sidecar 保存，backend 不持久化 plan（重启后由 plan hash
重新推导 campaign identity），真正可恢复的 headless scheduler 留待 2H。

## 10. 2D+2E Active Plan（当前执行范围）

> 本节是当前 active scope 的执行契约（2026-09-15 批准）。历史 2A/2B/2C 记录见 `## 9`，原样保留，
> 不删除、不清空、不改写。本节在实现开始前写入；其中 manifest/verifier digest 在实现时生成后回写，
> 占位描述不作为已验证证据。

### 10.1 目标与交付物

1. **2D correctness evaluator 首版**：只对已通过 2A 离线 verifier 的 artifact 做确定性、只读事实投影，
   输出独立 evaluation sidecar；本期不实现 LLM judge，不做 promotion。
2. **2E 第一条 benchmark adapter/verifier 垂直切片**：使用仓库内固定 `chatspeed-smoke` 小型 fixture，
   固定 `version/digest/split/verifier/resource profile`，复用 2C 唯一 experiment-run control-plane
   入口生成 artifact，由独立、确定性 verifier 产出绑定到 artifact 的分数事实。
3. 执行顺序：先本节文档对齐（U-1），再 evaluator（U-2）→ fixture/adapter（U-3）→ verifier（U-4）→
   验证与回写（U-5）。

### 10.2 范围边界与非目标

本期包含：确定性 evaluator、固定 smoke fixture、adapter、独立 verifier、CLI 离线/运行入口、
sidecar schema、focused tests、文档回写和可行时的 desktop smoke。

本期不包含：

- LLM judge 或任何新的 judge 外部 effect；
- 2A v1 `run.json/snapshot.json/events.jsonl/result.json/manifest` 的回写或 schema 破坏性升级；
- promotion、candidate、campaign、GEPA/DGM、lineage、自动 proposer；
- Harbor installed-agent 的真实容器运行、headless/daemon、worktree/patch apply、独立实验 data domain、
  MCP/skill 安装（属 2G/2H）；2E 只先固定未来 Harbor 接入所需的 adapter/verifier 合约；
- 新 DB migration、evaluator/benchmark 表、Tauri command/event wire 改动、既有 `/control/v1` 语义改动、
  旧 static/ccproxy router 改动；
- 保存原始 prompt、response、transcript、patch、完整环境变量、token、API key 或 private holdout 内容。

### 10.3 目标行为与 sidecar 契约

- `cs experiment evaluate <artifact-dir> --output <evaluation-dir>`：在无主进程、无 discovery、无网络、
  无数据库、无 LLM key 环境运行；先调用 `artifact::verify_bundle_dir`，再对可信 `VerifyReport` 做
  确定性检查，并在独立 evaluation sidecar 中原子发布结果；不改变 2A artifact 目录。
- evaluation sidecar 布局：`<evaluation-dir>/evaluation.json` + `<evaluation-dir>/artifacts/manifest.json`；
  sidecar manifest 只覆盖 sidecar data file（schema、algorithm、size、sha256、manifest_hash），
  不向 2A manifest 添加文件。`evaluation.json` 至少包含 `schema_version`、`evaluation_kind`、
  `evaluator_id/version`、`source_artifact{path_hint,run_id,session_id,artifact_schema_version,chain_head}`、
  `checks[]`、`correctness_status`、`provenance`、`created_at`；不包含原文 prompt/response/transcript，
  不包含 promotion verdict。
- `cs experiment benchmark run --suite chatspeed-smoke --task <task-id> ...`：复用既有
  `POST /control/v1/experiments:run`、2C spec 与预算 admission，完成后复用现有 capture 路径得到
  artifact；不新增第二套 runtime/HTTP/run loop。
- `cs experiment benchmark verify --suite chatspeed-smoke --task <task-id> --artifact-dir <dir>
  --output <dir>`：离线环境中由固定 verifier 检查 artifact 与 task manifest，输出绑定到 artifact 的
  score/verdict 事实；模型自述、普通文本或模型生成日志不能单独构成分数。
- verdict sidecar 布局：`<verdict-dir>/verdict.json` + `<verdict-dir>/artifacts/manifest.json`。
  `verdict.json` 至少包含 `schema_version`、`verdict_kind`、`dataset_id/version/digest/split`、
  `task_id/task_digest`、`verifier_id/version/digest`、`source_artifact{run_id,session_id,chain_head}`、
  `score`、`metrics`、`safety_status`、`infra_status`、`budget_facts`、`provenance`；
  promotion 结论不得由 verifier 产生，`promotion_status` 不出现在 verdict 之外的任何结论中。

### 10.4 Benchmark fixture 基线（chatspeed-smoke@2）

- 身份固定：`dataset_id=chatspeed-smoke`、`dataset_version=2`、`split=smoke`。
- task manifest 新增于 `work/agent-cli-smoke-benchmark/`（strict snake_case JSON 解析）；第一版使用
  少量不涉及原文持久化的 control-plane-observable smoke tasks。任务 id、instruction hash、
  expected structured facts、resource profile、verifier id/version 固定在 manifest；实际 prompt 仅在
  adapter→run 请求期间使用，不写入 artifact。
- digest：对 canonical manifest（排序 key、固定字段、无时间字段）做 domain-separated SHA-256。
  **已实现并回写的真实 digest（golden tests 锁定，见 `benchmark::tests::fixture_digests_match_golden_values`；
  桌面 smoke 阶段因资源 cap 校准调整过一次，见 Implementation Record）**：
  - `manifest_digest`（domain `cs-benchmark:manifest`，覆盖 manifest + 全部 task 文档按声明顺序；
    verifier v2 fixture）：
    `fcedab561697fbce2e095c04969e9313900bc73a7ed2e09c1b35bc89d2738918`
  - `task_digest[smoke_reply_ok]`（domain `cs-benchmark:task`）：
    `85acd5e8d744b20f0ce537a17ca6ae51aa2caaa4b3f71853a81fb1713f7d5405`
  - `task_digest[smoke_echo_ping]`（domain `cs-benchmark:task`）：
    `5ce34ddef4ad2586e621a858cf810bff388596423c6ccb9bf2040b961425ed04`
  - `verifier_digest`（domain `cs-benchmark:verifier`，绑定 verifier id/version + expected facts +
    resource profile；verifier v2）：`c9b6e8cd08cce5d3d9fb8c95e3fc95569c878e7167b0267a2eec6c5c7a6f21ef`
  - instruction hash domain：`cs-benchmark:instruction`（`smoke_reply_ok` =
    `f58cc8905c59cc469e451f19c4141d1f6f0a59dce07a40a37040078236de4b40`）
- resource profile 仅使用 2C 当前可观测且已支持的 `input_tokens/output_tokens/cache tokens/
  wall_time_ms/tool_calls/processes/concurrency` hard caps；disk/network 为 not_applicable；
  budget rejection/unknown settlement 视为不可通过的完整性/基础设施事实，而非 correctness 成功。
- verifier 为仓库内独立模块，读取固定 fixture 与已验证 `VerifyReport`，不读取模型生成日志、不接受
  运行时 verifier path；本期明确为 contract-level candidate-untrusted boundary，不宣称 2G/Harbor 的
  OS/container 隔离。

### 10.5 执行单元与验证映射

- **U-1** 本节文档对齐（AC-1、INV-1）→ V-1。
- **U-2** 2D deterministic evaluator + atomic evaluation sidecar（AC-2/3/4/8；INV-1..5）→ V-2/V-3/V-6。
- **U-3** `chatspeed-smoke@2` manifest + local control-plane adapter（AC-5/6/8；INV-1..4）→ V-4/V-6/V-7。
- **U-4** independent deterministic verifier + verdict sidecar（AC-5/7/8；INV-2/4/5）→ V-2/V-5/V-6。
- **U-5** focused verification、desktop smoke（可行时）与 Implementation Record 回写（AC-1..8）→
  V-1/V-7/V-8。
- 依赖：U-1 → U-2 → U-3 → U-4 → U-5，无环。

### 10.6 关键不变量（本期重申）

- **INV-1** 既有外部契约不变（Tauri command、workflow event、GatewayPayload、既有 `/control/v1` route、
  既有 `cs workflow`/`cs experiment run|capture|inspect|replay` 参数与输出语义）。
- **INV-2** 单一 runtime/data authority：CLI 不打开 SQLite、不创建 executor/lifecycle/input loop；
  benchmark run 只能复用 2C control-plane/application/workflow kernel；离线 evaluator/verifier 只读
  artifact/fixture。
- **INV-3** evaluator/verifier 本身零 LLM/tool/MCP/network effect；benchmark run 的唯一 effect 是既有
  2C 单次 run，必须经过现有预算 admission，默认不 retry。
- **INV-4** 所有新事实区分 `artifact/evaluator/benchmark_adapter/independent_verifier` provenance；
  无法安全脱敏或无法证明来源绑定时拒绝发布，不降级为原文写入或模型自报。
- **INV-5** sidecar/verdict 使用 staging、完整性校验和 atomic rename；必须绑定已验证 artifact 的
  `run_id + chain_head` 与固定 fixture digest；篡改、额外任务、未知字段、路径穿越、重复发布或
  verifier 输入不一致都失败。

### 10.7 Stop conditions（命中即停止并请求用户确认）

需要改 2A v1 verifier/schema；需要把 sidecar 写回 artifact；需要新增 DB migration/Tauri wire/
control-plane route；需要直接从 CLI 调 LLM/tool/MCP 或新增 retry；需要真实 Harbor/headless/container/
worktree；需要保存原始 prompt/response/patch/transcript/secret；需要新增依赖；需要把 fixture score
宣称为公开 benchmark 或 promotion verdict；需要改变既有 run/budget/普通 workflow 契约。

### 10.8 完成回写规则

全部 U/V 完成后：在 `## 9` 末尾追加 `2D+2E Implementation Record (as built)`（含真实 manifest/verifier
digest、验证命令与结果、desktop smoke 证据或其明确限制），并把 `## 0` 指针推进到 **2F**；仅在所有
验收证据完成且无 pending 工作时推进。历史 record 不删除、不清空、不改写。

## 11. 2G+2H Active Plan 与部分实施记录（进行中，指针未推进）

> 本节记录 2026-09-16 开始的 2G+2H 实施。**`## 0` 指针保持 `2G+2H`，未推进到 2I**：
> 2G+2H 的 required gate（真实 Docker owner、真实 Harbor task、真实 headless 进程 + 真实模型 smoke）
> 尚未取得证据，按计划第 9 节第 8 条不得推进。历史 `## 9`/`## 10` 原样保留，未删改。

### 11.1 本轮已交付

- **U-1 完成 — 2G+2H 契约冻结 + fixture resolver 共享**
  - 新增 `src-tauri/src/workflow/react/experiment_schedule/{mod.rs,types.rs,fixture.rs}`。
  - `fixture.rs`：把 `chatspeed-smoke@2` manifest/task parser、digest 计算与 adapter 投影从
    CLI-only 的 `src-tauri/src/bin/cs/benchmark.rs` 移入 library；新增 `FixtureTaskRefV1`
    （只含 suite/dataset/split/task_id + manifest_digest/task_digest/instruction_hash，**不含
    instruction 原文**）与 `resolve_task_ref`（dispatch 前按 pinned catalog 复算并 fail closed）。
  - `types.rs`：冻结 `campaign_schedule.v1` / `campaign_schedule_accepted.v1` / `campaign_job.v1` /
    `execution_profile.v1` / `bundle_manifest.v1` / `harbor_task_capability.v1` /
    `experiment_domain_marker.v1`；`ScheduleErrorCode` 稳定 machine code；job FSM
    （`JobState` × `DispatchMarker` 转移表 + `validate_dispatch_invariant`）与**纯函数**
    `classify_restart_recovery`（restart 分类唯一权威）。
  - 隔离面按构造 fail closed：`ExecutionProfileV1` 的 mount 只允许 `workspace|bundle`、
    `persistent_docker` 必须 digest-pinned image、`BundleManifestV1` 拒绝绝对/越界路径、远端抓取与
    疑似 secret 值、`HarborTaskCapabilityV1` 只接受绝对 root 且提供 `contains_owned_path`。
  - `src/bin/cs/benchmark.rs` 改为共享 resolver 的薄适配层；2E golden digest 未变。
- **U-2 完成 — v19 migration + ExperimentDomain guard + durable store**
  - `src-tauri/src/db/sql/migrations/v19.rs`：纯 additive 建表
    `experiment_domain` / `experiment_domain_lease` / `experiment_campaign_schedules` /
    `experiment_campaign_jobs` / `experiment_job_journal`（唯一 `AUTOINCREMENT`）/ `experiment_job_bundles` /
    `experiment_job_artifacts` + claim/campaign 索引；外部 ID 全为 TEXT，`dispatch_marker='confirmed'`
    与 `run_id IS NOT NULL` 由 CHECK 绑定。
  - `src-tauri/src/db/experiment_schedule.rs`：`ExperimentScheduleStore`（schedule 事务 + 有序 job、
    key/body 幂等、CAS claim/adopt/heartbeat/fenced transition/record_stage/dispatch intent、
    `park_unknown_manual`、`classify_restart`、cancel）。claim 只取 `not_dispatched` 且
    `queued`/lease 已过期的 pre-dispatch job；`dispatching` 永不被认领或 adopt；`unknown_manual`
    只在纯分类器判定 unknown 且 lease 非 live 时写入。
  - `src-tauri/src/headless/domain.rs`：`ExperimentDomain` 固定布局 + owner-only 权限 + 拒绝 symlink
    data dir + **拒绝无 marker 的既有 DB** + singleton generation-fenced domain lease。
- **U-3 部分完成 — transport-neutral runtime seams + `chatspeed-headless` binary**
  - `client/hub.rs`：新增 `WorkflowEventTransport` trait；desktop 用 `TauriGateway` 实现，headless 用
    `NoWindowTransport`（无自有 sink，SSE broker 是唯一观测面）；`WorkflowRuntimeHub::with_transport`
    保留唯一 input registry 与 broker。
  - `client/http/discovery.rs` + `server.rs`：discovery 读写全部改为显式 runtime dir
    （`write_discovery_in`/`read_discovery_in`/`remove_discovery_if_instance_in`），headless 在
    `<data-dir>/runtime/` 发布自己的 discovery，desktop 默认路径语义不变。
  - `ccproxy/router.rs`：`SharedState.app_handle` → `package_version: String`（`/api/version` 唯一用途），
    `routes(package_version, ...)` 不再要求 `AppHandle`；route 顺序/auth/ModelResolver/header 过滤未动。
  - `tools/tool_manager.rs`：抽出 `register_core_tools(main_store)`（AppHandle-free：FS/search 工具），
    desktop `register_available_tools` 在其之前注册 web 工具后再调用它。
  - `src-tauri/src/headless/bootstrap.rs` + `src/bin/chatspeed_headless.rs`：`HeadlessOptions`
    （`--data-dir`、显式 `--config-package` + `--config-category`、`--api-key-file`）、
    config/key preflight（locked 即 fail closed）、`ChatState::new(.., None, ..)`、
    `WorkflowRuntimeHub::with_transport(NoWindowTransport)`、manager/factory/application 组装、
    domain discovery 上的控制面、domain lease heartbeat、SIGINT/SIGTERM graceful shutdown；
    **启动失败会释放已取得的 domain lease**。

### 11.2 本轮验证证据（实际命令与结果）

```text
cd src-tauri
cargo fmt --all -- --check                                   # clean
cargo check --bin chatspeed --bin chatspeed-headless --bin cs # 0 warnings
cargo test --lib db::experiment_schedule                     # 15 passed
cargo test --lib headless                                    # 15 passed（domain 10 + bootstrap 5）
cargo test --lib migration                                   # 16 passed（含 v18→v19 additive 升级保数据）
cargo test --lib workflow::react::client                     # 51 passed（control plane + hub transport）
cargo test --lib workflow::react::campaign                   # 20 passed
cargo test --lib workflow::react::experiment                 # 68 passed
cargo test --lib commands::workflow                          # 67 passed
cargo test --lib db::budget::                                # 24 passed
cargo test --bin cs                                          # 108 passed
```

关键行为证据：
- 2E golden digest 在 resolver 迁移后不变（`experiment_schedule::fixture` 与 CLI 双侧断言）。
- `durable_rows_never_store_the_instruction`：对 `chatspeed.db` **原始字节**扫描，确认 fixture
  instruction 与 `instruction"` 键均不出现在库中。
- `a_desktop_style_database_gains_only_empty_tables`：v19 后 desktop 风格 DB 仅新增空表、marker 行数为 0。
- `only_one_worker_wins_a_claim_and_the_generation_is_fenced` / `a_superseded_worker_cannot_heartbeat_or_transition`：
  双 worker claim 单赢、generation 递增、旧 worker heartbeat/stage/transition 全部 `lease_lost`。
- `a_dispatch_intent_is_never_removed_by_automatic_recovery`：记录 intent 后不再可 claim、不可 adopt、
  live lease 时不可 park，lease 过期后 park 为 `unknown_manual`（`error_code=dispatch_uncertain`）且不可逆。
- `a_fresh_domain_starts_a_windowless_runtime` / `a_second_instance_on_the_same_domain_fails_closed`：
  headless 进程内真实启动 runtime + 控制面 + 自有 discovery，第二实例 `experiment_domain_locked`，
  shutdown 后 lease 释放可重启。
- `config_package_import_is_explicit_and_fail_closed` / `a_missing_api_key_file_fails_before_the_runtime_starts`：
  凭据/配置失败全部发生在 runtime 之前，且失败不残留 lease、不发布 discovery。

### 11.3 未完成项与阻塞（不得视为已通过）

> 本节在 2026-09-16 晚更新：U-3..U-8 的主体已交付（见 11.1/11.6），仍缺的见下。

- **U-3 完成**：web tool 显式拒绝已实现（`preflight_capabilities()`，code `web_tools_unsupported`，
  测试 `an_agent_requiring_web_tools_fails_closed_at_startup`）。
  打包层面仍待办：`chatspeed-headless` 链接整个 `chatspeed_lib`，运行时需要 GTK3 运行库
  （非功能阻塞，见 11.5；发行时需 feature-gate tauri 或镜像内提供 GTK）。
- **U-4 已交付**：`headless/profiles.rs`（服务端 execution profile 注册表）、
  `commands/workflow.rs` 的 `campaign_schedule/jobs/job/cancel/reconcile` core、
  additive `/control/v1` durable routes、`bin/cs/schedule.rs` 的
  `cs experiment campaign schedule|jobs|job|cancel|reconcile`（纯 HTTP）。
- **U-5/U-6/U-7 已交付**：`workflow/react/experiment_owner/`
  （`mod/patch/worktree/docker/harbor_task/bundle/capabilities`）：ExecutionOwner 契约、
  run worktree + 输入补丁 + 输出 `patch.diff`+manifest 原子发布、label/token/generation 围栏的
  PersistentDockerOwner、capability-manifest HarborTaskOwner、bundle acquire→stage→verify→release
  与 `PreparedCapabilityLease`。
- **U-8 部分交付（当前阻塞点）**：`experiment_schedule/scheduler.rs` 的有界 tick（recover/CAS
  claim/owner saga/dispatch intent/权威终态/collection/cleanup）、
  `headless/scheduler_runtime.rs` 的资源解析与 kernel 适配、`bootstrap.rs` 的 supervisor 接线
  均已实现并测试。**但 scheduled dispatch 现在是 fail-closed**：
  owner 确认的实例（`PreparedWorkspace`/`ContainerHandle`）尚未接入 run 的 shell execution plan，
  已验证的 `PreparedCapabilityLease` 也尚未注册进 session 级 MCP/skill overlay。
  在接线完成前，任何 scheduled job 都会以
  `owner_execution_context_unavailable`（`failed_precondition` + `not_dispatched`）结束——
  这是刻意的安全行为（INV-4：宁可拒绝，也不在 owner 之外无隔离执行），
  **不是**“已接线”。精确续做步骤见 11.6。
- **U-9 未开始**：`tools/harbor/` 0.23.0 pin、custom installed-agent、separate verifier fixture。
- **U-10 未开始**：crash matrix、真实 Docker owner gate、真实 Harbor、一次 pre-dispatch 重启恢复、
  真实模型 smoke（`cs@qwen-3.8-flash` → `cs@free:gemini-flash` → `cs@free:ds-v4-flash` →
  `cs@qwen3.8-flash`）与退出审计。

因此 **AC-2 的“调度执行面”仍缺（因 U-8 接线未完成）、AC-3/AC-4 的“run 内隔离与能力注册”仍缺、
AC-7 全部、AC-9 的 exit gates 仍缺**。已取得证据覆盖 AC-1 的 domain/layout/marker/lease 语义、
AC-2 的 durable 契约/持久化/租约/分类/cleanup 语义、AC-3/AC-5 的 owner 与补丁语义（含真实 Docker
容器生命周期与真实 worktree 主树只读）、AC-4 的 bundle 校验与租约语义、AC-6 的 HTTP-only CLI、
AC-8 的既有契约回归。指针保持 `2G+2H`。

### 11.4 已知的既有失败（非本轮引入）

`cargo test --lib ccproxy` 中
`ccproxy::adapter::backend::gemini::tests::supported_gemini_model_serializes_normalized_thinking_level`
失败（`Some("xhigh")` vs `Some("high")`）。该文件本轮未修改，失败与本轮改动无关（本轮 ccproxy 只把
`SharedState.app_handle` 换成显式 `package_version`）。

### 11.5 真实进程级 smoke（2026-09-16，更正 11.3 的早期结论）

**更正**：11.3 曾把“`chatspeed-headless` 无法启动”记为 GTK 阻塞。该结论**错误**：当时的失败来自
一次性执行环境的 rootfs/glibc 与库路径差异（`libgdk-3.so.0` 不可加载），**不是二进制或架构问题**。
在真实宿主环境（与 `pnpm tauri dev` 同一环境）中，`chatspeed-headless` 独立进程可正常启动、服务
控制面并优雅退出，证据见下。

**A. 真实 desktop（`pnpm tauri dev`，隔离 `CHATSPEED_HOME`、覆盖端口 1432）**

```bash
CHATSPEED_HOME=<repo>/dev_data/p2gh-home pnpm tauri dev --config \
  '{"build":{"devUrl":"http://localhost:1432","beforeDevCommand":"pnpm exec vite --port 1432 --strictPort"}}'
```

- 编译完成后真实启动：`[ControlPlane] Published discovery document at
  <repo>/dev_data/p2gh-home/runtime/control-plane-v1.json`、
  `[ControlPlane] Listening on 127.0.0.1:36053 (instance ca00652e…, pid 1344349)`；
- v19 迁移在**真实 969MB dev 库**上生效：`Database is already up to date at version 19`；
  库内 `experiment_domain`/`experiment_domain_lease`/`experiment_campaign_schedules`/
  `experiment_campaign_jobs`/`experiment_job_journal` 全部 **0 行**，即 desktop 库**未被自动标记**为
  实验域（AC-8/INV-9）；
- ccproxy 路由矩阵与改动前**逐行一致**（`--- ccproxy routes registered ---` 段：固定前缀 →
  OpenAI/Claude/Gemini/Ollama 直连 → compat → grouped），确认 `AppHandle → package_version` 重构
  未改变 route composition/order；
- 前端已在实际渲染（`get_system_skills returned 51 skills`、`get_workflow_snapshot` 周期调用成功），
  即窗口已打开并与 backend 正常通信；
- CLI 端到端：`cs --discovery-file <repo>/dev_data/p2gh-home/runtime/control-plane-v1.json doctor`
  → `Connected … endpoint http://127.0.0.1:36053, protocol v1, instance ca00652e…, pid 1344349`；
  `agent list` 返回 builtin agents、`workflow list` 返回真实会话 id。

**B. 独立 `chatspeed-headless` 进程（独立 data dir `dev_data/p2gh-headless`）**

- 启动：`chatspeed-headless --data-dir <repo>/dev_data/p2gh-headless` →
  固定布局 `artifacts/ bundles/ journals/ runtime/ worktrees/` 全部 0700、自有 `chatspeed.db`、
  `runtime/control-plane-v1.json` 0600；`domain domain-f97e34d916a3a8714a8d44556be9964a
  listening on 127.0.0.1:35525 (instance 7ec97bb8…)`；
- 独立数据域确实隔离：`cs doctor` 通过（`endpoint …:35525, instance 7ec97bb8…, pid 1334871`），
  而 `agent list`/`workflow list` **为空**——headless 域没有 desktop 的任何 agent/workflow 数据；
- fail-closed：同一 data dir 再启一个实例 → 退出码 **1**，
  `chatspeed-headless: experiment_domain_locked: experiment domain is locked by live owner
  'headless-1334871-fab23fe09a9700a9' (generation 1)`；
- 优雅退出：SIGTERM 后进程退出且 **discovery 文档被删除**（`discovery_exists false`），
  同域重启成功且 **domain id 不变**、lease generation 回到 **1**（干净释放而非过期接管）；
- 收尾：headless 实例已停止，其 discovery 已清理；`dev_data/p2gh-headless/{chatspeed.db,journals}`
  作为本次证据保留。

**尚未取得**（2026-09-16 晚更新，取代上文早期措辞）：U-8 的**引擎级接线**、U-9（Harbor 0.23.0）、
U-10（crash matrix、真实 Harbor、真实模型 smoke、退出审计）。U-4..U-7 的 durable schedule 提交/查询/
取消、owner/worktree/container/bundle 证据已在 11.6 补记。

### 11.6 2026-09-16 晚：评审 blocker 修复、主机级进程证据与续做说明

#### 11.6.1 本轮修复的三个真实缺陷

1. **调度终态语义反转（评审 blocker，本轮代码引入）**
   - 根因：同一个 `bool` 在两处含义不同——重启分类器里表示“可证明终态”，调度 tick 里被当成“是否成功”。
     后果：live run 被判为终态失败（立即 `Failed(run_failed)` 并拆环境）；`error`/`cancelled` 反被记为
     `Succeeded`。
   - 修复：`experiment_schedule/scheduler.rs` 引入显式 `RunVerdict{Running,Succeeded,Failed}`，
     `recover()` 用 `terminal_flag()`（仅“可证明终态”为 `Some(true)`）；
     `headless/scheduler_runtime.rs` 新增纯函数 `workflow_status_verdict()`：
     `completed`→Succeeded，`error|cancelled|failed`→Failed，其余（含未知状态）→Running。
   - 证据：`headless::scheduler_runtime::tests::the_workflow_status_verdict_separates_terminality_from_success`
     对真实状态串全覆盖（含 `awaiting_*`、`pending/thinking/executing/auditing/stopping/paused` 与未来未知值）。

2. **缺 owner 上下文时不再可能无隔离执行（评审 blocker 的最小要求）**
   - 新增 `ScheduledRunKernel::preflight_dispatch()`：**在写入 `dispatching+intent_recorded` 之前**询问
     kernel 是否能兑现 owner 上下文；不能则直接拒绝。
   - 过程级发现并修复时序缺陷：早期把意图写在拒绝之前，导致 job 卡在 `dispatching/intent_recorded`
     （重启后会被 park 成 `unknown_manual`，而非干净的预派发失败）。现在拒绝发生在意图之前。
   - 证据：`a_pre_dispatch_refusal_ends_the_job_cleanly_without_an_intent`（job `failed_precondition`、
     `not_dispatched`、无 run id、kernel 零派发、worktree 已回滚）。

3. **headless `--base-repo` 被静默丢弃（过程级新发现）**
   - 根因：clap 解析出该 flag，但二进制从未把它写入 `HeadlessOptions`（替换未生效）。
   - 修复：`bin/chatspeed_headless.rs` 提取 `build_options()` 并补全映射；
     回归测试 `the_command_line_flags_reach_the_startup_options` 断言每个 flag 都到达 options。

#### 11.6.2 主机模式下的真实进程级证据（2026-09-16 晚）

隔离目录 `.cs/2g-smoke/`（已删除），base repo 为临时 git 仓库，域内预置 1 个 primary agent 与
1 个 `smoke-local`(host_worktree) execution profile：

- 启动：`chatspeed-headless --data-dir … --base-repo …` 正常启动并发布 discovery；
  `cs doctor --discovery-file …` → `Connected … protocol v1 … Connectivity, authentication and protocol
  version are OK`（确认 11.5 的更正：早期 GTK 失败只是沙箱缺库）。
- 提交：`cs experiment campaign schedule --plan plan.json --profile smoke-local` 经真实 HTTP 受理，
  返回 `campaign_schedule_accepted.v1`、2 个有序 job、`plan_hash`/`schedule_hash`（CLI 纯 HTTP）。
- 调度：journal 记为 `workspace_acquired`×2 → `environment_ready`×2，**无 `dispatch_intent`**；
  两个 job 终态 = `failed_precondition` + `not_dispatched` + `error_code=owner_execution_context_unavailable`
  + 无 run id。
- 无隔离缺失：`workflows` 表 **0 行**（没有任何 run 被启动）；`worktrees`/`artifacts` 为空；
  base repo `status` 洁净且 `worktree list` 仅一个（无残留 worktree）。
- 优雅退出：SIGTERM 后实例自行删除 discovery、`experiment_domain_lease` 归 0；无残留 `cs-run`
  容器；smoke 目录已删除。未安装任何系统包，桌面 `pnpm tauri dev` 与 `/usr/bin/chatspeed` 未受影响。

#### 11.6.3 U-8 引擎级接线的精确续做步骤（当前唯一阻塞 AC-2 执行面的项）

> 2026-09-16 晚补充：已定位到实际注入点（下一轮可机械执行，无需再摸索）。

1. **注入点（已核对）**
   - run 的 sandbox 配置来源链：
     `campaign_run_core` → `create_and_start_budgeted_run`（`commands/workflow.rs:1926`）→
     `build_resolved_workflow_config`（1871）→ `build_workflow_config_for_request` +
     `resolve_agent_sandbox_snapshot(store, agent, &mut config)`（1883）。
     **owner 实例必须在 `resolve_agent_sandbox_snapshot` 之后强制覆盖**（否则会被 agent/scheme
     的解析结果覆盖掉）。
   - `commands/workflow.rs::build_agent_config_from_agent`（约 1138–1152）设置
     `config.sandbox_execution_mode = Some(agent.sandbox_execution_mode.clone())` 与
     `config.sandbox_scheme_id`，这是该快照的上游。
   - sandbox 的持久实例名存在于 `tools/sandbox/types.rs`：
     `AgentSandboxConfig.profiles: BTreeMap<String, SandboxProfileConfig>` 中的
     `SandboxProfileConfig.instance_name: Option<String>`（约 414），并被带到
     `ShellExecutionPlan.instance_name: Option<String>`（201）/
     `ShellExecutionPlanDetails.sandbox_instance_name`（231/254）。
   - `tools/sandbox/resolver.rs` 在 578 / 651 / 993 三处构造 plan 并显式写 `instance_name: None`
     （651 附近是 profile 路径）——owner 上下文必须在这条路径上把显式实例名代入，
     并禁止 Auto/Host 回退。
2. **需要的改动**
   - ✅ **已交付（本轮，2026-09-16 晚）**：`commands/workflow.rs` 新增
     `OwnerExecutionContext`（`owner_kind` / `instance_name` / `image_reference`）与其
     `apply(&mut AgentConfig)`：容器 owner 把 run 固定为
     `ShellExecutionMode::SandboxOnly` + `AgentSandboxConfig{runtime_preference: Docker,
     profiles:{"owner": SandboxProfileConfig{command_patterns: []（catch-all）,
     instance_name: Some(<container name>), image: <digest pin>}}}`（**禁 Auto/Host 回退**）；
     Harbor task owner 在“已被 capability manifest 证明的 task sandbox”内以 HostOnly 执行
     （沙箱本身即边界）；`host_worktree` owner 直接拒绝（无隔离执行环境，INV-4）。
     测试：`commands::workflow::owner_execution_context_tests` 4 项全通过
     （容器固定 / 文件系统 owner 拒绝 / 容器缺实例拒绝 / Harbor 沙箱内执行）。
   - ✅ **已交付（本轮）**：串入同一 run kernel —— `resolve_agent_sandbox_snapshot(store, agent, config, owner)`
     在 agent/scheme 快照**之后**调用 `owner.apply`（owner 永远胜出）；
     `BudgetedRunSetup.owner`、`campaign_run_core_with_owner(.., owner)` 与
     `WorkflowApplicationService::campaign_run_owned(..)` 已就位；**立即 2C/2F 路径全部传 `None`**，
     公共 DTO `CampaignRunRequestV1` 未改（INV-2）。`ScheduledDispatch` 增加 `owner_kind`
     （由 scheduler 从实际 owner 取得）；`ScheduledCampaignKernel::preflight_dispatch` 在写 intent
     **之前**解析 owner 上下文（取不到即干净预派发失败），`dispatch` 调用 owner-pinned kernel 入口
     ——**占位式无条件拒绝已移除**。
   - ✅ **已交付（本轮）**：run-scoped lease 的**内存通道**已就位 ——
     `WorkflowApplicationService` 持有 `prepared_leases: Mutex<HashMap<session_id, PreparedCapabilityLease>>`
     与 `register_prepared_lease` / `release_prepared_lease`；
     `create_and_start_budgeted_run` 在建出 session id 后即登记该 run 的 lease；
     `OwnerExecutionContext.capabilities`（仅内存，绝不落库，INV-6）由 scheduler 在 dispatch 时携带；
     scheduler 在 run 确认后记 `JobSagaStage::CapabilitiesRegistered`（journal 记 `lease.describe()`，
     不含秘密值），并在权威终态时释放该 session 的 lease（幂等）。
   - ✅ **已交付（本轮，executor 侧注入完成）**：`WorkflowExecutor` 新增
     `owned_capabilities` 字段与 `set_owned_capabilities()` / `register_owned_capabilities()` /
     `release_owned_capabilities()`；`ExecutionExecutor::new` 与 `PlanningExecutor::new` 增加
     `owned_capabilities` 参数（orchestrator 的子 agent 恒传 `None`），
     `workflow_start_core` 按 session id 从 service registry 取出并传入；
     session 级工具装配段（`engine.rs` 原 2768–2830 区间末尾）调用
     `register_owned_capabilities()`，把 lease 的 MCP servers 注册到 **`self.tool_manager`**
     （绝不进 `global_tool_manager`），`run_loop` 结束后在 spawn 内调用
     `release_owned_capabilities()` 注销；`ReActExecutor` trait 增加默认 no-op 方法并由两个
     executor 转发。scheduler 仍在 run 确认后记 `CapabilitiesRegistered`、终态释放 registry 条目。
   - ⏳ **仍缺**：真实容器 owner + 真实模型的端到端 scheduled run smoke（属 U-10）；
     skills 侧的 run-scoped 注入目前只记日志（未真正进入 SkillScanner 的 session 视图）。
     已核对的 skills 接缝：`workflow/react/skills.rs:46` 的
     `pub struct SkillScanner { search_paths: Vec<PathBuf> }`（`new(app_data_dir)` 按优先级组装搜索路径）
     ——run-scoped 技能应把 staged bundle 的技能目录加入该 session executor 的
     `skill_scanner.search_paths`（仅该 run 可见），run 结束时移除；这样才符合 AC-4 的
     “只有 verified bundle 进入 run-scoped capability registry”。
     注意 **格式约束**：`SkillScanner::scan()` 遍历每个 search path 下的**子目录**并要求
     `<skill_dir>/SKILL.md`（`skills.rs` 的 `try_load_skill`），而当前 `bundle_manifest.v1`
     的 `skills[].entry_path` 是**扁平文件**路径（如 `skills/smoke.md`）。
     因此要么把 bundle 的技能布局改为 `skills/<name>/SKILL.md`，要么在注入时合成该目录结构；
     二者取其一后，再把 `skill_scanner.add_run_scoped_path(...)`（需新增该 API，含移除）接上。
     已核对的最后接缝：executor 持有**两个** manager —— 进程级 `global_tool_manager`（161/1799，
     来自 DB）与 **session 级 `self.tool_manager`**（`engine.rs:1798`，executor 实际执行时用的是
     `engine.rs:2474` 的 `let tm: &Arc<ToolManager> = &self.tool_manager;`）。
     因此 lease 必须注入 **session 级** `self.tool_manager`，并在 run 结束时按 name 注销
     （`tool_manager.rs:837 register_mcp_server` / `:823 unregister_mcp_server`）；
     由于 executor 目前拿不到 `WorkflowApplicationService`，最干净的接法是
     把 lease 作为参数传入 `WorkflowExecutor::new`（三处构造点：
     `planners.rs:109`、`runners.rs:108`、`commands/workflow.rs:5279`，
     由调用方从 application service 的 registry 按 session id 取出后传入）。
     **注入时机（重要，已核对）**：`ToolManager::register_mcp_server(self: Arc<Self>, McpServerConfig)`
     是 **async** 且要求 `Arc<Self>`（`tool_manager.rs:837`），而 `WorkflowExecutor::new` 是**同步**的
     （`engine.rs:1676`）。因此 lease 的注入不能放在构造函数里，应放在 executor **异步运行路径**上
     ——即 engine 里“keep initial execution aligned with later runtime MCP configuration updates”
     那处按 run 同步 MCP 配置的位置，用 `self.tool_manager.clone()` +
     `register_mcp_server(config).await` 注册，并在 run 结束时用 `unregister_mcp_server(name)` 注销。
     映射：`RegisteredMcpServer{name, command: PathBuf, args, working_directory, env}`
     → `crate::mcp::client::types::McpServerConfig`（`mcp/client/types.rs:75`，stdio/command 形态，
     `name` 用 lease 的 name 以便幂等注销）。
     **注入点修正（最后核对）**：`refresh_workflow_mcp_runtime_capabilities`（`engine.rs:8452`）
     读的是 `self.global_tool_manager`（进程级、来自 DB），**不是**注入点；
     session 级 `self.tool_manager` 的实际装配在 `engine.rs` 约 2768–2830 那段
     （其中 `engine.rs:2801` 用 `tool_manager: self.global_tool_manager.clone()`，
     `engine.rs:2823` 调 `tm.register_mcp_tool_wrapper(..)`）。
     lease 必须注册到 **`self.tool_manager`**（每个 executor 一个，天然 session 作用域），
     **绝不能**注册进 `global_tool_manager`（否则跨 session 泄漏，违反 AC-4/INV-8）。
     由于注册是 async，应放在该异步装配段之后（或紧随其后的异步步骤），
     并把 `WorkflowExecutor` 增加一个 `owned_capabilities: Option<PreparedCapabilityLease>` 字段 +
     `set_owned_capabilities()`，由 `workflow_start_core`（`commands/workflow.rs:5279` 附近，
     构造 executor 处）按 session id 从 application service 的 registry 取出后设置
     （该函数已持有 `main_store`/`factory`/`session_id`，需要把 `svc` 或 lease 传进来）。
     已核对的接缝：`workflow/react/engine.rs:1798` 每个 workflow executor 自建
     `tool_manager: Arc::new(ToolManager::new())`（即 session 级作用域），
     `tools/tool_manager.rs` 提供 `register_mcp_server(config)`（837）与
     `unregister_mcp_server(name)`（`unregister_unified_tool` 别名，823），
     折叠式 MCP wrapper 走 `engine.rs:2823` 的 `tm.register_mcp_tool_wrapper(..)`。
     因此接线点应为：engine 组装该 session 的 tool manager 之后注入 lease 的 MCP/skills，
     并在 run 终态/失败时按 install_id + owner token 幂等注销（技能侧对应
     `workflow/react/skills.rs` 的扫描结果注入）。
     **设计约束（已确认）**：lease 的 `env` 是秘密值，**不能**经 `AgentConfig` 落库
     （持久化配置会进入 workflow 行，违反 INV-6）。因此应走**内存**通道：
     `WorkflowApplicationService` 持有 run-scoped prepared-lease registry
     （`session_id -> PreparedCapabilityLease`），scheduler 在 dispatch 成功后登记并记
     `JobSagaStage::CapabilitiesRegistered`；executor 在装配 tool manager 时按 session id 取出并注册；
     终态/失败时按 install_id + owner token 幂等注销。
     生产环境的 executor 构造点共三处：`workflow/react/planners.rs:109`、
     `workflow/react/runners.rs:108`、`commands/workflow.rs:5279`（`workflow_start_core`），
     三处都必须能拿到该 session 的 lease（或统一在 `WorkflowExecutor::new`/engine 内部按 session id 查询）。
   - ⏳ **仍缺一个 focused integration test**：证明 scheduled job 在容器 owner 下能真正运行
     （端到端；本轮只到“resolve 后由 kernel 入口携带 owner 上下文”这一层，需真实 Docker 环境跑）。
3. **移除占位拒绝**
   - `headless/scheduler_runtime.rs::ScheduledCampaignKernel::preflight_dispatch` 目前**无条件**返回
     `owner_execution_context_unavailable`；`dispatch` 同样只返回该错误。
     接线完成后：`preflight_dispatch` 只在“无法取得 owner 确认实例”时拒绝，
     `dispatch` 恢复调用 `campaign_run_core`（owner context 随内部参数传入），
     并保留 `require_owner_workspace`（`scheduler_runtime.rs`）作为防御性前置检查。
   - 需要一个 focused integration test 证明：scheduled job 能实际运行；缺 owner/lease 时仍在
     **写 intent 之前** fail closed（现有
     `a_pre_dispatch_refusal_ends_the_job_cleanly_without_an_intent` 覆盖后者）。
4. **诊断性（顺带）**：headless 进程当前没有日志 sink，scheduler/saga 的 `log::warn!` 不可见；
   建议在 headless 二进制安装 stderr logger，使调度失败可直接观察（本次只能通过 durable
   `error_code` + journal 观察）。

#### 11.6.4 U-9 前置（A-1）已在主机核实（2026-09-16 晚）

- `harbor==0.23.0` 可下载并已装入隔离 venv：`.cs/harbor-probe/venv`
  （`python3 -m venv` + `pip install harbor==0.23.0`，**未动系统包**；
  下一段可直接复用该 venv，避免重复安装）。
- **`BaseInstalledAgent` 的实际位置**：`harbor.agents.installed.base`
  （`class BaseInstalledAgent(BaseAgent, ABC)`，同模块还导出 `with_prompt_template`）。
  注意：它**不在** `harbor.agents` 顶层，按顶层名查找会失败。
- **参考适配器模板**：`harbor/agents/installed/pi.py` ——
  `class Pi(BaseInstalledAgent)` + `PiOptions(InstalledAgentOptions)`（pydantic `Field` +
  `Cli("--flag")` 注解声明 CLI 选项），并导入
  `harbor.agents.capabilities.AgentCapabilities`、
  `harbor.environments.base.BaseEnvironment`、
  `harbor.models.agent.context.AgentContext`、
  `harbor.agents.model_connection.{ModelConnectionSpec, ResolvedModelConnection}`。
- 因此 U-9 可直接按此模板实现，无需再猜版本漂移 API（A-1 的“若不一致则停止并更新 pin”
  分支未触发）。
- **已交付（本轮）**：`tools/harbor/pyproject.toml`（pin `harbor==0.23.0`）、
  `tools/harbor/chatspeed_agent.py`（`ChatSpeedAgent(BaseInstalledAgent)`：`install()` 只**校验**
  任务环境内已存在的 `chatspeed-headless`/`cs`（缺失即 trial 失败，不下载）、`run()` 写 frozen plan
  → 写 `harbor-task-capability.json`（0600、仅当前用户）→ 启动隔离 headless →
  用**显式 discovery** 执行 `cs doctor` 与 `experiment campaign schedule/jobs` 并把结构化
  campaign/job id 与状态写入 `AgentContext.metadata`）、
  `tools/harbor/artifact_contract.py`（声明式 allowlist `DECLARED_ARTIFACTS`、
  `capability_manifest()` 产出与 Rust `HarborTaskCapabilityV1` 同 schema 的文档、
  `publish_artifacts()` 只发布声明文件并记录 sha256、`verify_published_artifact()` 重算 hash +
  拒绝凭据标记）。
- **已在 pinned venv 实测**（`.cs/harbor-probe/venv`，harbor 0.23.0）：
  `ChatSpeedAgent` 可导入且 `__abstractmethods__` 为空（Harbor 可按 `module:Class` 加载）；
  capability 文档键 = `artifact_roots/issued_at/network_policy/nonce/owner_token_hash/
  read_only_roots/schema_version/task_id/task_root/workspace_root`；
  负向验证通过：hash 不符 → `ArtifactBoundaryError`，含 `sk-` 标记 → `ArtifactBoundaryError`。
- **U-9 仍缺**：Harbor smoke task fixture（`work/agent-cli-harbor-smoke/{task.toml,instruction.md,
  environment/,tests/}`）、separate verifier 与 5 类负向 fixtures（tamper / undeclared path /
  secret marker / missing capability / network mismatch）以及一次真实 Harbor trial（需 Docker 任务环境
  与模型，属 U-10）。

#### 11.6.6 本轮回归证据（最后一次改动之后）

`cargo fmt --all -- --check` 干净；`cargo test --lib experiment_owner` **38 passed**、
`--lib experiment` **138**、`--lib headless` **22**、`--lib workflow::react::client` **54**、
`cargo test --bin cs` **111**、`cargo test --bin chatspeed-headless` **2**，全部 0 failed；
三个 bin 编译通过（仅剩 `mcp/client/types.rs` 与 `db/chat.rs` 两处与本次改动无关的既有可见性 warning）。
真实 Docker owner gate（创建/容器内 exec/绑代 adopt/只删自有容器/同名异 label 永不被接管）
为 `experiment_owner::docker` 5 项测试，均未 skip。

### 11.7 2026-09-17：真实 scheduled smoke 通过 + 三个引擎级缺陷修复

> 本节取代 11.3/11.6.3 中“scheduled dispatch 仍 fail-closed”“U-8 只到 kernel 入口”的结论：
> 引擎级接线已完成并在真实 Docker + 真实模型下端到端跑通。指针**仍为 `2G+2H`**（原因见 11.7.4）。

#### 11.7.1 本轮修复的三个真实缺陷（均含新增回归测试）

1. **headless 调度器空转**（严重：idle 实例 ~120% CPU，实测 `db-writer` 46% + reader 各 17%）
   - 根因：`spawn_scheduler` 把 `sleep(poll_ms)` 与 `spawn_blocking(tick)` 放进同一个 `select!`；
     空 tick 微秒级返回，于是循环以约 1400 次/秒重跑 tick（诊断日志 `diag iterations=10000` 在 7 秒内），
     1 秒的间隔分支永远赢不了。
   - 修复：抽出 `run_scheduler_loop()`，**先 tick、后固定间隔**（间隔期间仍可被 shutdown 打断）；
     `MIN_SCHEDULER_POLL_MS` 显式声明下界。
   - 证据：idle CPU 从 480 ticks/4s（≈120%）降到 **3 ticks/4s**；
     新测试 `headless::bootstrap::tests::an_idle_supervisor_waits_between_ticks`（400ms 内 ≤8 tick 且 ≥2）。
2. **campaign scope 存在性判断反向**（严重：每个 scheduled job 都在写完 intent 后被 kernel 拒绝）
   - 根因：`get_budget_scope_status()` 返回 `Result<Option<_>>`，而 `ensure_campaign_scope` 用
     `.is_ok()` 判断，`Ok(None)`（不存在）被当成“已冻结”，于是从不创建 scope；
     kernel 随后以 `campaign ... not found` 拒绝，job 变成 `failed_precondition`，
     重启后又被 park 成 `unknown_manual`（真实观测：`error_code=dispatch_uncertain`）。
   - 修复：区分 `Ok(Some(_))`/`Ok(None)`；拒绝 durable campaign id 与 frozen plan 派生 id 不一致；
     并把 freeze **移到 `preflight_dispatch`（写 intent 之前）**，使缺失 agent / 不可创建 scope
     也变成干净的 pre-dispatch 失败。
   - 证据：`scheduler_runtime::tests::an_absent_campaign_scope_is_created_rather_than_assumed`、
     `a_frozen_campaign_scope_is_never_refrozen`。
3. **headless 没有自己的 ccproxy**（`cs@group@alias` 模型全部 401）
   - 根因：`cs@…` 模型走**进程内 loopback ccproxy**，其地址与内部 key 都是进程级全局
     （`CHAT_COMPLETION_PROXY` / `INTERNAL_CCPROXY_API_KEY`）。headless 从不启动该代理，
     请求落到同机**其它实例**的 11435 上（`/usr/bin/chatspeed`），对方用自己的 key 校验 → `401 invalid_api_key`。
   - 修复：按计划从 `http/server.rs` 抽出可复用 **`ccproxy::launcher`**（绑定重试、发布自身地址、
     优雅关闭），desktop 与 headless 共用；headless 启动时own 一个代理端口（`ccproxy_failed` 为新的 fail-closed code），
     并在 `shutdown()` 中关闭。**路由组合/顺序/鉴权/ModelResolver/header filtering 未改**（ccproxy constitution §4/§5/§6）。
   - 证据：真实请求不再 401，而是到达**本实例**代理并返回真实上游结果（见 11.7.2 的成功 run）。
4. **超出 tick 预算的 run 永不回收**（随修复 1 暴露）
   - 根因：`wait_for_terminal()` 超时后 job 保持 `running`；而 `claim_next_job` 只认领
     `not_dispatched` 的 `queued|preparing|prepared`，于是“下一个 tick 收集它”的日志是假的，
     job 永久停在 `running`（真实观测：日志每 tick 重复该行）。
   - 修复：`recover()` 增加**稳态收集**：`confirmed` + `run_id` + run 已终态时，用 `owner.adopt()`
     接管**同一 generation** 的环境、发布补丁、`cleanup_done`，再按 run verdict 落终态；
     **从不**重新 dispatch。同时区分**启动分类**与**稳态观测**（只有刚启动的进程才能把
     “无法证明未发生”判为 `unknown_manual`；稳态下不得 park 自己正在跑的 run）。
   - 证据：`scheduler::tests::a_run_that_outlives_its_tick_is_collected_by_a_later_tick`
     （真实场景复现：run 在 tick 后终态 → 下一个 tick 收集为 `succeeded` + `artifacts_collected` + `cleanup_done`，
     dispatch_count 不变）；`an_unprovable_post_dispatch_effect_is_parked_and_never_redispatched`
     增加“稳态不得 park 自有 live run”断言，重启语义改为**新建 scheduler**（真实重启语义）。
   - 另：cleanup/journal 顺序调整为“先 cleanup + 记 `cleanup_done`，后落终态”，
     这样带围栏的 journal 追加发生在 job 仍为 `collecting` 时（终态后追加会被围栏拒绝）；
      `bundle::release_job_staging()` 让后续 tick 也能清掉跨 tick 遗留的 staged bundle。

#### 11.7.2 真实 scheduled smoke（Docker owner + 真实模型，主机模式）

隔离域 `dev_data/2gh-smoke/domain`（独立 DB/布局/marker/lease），base repo 为临时 git 仓库，
execution profile `docker-smoke` 为 digest-pinned `persistent_docker`（`git:latest` 的 image id），
credential 走真实 `--api-key-file`，模型按用户指定顺序尝试：

| 顺序 | 模型 | 结果 | 性质 |
|---|---|---|---|
| 1 | `cs@qwen-3.8-flash` | alias 不存在（该域 `chat_completion_proxy` 只有 `qwen3.8-flash` / `free:qwen-3.8-flash`） | 配置缺失，确定性 |
| 2 | `cs@free:gemini-flash` | HTTP 503（上游 “high demand”） | 已知失败（非 unknown） |
| 3 | `cs@free:ds-v4-flash` | **succeeded** | 真实交付 |

成功 run 的结构化证据（`dev_data/2gh-smoke/evidence.txt` 已固化）：

```
campaign_id=camp-96b465c3bb771ed0f74f1ab0d86babf1  profile=docker-smoke  concurrency=1
job_id=job-df202b49610b223ef6e5fbb734a065bf  state=succeeded  dispatch_marker=confirmed
run_id=0rmsf5a1c0400  error_code=null  last_stage=cleanup_done
journal: workspace_acquired -> environment_ready -> dispatch_intent -> workflow_started
         -> artifacts_collected -> cleanup_done
snapshot: state=Running -> state=Completed（provider 真实返回）
artifact: artifacts/job-df…/patch.diff + patch-manifest.json
          （schema run_patch_manifest.v1；绑定 job/run/session/candidate/base_revision=HEAD；diff_sha256 已记录）
容器审计: 无遗留 `cs.owner_schema` 容器（saga cleanup 生效）
base repo: status 干净、只有一个 worktree、HEAD 未变（主树零改动）
```
（另有 pre-fix 遗留容器 `cs-run-job-6bc193a3…-g1`，正是修复 4 之前“超时 run 永不回收”的产物，
已按 label 确认 ownership 后手工清理。）

#### 11.7.3 本轮回归证据（最后一次改动之后）

`cargo fmt --all -- --check` 干净；三个 bin `cargo check` 通过（仅剩 `mcp/client/types.rs`
既有可见性 warning）。聚焦测试（全部 0 failed）：
`experiment_schedule` **80**（含 4 个本轮新增）、`experiment_owner` **38**（含真实 Docker gate 5 项）、
`headless` **27**、`campaign` **37**、`budget` **84**、`workflow::react::client::http` **33**、
`commands` **104**、`skills` **11**、`cargo test --bin cs` **111**。
两个**与本次改动无关的既有失败**：`ccproxy::adapter::backend::gemini::tests::supported_gemini_model_serializes_normalized_thinking_level`
（`xhigh` vs `high`）与 `tools::shell::tests::*`（本机 policy 环境的相对路径判定）；二者文件均未在本次改动集中。

#### 11.7.4 U-9/U-10 剩余阻塞（因此指针不推进）

- **U-9 已交付但未取得真实 trial**：`tools/harbor/{pyproject.toml,chatspeed_agent.py,artifact_contract.py}`、
  `work/agent-cli-harbor-smoke/{task.toml,instruction.md,tests/verify_artifacts.py}` 均已存在；
  pinned `harbor==0.23.0` 已实测 `BaseInstalledAgent` 可加载、契约与负向 fixture 自检通过。
  **真实任务环境被两个已实测的打包事实阻塞**（非架构问题）：
  1. 现成 `git:latest` 是 **Alpine/musl**，glibc 二进制无法运行；
  2. 换 Debian bookworm（glibc 2.36）后：`cs` 需要 **GLIBC_2.38**（本机 Ubuntu 24.04 / 2.39 编译），
     `chatspeed-headless` 还缺 **libgdk-3.so.0**（链接了整个 lib + Tauri）。
  即真实 Harbor 任务环境需要“glibc ≥2.38 且带 GTK3 的 base”；并且一次**真实模型** trial 还需要
  容器内 provider egress 与凭据输入（当前 `task.toml` 为 `no-network`）。
- **U-10 其余出口门禁**：crash/failure-injection matrix、一次 pre-dispatch 重启恢复、退出审计、
  以及重跑用户模型顺序后的完整记录仍待补（本轮只完成真实 Docker owner + 真实模型 scheduled smoke）。
- **desktop 侧（AC-8）**：`ccproxy::launcher` 为 desktop/headless 共用代码，编译通过；
  运行中的 `pnpm tauri dev` 实例已由 tauri 监视器自动重编译（实测运行二进制 mtime 00:51:43
  晚于 `launcher.rs`/`server.rs` 的 00:21），该实例控制面可连、`cs doctor` OK，并经新 launcher
  拥有自己的 ccproxy（127.0.0.1:11436）。此条更正 11.7.4 早前“尚未以新代码重启验证”的表述。

### 11.8 2026-09-17 晚：终审三项 required fixes 的落实

#### 11.8.1 AC-7 capability hand-off 修复（终审 blocker）

- **根因**：adapter 把 capability manifest 写到 `/installed-agent/harbor-task-capability.json`
  （domain 目录**外面**），而 runtime 读取 `<data-dir>/runtime/harbor-task-capability.json`；
  两边从不一致，导致任何 Harbor-owner job 都在 `preflight_dispatch` 以 `ownership_mismatch` 失败关闭。
- **修复**：
  - `tools/harbor/artifact_contract.py` 引入 `DOMAIN_ROOT`，并把 `CAPABILITY_FILE` 改为
    `DOMAIN_ROOT/runtime/harbor-task-capability.json`；同时固定 `DISCOVERY_FILE`，并提供
    `python3 artifact_contract.py {paths|emit <task_id> <nonce>}`（仅标准库），使路径契约可被外部断言。
  - `tools/harbor/chatspeed_agent.py` 不再自定义 `DOMAIN_ROOT`，改为从 contract 导入；
    写 manifest 前先把**自己拥有的 root**（`/installed-agent`、`/logs/artifacts`、
    domain）建好，并要求 task image 必须提供 `/workspace` 与 `/tests`——缺失即 trial 失败并点名 root，
    不再产出“声明了不存在 root”的 capability。
  - Rust 侧新增 `scheduler_runtime::harbor_capability_path()` 作为唯一路径来源（`DomainSchedulerResources` 使用它）。
  - 新增 `work/agent-cli-harbor-smoke/environment/Dockerfile`：显式声明 base 要求
    （**glibc ≥ 2.38 + libgtk-3-0**）并创建全部 declared roots（`/workspace`、`/tests`、
    `/installed-agent`、`/logs/artifacts`），把二进制 COPY 进镜像（不在任务内下载）。
- **证据（新增 2 个 focused test）**：
  - `the_harbor_adapter_writes_the_capability_this_runtime_reads`：运行**真实的 Python producer**
    （`paths` 与 `emit`），断言 adapter 的 `capability_file == domain_root/runtime/harbor-task-capability.json`、
    断言 `harbor_capability_path()` 解析同一相对布局、把 producer 输出的 manifest 写到该路径后
    `HarborTaskOwner::load` 成功且 token/task_id/nonce 一致。
  - `the_adapter_declares_the_roots_the_task_image_provides`：断言 adapter 声明的 roots
    与 task image Dockerfile 实际创建的 roots 一致。

#### 11.8.2 稳态 park 门控回归修复（本轮自查发现的真实缺陷）

- **根因**：11.7 引入的“只有刚启动的进程才 park”门控过于宽泛——重启分类每进程只跑一次，
  若该 job 因**已崩溃 generation 的 job lease 仍活跃**而被 `LeaseConflict` 推迟，之后的 tick
  永远不再重试，job 卡在 `dispatching/intent_recorded`。
- **修复**：门控按状态收敛为 `may_park_in_steady_state(state) = state != Running`：
  只有 `running` 可能是本 supervisor 自己的 live run（必须保留），而
  `dispatching/intent_recorded`（有 intent 无 confirmed run）这类崩溃产物在任何 sweep 都可 park。
- **证据**：新测试 `a_steady_state_sweep_parks_crash_artifacts_but_never_a_live_run`；
  以及 11.8.3 的真实进程矩阵 CASE C（修复前卡住，修复后正确落到 `unknown_manual`）。

#### 11.8.3 进程级 crash / restart 矩阵（U-10/V-3，真实进程）

`dev_data/2gh-smoke/crash-matrix.sh` → `dev_data/2gh-crash/matrix.txt`（真实 SIGKILL + 重启）：

| 用例 | kill 点（durable） | 重启后 | run kernel 次数 |
|---|---|---|---|
| A 预派发重启恢复 | SIGKILL 时 `queued/not_dispatched`（无 run） | **`succeeded/confirmed`**，journal 完整 6 阶段（`workspace_acquired → … → cleanup_done`），run `0rmsxwnwg0400` | 1（仅一次） |
| B kill 于准备/派发边界 | journal 已到 `dispatch_intent` | `unknown_manual/intent_recorded`，`dispatch_uncertain` | 0 |
| C kill 于 dispatch intent 之后 | `dispatching/intent_recorded`，无 run id | `unknown_manual/intent_recorded`，`dispatch_uncertain`，并补 `cleanup_done` | **0（未再次调用 run kernel）** |

- 顺带证实 fail-closed：崩溃 generation 的 domain lease 未过期时，重启实例以
  `experiment_domain_locked` 拒绝启动（矩阵脚本据此先等 lease 过期再重启）。
- 主动清理：崩溃 generation 遗留的 owner 容器与 worktree 由**人工按 label/path 校验后**删除
  （parked job 按设计永不被自动清理），记录在 `gate-evidence.txt`。

#### 11.8.4 退出残留 + 敏感内容审计（U-10/INV-6）

`dev_data/2gh-smoke/gate-audit.sh` → `dev_data/2gh-smoke/gate-evidence.txt`，覆盖 smoke 与 crash 两个 domain：

- **敏感面**：fixture instruction（`Reply with exactly: OK`）在 `experiment_campaign_schedules`/
  `experiment_campaign_jobs`/`experiment_job_journal` 中出现 **0 次**（只存 refs/digests）；
  journal detail、artifacts、instance log 中 `sk-/ghp_/xoxb-/-----BEGIN/AKIA` 命中 **0**。
- **残留面**：两个 domain 的 discovery 均已消失、domain lease 已释放、无本任务 headless 进程、
  无 owner 容器、base repo `status` 干净且 HEAD 未变（仅崩溃 generation 留下 2 个 worktree，
  已按 path 校验后清理）。

#### 11.8.5 本轮回归（最后一次改动之后）

`cargo fmt` 干净；`experiment_schedule` **81**、`headless` **29**、`experiment_owner` 38、
`campaign` 37、`budget` 84、`workflow::react::client::http` 33、`commands` 104、`skills` 11、
`cargo test --bin cs` 111 —— 全部 0 failed；两个既有失败仍与本次改动无关（gemini thinking-level、
`tools::shell` 本机 policy）。

#### 11.8.6 AC-7 真实 Harbor trial：已定位的 stop condition（待用户裁决）

adapter 与任务环境已修好并可在 pinned 环境加载，但**本机无法完成真实 trial**，两个实测事实：

1. **无法获得可承载二进制的 base 镜像**：本机可用的 glibc 基础镜像只有 Debian bookworm（glibc **2.36**）
   与 Alpine/musl；实测 `cs` 需要 **GLIBC_2.38**，`chatspeed-headless` 还需 **libgdk-3.so.0**（GTK3）。
   而 Docker Hub 在本机不可达（`docker build FROM ubuntu:24.04` → `auth.docker.io` i/o timeout），
   因此无法拉取 Ubuntu 24.04 / 其他满足要求的 base。
2. **真实模型 run 还需容器内 egress 与凭据输入**：当前 `task.toml` 的 agent 为 `no-network`，
   且凭据必须受控注入（INV-6），本阶段未设计该通道。

按计划 §5 的 stop condition 规则，这里**不降低验收**：指针仍为 `2G+2H`，并把该决策交用户裁决
（拉取所需 base 以完成真实 trial，或明确接受该缩减后的验证范围）。

#### 11.8.7 真实 Harbor trial 执行结果（用户已提供 `ubuntu:26.04` base）

用户在本机提供了 `ubuntu:26.04`（glibc 2.43），因此真实 Harbor 任务环境**已可构建并运行**：

- **任务镜像**（`work/agent-cli-harbor-smoke/environment/Dockerfile`）实测可用：
  `ubuntu:26.04` + `libgtk-3-0` + `libwebkit2gtk-4.1-0` + `python3` + git-initialized `/workspace`，
  四个 declared roots 全部存在，镜像内 `cs --version` / `chatspeed-headless --version` 均正常。
- **Harbor 0.23.0 `--install-only` 通过**：Harbor 按 `chatspeed_agent:ChatSpeedAgent`
  加载 custom installed-agent、provision 任务环境并执行 adapter 的 binary 校验（exit 0）。
- **完整 trial 已跑到生命周期末端**（`dev_data/2gh-harbor/harbor-trial.txt`、`jobs/2gh-trial/**`）：
  `install` → 写 plan → 写 config package（agents + 其依赖类别，仅 agent，无 secret）→
  写 capability manifest 到 `<data-dir>/runtime/harbor-task-capability.json` → 启动 headless
  （`--config-package … --config-category ai-models --config-category skills --config-category mcp
  --config-category sandbox --config-category agents`）→ `cs doctor` **通过** →
  `cs experiment campaign schedule --profile harbor-task` **被受理** →
  两个 job 均 `dispatch_marker=confirmed` 且在沙箱内启动真实 run（`run_id=0rmxnjmkc0400`、`0rmxnjnq40400`）→
  adapter 把 declared artifacts 写入 `/workspace`。
- **trial 未通过**（唯一的未满足项）：adapter 的有界等待结束时两个 job 仍为 `collecting`，
  因此 adapter 以“未成功”拒绝，separate verifier 未取得 reward。
  两个待办（均未在本轮完成）：
  1. **沙箱内模型访问**：domain 里没有 model/凭据（INV-6 不把 secret 放进沙箱），
     真实 run 无法成功；需要设计受控凭据/模型注入通道。
  2. **harbor_task owner 的 collection 终态**：观测到 job 长时间停在 `collecting`，
     需要确认 `HarborTaskOwner` 的 adopt/collect 在该环境下的行为（属实现缺陷排查）。
- 因此 AC-7 的“真实 trial 通过”仍未取得，指针保持 `2G+2H`；上述两项已作为精确 stop condition 记录。

#### 11.8.8 collection 停摆的真实根因（已修复）+ 沙箱模型通道接线

用户提供 `ubuntu:26.04` 与临时代理 token 后，继续把 trial 推到“run 在沙箱内真实调用模型”这一层：

1. **collection 停摆根因（已修复，真实缺陷）**：adapter 新增 runtime-log 诊断后，沙箱内 scheduler 每 tick 报
   `workspace_escape: the artifact destination '/installed-agent/chatspeed-domain/artifacts' is outside every root
   declared by the harbor capability`——即 scheduler 把补丁发布到 domain 自己的 `artifacts/`，而 Harbor capability
   只声明 `/logs/artifacts`，于是 `HarborTaskOwner::require_owned` 拒绝，job 永远停在 `collecting`。
   - 修复：`ExecutionOwner` 新增 `artifact_root()`（默认 `None`），`HarborTaskOwner` 返回 capability 声明的
     artifact root；`collect_dispatched` 用它作为补丁发布目标，只有未声明 root 的 owner 才回落到 scheduler 自己的
     artifact root。
   - 证据（真实 trial）：两个 job 现在都走到终态并发布补丁——
     `JobOutcome { state: Failed, run_id: Some(…), artifact_path: Some("job-…/patch.diff"), error_code: Some("run_failed") }`；
     本地回归 `experiment_owner` 38、`experiment_schedule` 81、`headless` 29 全绿，三 bin check 无 error。
2. **沙箱内模型访问已接线**：adapter 现在把“模型 + agent”通过**权限受限的 config package**（0600，位于 task root，
   绝不进 artifact）注入 domain，token/endpoint 由 job 环境变量提供（`CHATSPEED_SMOKE_MODEL_BASE_URL/__TOKEN`），
   并用 `--extra-docker-compose` 让沙箱共享 host 网络以访问宿主桌面实例的 ccproxy。
   - 证据：run 已真实抵达该端点并收到**上游语义的错误**（先 `budget_exceeded` → 调大 fixture caps 后变为
     `404 Model Not Found: 模型别名 'ds-v4-flash' 未找到` / 空 details 的 404），说明 egress、鉴权与 domain 内 AI
     客户端链路均已打通；**剩余仅为 smoke 侧的 proxy 路径形状**（客户端拼接的 URL 与该 ccproxy 的
     `/<group>/v1/chat/completions` 路由尚未对齐），属一行级别的 harness 接线。
3. 因此 AC-7 的“真实 trial 通过 + verifier reward”仍差最后一步（路径形状），指针保持 `2G+2H`。

#### 11.8.9 AC-7 真实 Harbor trial **通过**（2026-09-17）

在 11.8.8 的基础上补齐最后三处（均为 smoke/harness 接线，非产品缺陷），trial 现已端到端通过：

1. **沙箱内模型 URL 形状**：runtime log 显示客户端实际请求 `…/cs/chat/completions`（§11.8.8 只看到 404 症状）；
   `curl` 实测 host ccproxy 的可用形状为 `http://127.0.0.1:11436/cs/v1/chat/completions`（200，真实补全），
   故把 job 环境里的 `CHATSPEED_SMOKE_MODEL_BASE_URL` 设为 `http://127.0.0.1:11436/cs/v1`。
2. **separate verifier 的入口必须由镜像提供**：Harbor 约定 verifier 运行**镜像自带**的 `/tests/test.sh`；
   任务镜像构建上下文改为 task 目录，并在 Dockerfile 中 `COPY tests/test.sh`、`COPY tests/verify_artifacts.py` 到 `/tests`。
3. **verifier 只拿得到 declared artifact 根**：adapter 的 manifest 写在宿主侧，verifier 环境看不到，
   故 verifier 增加“直接校验已收集 artifacts”的模式（allowlist + 无凭据标记 + campaign schema +
   **每个 job 的终态必须为 `succeeded`**），hash 模式仍保留并继续用于负向自检。

**通过证据**（`dev_data/2gh-harbor/harbor-trial.txt`、`jobs/2gh-trial/**`）：

```
Harbor: Trials 1 / Mean 1.000 / Exceptions 0
verifier reward=1  → "verified 2 declared artifact(s); all 2 job(s) reached 'succeeded'"
artifact collection: ok  /logs/artifacts
  chatspeed-campaign.json (341B)  chatspeed-jobs.jsonl (1588B)
  job-2b49e2b8…/patch.diff + patch-manifest.json
  job-e03434a7…/patch.diff + patch-manifest.json
durable rows: job-2b49e2b8… candidate=baseline state=succeeded marker=confirmed run=0rmyapb1w0400 profile=harbor-task
              job-e03434a7… candidate=cand-a  state=succeeded marker=confirmed run=0rmyapzj40400 profile=harbor-task
```

即：Harbor 0.23.0 以 `module:Class` 加载 custom installed-agent → 在真实任务环境中校验 binaries →
写 capability（正确路径）→ 启动 isolated headless（config package 供 agent+模型，0600，无 secret 进 artifact）→
`cs doctor` → `cs schedule --profile harbor-task` 受理 → **两个 run 在沙箱内真实调用模型并 `succeeded`** →
owner 把补丁发布到 capability 声明的 `/logs/artifacts` → 声明 artifacts 被 Harbor 收集 →
**fresh verifier 复算并给出 reward=1**。

因此 AC-7 的“真实 trial 通过 + separate verifier”已取得；AC-9 的 required gates 至此齐备。

#### 11.8.10 凭据通道修复（INV-6，终审 blocker）与审计覆盖扩展

- **根因**：adapter 把模型凭据写进被 exec 的命令（`printf '%s\n' '<package with api_key>' > chatspeed-config-package.json`），
  而 Harbor **逐字记录每条执行过的命令**，于是操作者 token 以明文落在
  `jobs/2gh-trial/job.log` 与 `trial.log`（INV-6 / D-6“明文 secret 不进 argv/log/artifact”被违反）。
- **修复**：`_write_config_package` 改为**文件通道**——在宿主侧临时目录生成 0600 的 package，
  通过 `BaseEnvironment::upload_file()`（docker cp / mounted-env copy，不发生命令记录）送入 `/installed-agent/`，
  随后只执行不含秘密的 `chmod 0600` + `chown root:root`。capability manifest / execution profile /
  declared artifacts 仍走命令写入，但它们**不含任何凭据**（capability 只带 `owner_token_hash` 摘要）。
- **审计扩展**：`dev_data/2gh-smoke/gate-audit.sh` 新增 Harbor 通道段落——扫描 job.log/trial.log/agent log/artifacts
  的 marker 与（从环境读取、从不回显的）操作者 token；并排除 `harbor-task-capability.json` 文件名造成的 `sk-` 误报。
- **复验证据（修复后重跑真实 trial）**：
  `Mean 1.000 / Exceptions 0`、`reward=1`、verdict `verified 2 declared artifact(s); all 2 job(s) reached 'succeeded'`；
  Harbor 通道 24 个文件扫描结果：**markers=0、token occurrences=0**（修复前该 token 出现在 job.log/trial.log）。
  由于 `run-trial.sh` 每轮清空 jobs 目录，旧一轮含 token 的日志不再保留。
- **扫描器与阳性对照（针对终审 info：审计方法可核查性）**：扫描逻辑抽为
  `dev_data/2gh-smoke/scan_secrets.py`（唯一实现，审计与自检共用；token 从
  `CHATSPEED_SMOKE_MODEL_TOKEN` 读取且从不回显），并新增
  `dev_data/2gh-smoke/audit-selftest.sh` 作为**阳性对照**：
  `detection OK (token=1 markers=1)`（植入凭据能被发现）、
  `false-positive OK`（仅出现 capability 文件名时 `token=0 markers=0`）、
  `missing-token OK`（未提供 token 时扫描器拒绝输出“干净”结果）。
  因此审计的 `token=0` 是**经过检测能力验证**的结论，而不是未经验证的 grep。
  诚实说明：修复前那轮含 token 的日志已被 `run-trial.sh` 的每轮清理删除，
  故“修复前/后”无法再从磁盘复现，只能用阳性对照证明扫描器确实能发现同类泄漏；
  该审计为 operator 运行（需在环境中提供 token），未提供时明确记录 SKIPPED。
- **唯一命中位置（已核查并说明）**：全仓 `grep -rlF <token>` 只命中操作者自己的桌面实例库
  `dev_data/chatspeed.db`（mode 0600）的 `config.chat_completion_proxy_keys`——那是**操作者在 UI 里
  自行创建的 ccproxy 代理密钥**，属于产品既有的凭据存储，不是本任务的 queue/journal/log/artifact/Harbor
  通道；本任务的 smoke 脚本只从该库读取 `agents`，无任何任务路径写入它。因此审计把该库显式排除并写明理由，
  其余任务产出路径（`src`、`src-tauri/src`、`tools`、`work`、`dev_data/2gh-*` 与两个实验 domain）
  扫描结果为 **hits=0**。

#### 11.8.11 2G+2H owner-bound capability audit remediation（2026-09-17，as built）

本节是对既有 Implementation Record 的**追加勘误与验证记录**，不改写 11.1–11.8.10 的历史事实。它关闭
2I 进入前针对 job-scoped verified bundle MCP/skill 生命周期、Docker owner 边界、失败清理和 secret log 的
剩余审计项。

- **Docker owner-bound MCP stdio**：scheduler 在 owner acquire 前建立 server-derived job bundle root；
  `PersistentDockerOwner` 仅接受 profile 声明的单一只读 bundle bind mount，并将其记录在 fenced
  `ContainerHandle`。持久 Docker owner 的 bundle MCP 被投影为
  `docker exec -i -w <container-bundle-dir> [-e NAME] cs-run-<job>-g<generation> <program>`；程序、cwd、
  文件系统与既有 network policy 都在已 label-fenced owner container 内。仅 env **变量名**进入 docker argv，
  resolved secret value 继续只存在 session-local `McpServerConfig.env` 内存中。无 bundle 的 Docker job
  不创建/挂载 bundle root。
- **Harbor 与 desktop/CLI 不变**：Harbor target 保持原 verified stdio command/args，因当前进程已在
  Harbor task sandbox 内；普通 run 仍无 capability target，global tool manager、用户 MCP 配置和 desktop/CLI
  MCP fallback 均未改变。
- **session capability lifecycle**：所有已验证 bundle lease 聚合为 `PreparedCapabilityLeaseSet`；重复 MCP
  server name fail closed。MCP 仅注册到 executor 的 session-local manager，解析、dispatch、approval replay
  和事件 metadata 均采用 session-local 优先、global fallback。global MCP refresh 不再清除/重启 owner-scoped
  wrapper；verified owner server 仅在其由 session manager 解析到时可越过持久 global MCP allowlist，绝不放宽
  同名 global server。skills 以 `RegisteredSkill.bundle_root` 注册并在 release 时对称移除。
- **failure/recovery 与脱敏**：acquire、bundle verify、preflight 和 dispatch-refusal 的未派发 rollback 均
  best-effort release job staging 与 owner；已确认终态的 collection 失败也进行该清理。终态但不可 adopt 的
  recovery 只删除有界 `<bundles_root>/<job_id>` staging 并 park `unknown_manual`，不对未获 owner proof 的
  容器猜测性 teardown。`ToolManager::register_mcp_server` debug logging 不再输出完整 config，避免 resolved
  `env` secret 出现在日志。
- **本轮验证（最后一次修改后）**：`cargo fmt --all`、`cargo check --lib`；
  `cargo test --lib experiment_owner`（40 passed）、`cargo test --lib experiment_schedule`（82 passed）、
  `cargo test --lib owner_execution_context_tests`（4 passed）；新增 owner capability allowlist、Docker/Harbor
  config 投影、lease collision、terminal unadoptable staging cleanup 单测均通过。真实 Docker gate
  `a_verified_bundle_mcp_executes_inside_the_owner_container` 通过：实际创建 fenced container，
  `docker exec -i` 在容器内运行 bundle program，验证 container cwd、`-e NAME` secret forwarding 和只读
  bundle mount。编译/测试仅保留与本轮无关的既有 `private_bounds`（及测试态 db visibility）warning。

**阶段结论**：2G+2H 的 owner-bound capability audit remediation 已完成；无 pending remediation，`## 0` 的
**下一个入口 2I** 维持有效。2I 的 promotion/apply 范围未在本轮实现。

## 12. 2I Active Plan（promotion / apply / 受控灰度闭环，当前执行范围）

> 本节是 2I 的**当前 active plan**，只登记本轮实现的契约、边界与验证口径，不改写 `## 9` 与 `## 11`
> 的任何历史 Implementation Record。`## 0` 的指针**只有在本节所有 Acceptance Criteria、Protected
> Invariants、Execution Units 与 Verification 均有真实证据后才允许推进**。

### 12.1 目标与交付物

把 durable campaign 中隔离 owner 产出的代码 `patch.diff` 作为**代码候选**，经过独立 verdict 事实绑定、
server-owned 自动 policy、隔离 checkpoint apply、分阶段 paired canary、实验分支本地 commit，形成
`verdict → apply → controlled canary → audit` 闭环。

目标命令形态：

```text
cs experiment promotion run \
  --campaign-id <durable-campaign> \
  --candidate <candidate-key> \
  --target <server-registered-target-ref> \
  --out <audit-dir>
```

CLI 自动定位同一 campaign 的 baseline/candidate jobs，复用并重验 2A/2D/2E evidence，提交严格 projection；
backend 交叉绑定 durable job/fixture/patch、执行自动 policy、创建 checkpoint commit、依序跑 paired canary，
最后 CAS 推进实验分支或收敛为 `rejected | canary_failed | rolled_back | unknown_manual`。CLI 等待终态并导出
可离线重验的 audit bundle。

### 12.2 范围边界与非目标

**本阶段明确不做**：

- 不修改 Agent defaults、候选 prompt catalog、产品主分支或用户当前工作树/index；
- 不执行 `git push/fetch/pull`、remote 管理、merge/rebase/cherry-pick，也不自动快进其他分支；
- 不改变 2A artifact、2E verdict 或既有 campaign/schedule schema（v20 纯 additive）；
- 不允许候选控制 verifier、policy、target、secret、sandbox 或 budget ledger；
- 不实现 GEPA/DGM proposer、远端发布或生产流量控制；
- 本阶段“灰度”定义为对同一 immutable checkpoint 逐级执行 backend 注册的 deterministic paired canary
  gates，**不是生产流量分配**；首版 canary 为 no-network、无 LLM/tool effect。

### 12.3 验收契约（AC）与不变量（INV）

- **AC-1**：路线文件新增 2I active plan（本节），历史 record 不改写。
- **AC-2**：strict versioned contracts 完整绑定 campaign/plan/schedule/candidate/job/run/session、artifact
  chain、verdict/verifier/fixture、budget、patch/base、profile、target/policy/canary digests；缺失、漂移、
  篡改或 caller/candidate 注入 trust 字段均在 effect 前拒绝。
- **AC-3**：v20 store/FSM 是 promotion 唯一 authority，具备幂等、单 target 单飞、lease/fence、事务 CAS、
  journal 和 restart reconcile；CLI 不直接访问 DB、owner 或 Git。
- **AC-4**：server-owned 自动 policy 以 safety/provenance/budget/infra 为 hard gates，再比较 baseline/candidate
  与 canary structured metrics；无改善、样本不足、unknown/partial/cost unknown、预算拒绝或关键回归不晋级。
- **AC-5**：在 server-derived detached worktree 应用 digest-bound patch，创建**英语** local checkpoint commit
  和 immutable ref；绝不调用 remote Git，也不修改 base worktree/index/HEAD。
- **AC-6**：old HEAD 与 checkpoint 在相同 digest-pinned profile、verified bundle、no-network/resource/time caps
  下跑有序 paired stages；只接受 strict structured result，失败立即停止且 branch 不动。
- **AC-7**：通过后仅以 expected-old CAS 更新注册实验 branch；并发移动拒绝；崩溃按 old/new/third-value ref
  observation 收敛；历史 commit/journal 不删除或覆盖。
- **AC-8**：CLI 提供 `run/status/reconcile/audit/inspect`；mutation 使用 bearer + Idempotency-Key；run 自动导出
  atomic audit sidecar，可离线重建全链且无敏感数据。
- **AC-9**：聚焦测试和真实 local Git smoke 证明连续成功节点形成线性 commits、失败节点不推进 branch、
  无 remote effect；按真实证据回写 Implementation Record 和 smoke 文档。

不变量（INV-1..INV-10）与本节 AC 一一对应，重申其中四条最容易在实现中走样的：

- **INV-2**：backend MainStore/promotion scheduler/promotion owner 是唯一 authority；CLI 不直开 SQLite、
  不启动 runtime/scheduler、不执行 Git mutation。
- **INV-3**：2E verdict 继续 facts-only；promotion policy 独立，candidate/model 文本与 program 自报
  `status` 都不能满足 gate（runner 必须从配对数字重算）。
- **INV-5**：Git effect 只允许 promotion worktree、`refs/chatspeed/checkpoints/*` 与注册的本地实验 branch；
  绝无 push/remote effect。
- **INV-7**：每次 effect 前先写 durable intent；无法证明的 effect 进入 `unknown_manual`，绝不盲目重试。

### 12.4 关键设计（server-owned target 与 FSM）

- **target 由 server registry 提供**：experiment domain 下的 `promotion-targets/<target_ref>.json`
  （`promotion_target.v1`）声明 base repo ref、full `refs/heads/*` 实验分支、command-level Git identity、
  canary profile/bundle/executable/stages 与 policy。caller 只提交 opaque `target_ref`，不提供路径、branch、
  命令、阈值或 secret。
- **FSM**：

  ```text
  queued → evidence_validating → rejected
                               ↘ checkpointing → checkpointed
                                                  → canary_running → canary_failed
                                                                   → ready_to_advance
                                                                      → advancing → promoted
  ```

  异常终态：`rolled_back | unknown_manual`。每个 effect 前有 durable intent：
  checkpoint intent → ref 不存在且可证明未发生时重试、存在且 trailers/bindings 一致时 adopt、不一致
  `unknown_manual`；branch intent → target=old 重试 CAS、target=checkpoint roll-forward journal、
  第三值绝不覆盖。
- **checkpoint commit 契约**（U-4）：
  - subject：`experiment(promotion): checkpoint <promotion_id>`（英语）
  - trailers：`Promotion-Id`、`Evidence-Hash`、`Patch-Sha256`、`Base-Revision`、`Target-Ref`
  - ref：`refs/chatspeed/checkpoints/<promotion_id>`
  - 不写用户 Git config，不 checkout target branch，永不调用 remote/merge/rebase/cherry-pick。
- **canary**：从 verified read-only bundle 解析相对 executable，在 digest-pinned/no-network/container limits 下
  对 old/checkpoint 运行相同 stages；显式 argv、无 shell、无 candidate-owned verifier；输出必须是单一 strict
  `canary_result.v1`，原始输出只做 hash 和 bounded diagnostics。
- **失败语义**：canary 失败发生在 advance 前，branch 自然保持 old，checkpoint 仍保留（“每个进步节点保存成果”）。

### 12.5 执行单元与验证映射

| 单元 | 内容 | Covers | 验证 |
|---|---|---|---|
| U-1 | 冻结 2I active plan、strict promotion contracts/FSM/hash domains 与 server-owned target registry | AC-1/2/4/5/6 | V-1 |
| U-2 | v20 纯 additive promotion tables、事务性 store、lease/fenced CAS、幂等提交、canary result 与 append-only journal | AC-3/7/8 | V-2/V-6 |
| U-3 | 桥接 durable baseline/candidate jobs、2A/2D/2E verified evidence、immutable patch manifest 与自动 policy | AC-2/4/8 | V-1/V-3 |
| U-4 | 独立 `PromotionCheckpointOwner`：detached worktree、local checkpoint commit/ref、`update-ref` CAS | AC-5/7 | V-4/V-6/V-8 |
| U-5 | paired staged canary runner（digest-pinned、verified bundle、no-network、严格结构化结果） | AC-4/6/7 | V-3/V-5/V-8 |
| U-6 | bounded promotion supervisor 与崩溃恢复（intent-first、fence、ref observation） | AC-3/6/7 | V-2/V-5/V-6 |
| U-7 | additive promotion HTTP routes 与 CLI `run/status/reconcile/audit/inspect` 及 audit sidecar | AC-8 | V-7/V-8 |
| U-8 | 聚焦回归、真实 local Git/容器 smoke、SIGKILL 恢复矩阵、secret/no-remote 审计与文档回写 | 全部 | V-1..V-9 |

### 12.6 Stop conditions（命中即停止并请求用户确认）

- 需要 push/remote/自动合并主分支；
- 需要修改 2A/2E schema 或放宽旧 promotion denylist；
- 需要让 caller/candidate 提交 target path/branch/command/policy；
- 需要 network/LLM canary 却没有新的 admission/recovery 设计；
- 需要新依赖、写用户 Git config、改普通 workflow/Tauri/ccproxy；
- 需要保存 transcript/private holdout/secret；
- 需要 destructive reset 覆盖未证明 ownership 的 ref/worktree。

### 12.7 完成回写规则

2I 完成时在本文件 `## 9` 末尾**追加** `### 2I Implementation Record (as built)`，并把 `## 0` 的指针推进；
在 `work/agent-cli-phase-2-smoke-test.md` 追加 2I smoke 记录。历史 11.x 与 `## 9` 既有段落一律不改写。

### 12.8 2I Implementation Record (as built，2026-09-17)

**本轮交付**：U-1..U-8 全部完成，`verdict → 自动 policy → checkpoint → paired canary → 分支 CAS → 审计`
闭环可用，且所有 Git effect 限于 promotion worktree、`refs/chatspeed/checkpoints/*` 与注册的本地实验分支。

- **U-1（V-1）**：`experiment_promotion/{mod,types,policy,smoke}.rs` + `headless/promotion_targets.rs`。
  strict `promotion_evidence.v1` / `promotion_request.v1` / `promotion_target.v1` / `promotion_policy.v1` /
  `canary_result.v1` / `canary_arm_sample.v1` / `promotion_projection.v1` / `promotion_reconcile.v1` /
  `promotion_audit.v1`；12 态 FSM + 纯 recovery classifier；server-owned target registry
  （full `refs/heads/*`、bundle-relative executable、profile/bundle 交叉授权、digest-pinned 镜像）。
  旧 2F/2G promotion denylist 未放宽。
- **U-2（V-2）**：纯 additive v20（promotions / promotion_journal / promotion_canary_results +
  “每 target 至多一个非终态” partial unique index）；事务 store：幂等 submit、单 target 单飞、
  lease generation fence、事务 CAS、intent-first checkpoint/advance、append-only journal（含顺序 digest）。
- **U-3（V-3）**：`binding.rs` 对 durable campaign/job、fixture、profile、run/session、2A artifact 行、
  immutable patch manifest 逐字段交叉绑定 + `verify_artifact_file` 逐字节复验；
  `ExperimentScheduleStore::job_artifacts` 暴露 scheduler 自己记录的 artifact 行。
  如实边界：操作者本地的 2D/2E sidecar 链无法由 backend 重放，其 digest 作为 adapter 事实
  （结构严格校验 + 其挂靠的 durable 半边全量交叉验证），已写入 `binding.rs` 文档。
- **U-4（V-4）**：`PromotionCheckpointOwner`：argv allowlist（push/fetch/pull/remote/merge/rebase/
  cherry-pick/reset/clone 等 spawn 前拒绝）、硬化子进程环境、detached worktree、`git apply --check`/apply、
  固定 identity 的英文 checkpoint commit + trailers、`refs/chatspeed/checkpoints/<id>`（空 old-value 防并发覆盖）、
  仅 expected-old CAS、checkpoint→old 补偿、cleanup 保留 checkpoint；普通 `ExecutionOwner` 未获得任何
  分支变更能力。
- **U-5（V-5）**：`promotion_canary.rs` + docker `exec_capture_bounded`（wall-clock + 输出上限）。
  canary program 每次只报**单臂单 stage** 的严格 `canary_arm_sample.v1`（无 status 字段，
  程序无法自报通过），runner 装配配对结果并用与 policy 共享的 `canary_stage_passes` 规则重算；
  非 `none` 网络策略直接 `canary_effect_forbidden`；失败/超时/越界立即停止、双臂资源清理。
- **U-6（V-6 部分）**：`experiment_promotion/scheduler.rs` supervisor + `headless/promotion_runtime.rs`
  + `bootstrap.rs` 接线（bounded tick、shutdown-aware loop）；claim→gate→checkpoint→canary→CAS，
  intent-first，恢复按 checkpoint/branch 观察分类（absent 重做 / 一致 adopt / 不一致或 third-value park）。
- **U-7（V-7）**：application facade + additive HTTP `POST /control/v1/promotions`（bearer+Idempotency-Key）、
  `GET /promotions/{id}`、`POST /promotions/{id}/reconcile`、`GET /promotions/{id}/audit`；
  CLI `cs experiment promotion run/status/reconcile/audit/inspect`（run 等待终态并原子导出 audit sidecar，
  inspect 全离线验证 manifest + integrity + journal digest）。
- **U-8（V-8/V-9）**：真实 local Git + 真实容器 canary 的端到端 smoke
  （`experiment_promotion/smoke.rs`，详见 `work/agent-cli-phase-2-smoke-test.md` `## 8`）：
  连续两个成功节点线性推进分支、失败节点分支不动且 checkpoint 保留、重启后 checkpoint 恰好一次、
  无 remote effect、secret 扫描阳性对照有效。

**验证汇总（全部通过）**：`cargo test --lib experiment_promotion`(41) / `experiment_owner`(51) /
`db::experiment_promotion`(8) / `db::sql::migrations`(15) / `db::experiment_schedule`(15) / `headless`(34) /
`workflow::react::campaign`(20)；`cargo fmt --all -- --check`；三 binary check 0 error。

**未执行项（如实说明）**：桌面端 `pnpm tauri dev` + 真实模型全链路未跑——
2I 不新增 LLM effect（INV-9），模型链路属 2B/2C/2F 既有范围；操作者本地 2D/2E sidecar 链无法由 backend
重放（见 U-3 边界说明）。

**终审修复（2026-09-17，全部落实并验证）**：

- **canary 非成功结果全部收敛终态**：`converge_canary_failure` 把门禁类失败（stage 回退/超时/
  结果不可信——malformed/oversize/不匹配）持久化为 `canary_failed`（`canary_stage_failed` /
  `canary_result_invalid`），环境类失败 park `unknown_manual`；不再出现停在 `canary_running`
  被反复重试的情况。回归：垃圾输出 → `canary_failed` 且分支不动、终态不再 re-claim；
  本地不存在的镜像 digest → park `executor_unavailable`。
- **CLI `run` 等待预算**：`minimum_wait_budget_secs` = 8 stages × 900s × 2 arms + 300s = 14,700s
  （覆盖服务器允许的 canary 上界；原 900s 固定预算会在合法运行中提前放弃）；override 只能上调；
  `wait_for_terminal` 抽出并可脱离 HTTP 测试（4 项 CLI tests）。
- **真实 SIGKILL/restart 矩阵**（`experiment_promotion/sigkill.rs`，真实 headless 子进程 +
  真实 SIGKILL）：checkpoint 边界 SIGKILL → 重启收敛且 checkpoint 恰一次（`rev-list --count == 1`，
  journal 各 stage 恰一条）；canary 中途 SIGKILL → 重启侧 `cleanup_stale_arms` 回收死亡尝试的
  容器/worktree 后再跑一次并恰一次推进；branch 边界三态（roll-forward / 恰一次 CAS / 第三值 park
  `unknown_manual`）。配套修复：重启侧 stale arm/checkpoint worktree 回收（ownership 可证明的命名），
  `CHATSPEED_PROMOTION_LEASE_MS` 运维变量，`DOCKER_GATE` 串行化容器测试，
  scenario patch 加盐避免跨场景 promotion id 冲突。

**指针推进**：Phase 2 的 2A–2I 全部完成，`## 0` 指针由“下一个入口 2I”推进为“Phase 2 完成”。

