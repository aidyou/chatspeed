# cs CLI 第二期 2A 冒烟测试记录（Artifact / Provenance + offline inspect/replay）

本文件记录 2A 的验证证据。2026-09-14 已实际启动 `pnpm tauri dev` 的 Tauri
桌面主进程，并通过 `cs` CLI 完成真实 desktop-owned 固定模型 smoke。2A 不引入
headless owner（属 2H），desktop 主进程仍是唯一 capture 来源。

## 1. 环境与工作区状态

- 平台：Linux（sandbox），无 `DISPLAY`/`WAYLAND_DISPLAY`；`pnpm` 不在沙箱 PATH。
- 无运行中的 ChatSpeed 主进程：`cs doctor` 连接 `127.0.0.1:40513` 返回 transport 失败（discovery 陈旧/主进程未运行）。
- 工作区（`git status --short`）仅包含 2A 变更与既有需保留改动：
  - `M src-tauri/src/bin/cs.rs`
  - `M src-tauri/src/bin/cs/args.rs`
  - `M src-tauri/src/workflow/react/client/http/server.rs`（既有未提交的 `EnvGuard::_lock` 测试辅助修改，已保留）
  - `?? src-tauri/src/bin/cs/artifact.rs`
  - `?? src-tauri/src/bin/cs/experiment.rs`
  - `?? work/agent-cli-phase-2-implementation-plan.md`
  - `?? work/agent-cli-phase-2-smoke-test.md`

> 更新：上述变更已按用户要求提交为两个独立 commit——`615962ef`（2A 功能 + 两份文档）与
> `57d30351`（既有 `server.rs` `_lock` 测试辅助修复）；提交后工作区干净。

## 2. 已执行的确定性验证（offline，可复现）

### 2.1 格式与双 binary 编译（无 warning）

```text
cargo fmt --all -- --check            # FMT_CLEAN
cargo check --bin chatspeed --bin cs  # Finished，无 warning
```

### 2.2 artifact/CLI focused tests（V-2/V-3）

```text
cargo test --bin cs   # 51 passed; 0 failed
```

覆盖：
- canonical JSON key 顺序无关、数组顺序保留、domain 分离哈希确定性；
- 递归脱敏：secret key→marker、free text→{type,len,sha256}、path→{type:path,sha256}、
  安全标识符/数字/布尔保留、非 secret key 下的 secret 值（`sk-`/`Bearer `）→marker；
- complete 往返：capture→write→verify 得 `artifact_status=complete`、`cost_status=known`、
  `correctness_status=not_evaluated`、`promotion_status=not_applicable`；
- 非终态 → `incomplete`；无 usage summary → `cost_status=unknown`；`unpriced_tokens>0` → `unknown`；
- 篡改负向：改 event payload 字节→`hash_mismatch`；删/重排事件（刷新 manifest 后）→`chain_break`；
  改 manifest→`manifest_mismatch`；删文件→`missing_file`；未来 schema_version→`incompatible_schema`；
  混入其他 session 事件→capture 阶段 `session_mismatch`；complete 但缺终端事件→拒绝；
  目标已存在→`target_exists`；symlink 目标→拒绝；
- manifest 固定集加固：移除任一必需文件条目 / 绝对路径 / `..` 穿越 / 重复条目 → `manifest_mismatch`
  （即便重算 `manifest_hash` 也拒绝），保证必需文件无法被移出完整性覆盖；
- 终端状态映射：durable `workflow_failed`→`failed`、`workflow_cancelled`→`cancelled`；
  stale `completed` snapshot 但无终端 durable 事件 → `incomplete`（不折叠为 completed）；
- 状态交叉校验（fail closed）：terminal/artifact 状态由**已验证事件链**派生，`verify_bundle_dir` 严格解析并
  交叉校验 `run.json`/`result.json` 声明——run 与 result 不一致、声明与事件链派生值不一致（如把 failed 改称
  completed）、缺失/非法 `artifact_status`、非法 `cost_status` 均返回 `invalid_artifact` 且非零退出，不返回 Ok；
- 隐私扫描：`run.json`/`snapshot.json`/`events.jsonl` 均不含原始 prompt、message 正文、
  tool 结果原文、`sk-...`、`Bearer ...`、绝对 workspace 路径。

### 2.3 一期回归（V-4）

```text
cargo test --lib workflow::react::client   # 36 passed; 0 failed（含 server.rs _lock 相关测试）
pnpm test:workflow                          # 62 passed; 0 failed
```

一期 HTTP/SSE control plane 与前端 workflow 契约无回归；`EnvGuard::_lock` 未提交修改保留且编译通过。

### 2.4 真实 CLI 离线进程验证（V-3）

```text
cs experiment --help
# 列出 capture / inspect / replay 子命令

cs experiment inspect /tmp/cs-no-such-artifact --output json
# stdout: {"artifact_status":"invalid","code":"missing_file",
#          "correctness_status":"not_evaluated","promotion_status":"not_applicable", ...}
# stderr: cs: missing_file: missing file run.json
# exit=1
```

证明 `inspect` 在无主进程/无 discovery/无网络环境下运行、fail closed、返回稳定 machine code 与非零退出。

### 2.5 capture 只读性（AC-6/INV-3 结构证据）

`experiment.rs` 的 `capture` 仅调用 `client.get(...)`（`/control/v1/meta`、
`/control/v1/workflows/{id}`、`/control/v1/workflows/{id}/events`），无任何 `post`/`stream`/
signal/stop；`artifact.rs`/`experiment.rs` 不 import `MainStore`/SQLite/`WorkflowManager`/executor/
`crate::workflow`/`crate::db`/`crate::commands`（INV-2）。

## 3. 真实 desktop 固定模型 smoke（V-5）

**已执行环境与结果**：`pnpm tauri dev` 启动后，control plane 监听
`127.0.0.1:39361`（protocol v1，instance `5ac23af75d8809a83e0c0bebd15ab3b0`），
`cs doctor` 认证、连通性和协议检查均通过。使用 `builtin:coding`、模型
`cs@free:ds-v4-flash` 和短 prompt `Reply with exactly: OK` 运行的 session
`0rm2f4v4r0400` 终态为 `completed`。

- `cs experiment capture` 成功产出 complete artifact，含 8 条 durable events。
- 对不存在的 discovery 文件显式调用 `experiment inspect` 和 `experiment replay` 均成功，
  证明两个命令在此路径不依赖 discovery 或已运行 desktop。
- capture 前后 `workflow get` 的 JSON 字节一致（`SNAPSHOT_UNCHANGED`）。
- 对 `run.json`、`snapshot.json`、`events.jsonl` 的 prompt/Bearer/API key/workspace 路径扫描无命中。
- artifact usage 含 legacy breakdown 与 unpriced tokens，离线 inspect 投影 `cost_status=unknown`，
  未把未知成本降为零。
- 普通 shell 无法复制 host CLI 生成的真实 artifact（隔离临时文件系统只读挂载），所以手工篡改该真实
  副本未执行；`cargo test --bin cs` 覆盖 event/hash/chain 篡改和刷新 manifest 后 result usage 篡改的
  fail-closed 路径。

**可复现脚本**（模型固定 `cs@free:ds-v4-flash`，短任务）：

```bash
# 1) 启动主进程（后台），等待 discovery 就绪
cd <repo>/src-tauri && pnpm tauri dev &
# 等待 ${CHATSPEED_HOME:-~/.chatspeed}/runtime/control-plane-v1.json 出现且可连
cargo run -q --bin cs -- doctor

# 2) 跑一个短且有界的 workflow（free 模型）
cargo run -q --bin cs -- workflow run \
  --agent builtin:coding \
  --model cs@free:ds-v4-flash \
  --prompt "Reply with exactly: OK" \
  --follow
# 记录输出中的 session_id，例如 SESS=0rkyq74ag0400

# 3) capture（只读）
cargo run -q --bin cs -- experiment capture "$SESS" --artifact-dir /tmp/cs2a-artifact

# 4) 关闭/断开主进程后离线验证（无网络/无 DB/无 key）
cargo run -q --bin cs -- experiment inspect /tmp/cs2a-artifact --output json
cargo run -q --bin cs -- experiment replay  /tmp/cs2a-artifact --output jsonl

# 5) 篡改负向（复制后破坏，期望非零 + machine code）
cp -r /tmp/cs2a-artifact /tmp/cs2a-tampered
# 改 run.json 一个字节（不刷新 manifest）→ inspect 期望 hash_mismatch/manifest_mismatch
printf 'x' >> /tmp/cs2a-tampered/run.json
cargo run -q --bin cs -- experiment inspect /tmp/cs2a-tampered --output json   # exit=1

# 6) 隐私扫描（期望无命中）
grep -RiE "sk-[A-Za-z0-9]|Bearer |Authorization|/home/[^ ]*/(workspace|\.ssh)" \
  /tmp/cs2a-artifact/run.json /tmp/cs2a-artifact/snapshot.json /tmp/cs2a-artifact/events.jsonl || echo "NO SECRET HIT"

# 7) 确认无新增 workflow effect：capture 前后 snapshot/events 一致
cargo run -q --bin cs -- workflow get "$SESS" --output json > /tmp/cs2a-before.json
# （执行 capture 后）
cargo run -q --bin cs -- workflow get "$SESS" --output json > /tmp/cs2a-after.json
diff /tmp/cs2a-before.json /tmp/cs2a-after.json && echo "SNAPSHOT UNCHANGED (read-only capture)"
```

预期：capture 生成 5 文件固定布局；inspect/replay 在离线环境成功；篡改/缺文件/隐私命中均 fail closed；
capture 前后 snapshot/events 无差异（只读）。

## 4. 结论

2A 的 artifact schema v1、脱敏、canonical hash、event chain、manifest、原子 writer、
usage/cost 投影与离线 inspect/replay 已通过 51 项 `cs` focused tests、双 binary 无 warning 编译、
一期 HTTP/前端回归（36 + 62），以及 `pnpm tauri dev` 实例上的真实 CLI desktop smoke。真实
artifact 的离线 inspect/replay、隐私扫描与 capture 无副作用均已验证；手工篡改真实副本受
host/sandbox 临时文件系统隔离限制，等价 fail-closed 路径已由 focused tests 覆盖。

## 5. 2C Budgeted Experiment Run + Artifact Handoff（as built，2026-09-15）

### 5.1 已执行的确定性验证（offline，可复现）

`cd src-tauri`：`cargo fmt --all -- --check` 通过；`cargo check --bin chatspeed --bin cs` 双 binary 零 warning。
- `cargo test --lib workflow::react::experiment` 10 passed：strict `ExperimentRunSpecV1` round-trip + 负向矩阵
  （unknown field/version、`max_attempts!=1`、disk/network hard-cap、required 无 cap、currency mismatch、
  token-only 带 money cap）均在 effect 前拒绝。
- `cargo test --lib budget::` 66 passed：新增 `create_experiment_run_atomic` 原子性（恰一 workflow + 四级
  canonical scope）、重复 session 回滚零部分、无效 envelope 不写入；`get_budget_scope_chain` 无链→None、
  canonical→正解、非 canonical→fail-closed。
- `cargo test --lib chat::openai` 55 passed：AdmissionContext 仅在 durable chain 存在时生成（attempt=1、
  canonical scope ids），普通 session→无 context，非 canonical→send 前 fail-closed。
- `cargo test --lib workflow::react::client` 41 passed：`POST /control/v1/experiments:run` 缺 Idempotency-Key
  →400、无 bearer→401、unknown spec field / wrong version →400 且无 workflow 落库；valid run→201 且真实
  `MainStore` 落库恰一 workflow + 4 scopes、同 key/body replay 不双建、异 body→409。
- `cargo test --lib admission` 16 / `stat_guard` 4 / `ccproxy::handler` 28（2B 回归，含伪造 external
  admission header 走普通路径）；`workflow::react::{llm,intelligence,compression}` 41/6/59（budgeted retry
  门控无回归）；`cargo test --bin cs` 56（2A capture 重构无回归 + budget exit-9）；根目录 `pnpm test:workflow` 62。

### 5.2 真实 desktop 固定模型 smoke（V-7，2026-09-15 已执行）

用户关闭既有桌面实例后，`pnpm tauri dev` 以 2C 重建二进制成功启动（Vite 就绪、51 skills 加载、
control plane 发布 discovery，`cs doctor` 连上 `pid 1193377`）。真实 `cs@free:ds-v4-flash` 端到端结果：

1. **admitted completed run**：`cs experiment run --agent builtin:coding --spec <token_resource_only>
   --prompt "Say OK." --artifact-dir ... --output json` → 201 `started`（run_id=session_id=
   `0rmancd3g0400`，scopes `:trial/:candidate/:campaign` 正确派生）；workflow 达 `completed`；
   artifact `complete`、8 events；离线 `inspect`/`replay` 通过（terminal=completed、cost=known、
   usage input 22731/output 93/cache 9216），CLI exit 0。
2. **预算 ledger 真实触发**：completed run 与失败 run 均见 `[Budget][admission] reserved res_...
   effect llm:<session>:... (attempt 1)`；且**语言检测 helper 与主 ReAct 两个 attributed LLM effect
   各自 reserve**（helper coverage，AC-3）。
3. **极小 cap 负向（input_tokens=1）**：session `0rmap8ny00400` →
   `Experiment admission rejected before send: budget_exceeded: ... projected 200 exceeds hard cap 1`
   （language helper）与 `projected 30519 exceeds hard cap 1`（主 react），**provider 零调用**；
   language detection `attempt 1/1`（单 attempt，AC-4）；workflow 进 `error`。
4. **普通 workflow 对照（INV-4）**：session `0rmapkhdw0400` 全程 **0 条 `[Budget][admission]`**（无 request
   scope → 不进预算 gate/ledger）；且其 429 **按指数退避重试**：`Retrying in 1s (attempt 1/10)` → `2s
   (2/10)` → `4s (3/10)`。与 experiment 的单 attempt 形成明确对比。

**发现并修复的 CLI bug**：`wait_for_terminal` 原按 `"failed"` 匹配终态，但持久化 `WorkflowState`
序列化为 `"error"`，导致 `--artifact-dir` 在失败 run 上轮询到超时（看似"429 直接退出/挂死"）。已改为
`completed|error|cancelled`；失败 run 现在正确捕获 artifact 并按结局退出（completed→0，其它非完成终态→1，
可识别的预算拒绝→9）。

**已知限制（如实）**：LLM-path 的**执行期**预算拒绝目前只出现在 crash 日志与瞬时 error chunk，未进入
CLI 可读的 durable 事件/snapshot（captured events 仅 workflow_started/effective_task_objective_changed/
state_changed），故该异步场景 CLI 退出 1 而非 9；**同步** control-plane 预算 machine code 响应仍映射
exit 9（client decode `is_budget_code`，已测），**tool-path** 拒绝会写入 durable tool observation
（含 "experiment admission rejected"）故可被 `budget_rejected_in_events` 命中→exit 9。若需让 LLM-path
执行期拒绝也稳定 exit 9，需要后端把 admission 拒绝码持久化进 durable 失败事件（属 runtime 变更，另行评估）。

---

## 4. 2D+2E 冒烟记录（deterministic evaluator + 初始 chatspeed-smoke@1 adapter/verifier，2026-09-15）

### 4.1 环境与执行方式

- **真实 desktop 全链 smoke 已执行**：`pnpm tauri dev` 启动桌面实例（control plane
  `127.0.0.1:33185`，instance `b546ce632fe4ec856c56009c172accb1`），`cs doctor` 认证通过；
  免费模型 `cs@free:ds-v4-flash`（`benchmark run --model` 透传为 2C spec workflow override）。
- 另有离线 CLI 进程 smoke（无 discovery 文件时）先行完成，证明 evaluate/verify 离线性。

### 4.2 真实 desktop 全链结果（V-8，已执行）

```bash
cd src-tauri
# 1) 在线 run（经既有 POST /control/v1/experiments:run + 预算 admission，单次 attempt）
cargo run --bin cs -- experiment benchmark run --suite chatspeed-smoke \
  --task smoke_reply_ok --agent builtin:coding --model "cs@free:ds-v4-flash" \
  --artifact-dir /tmp/cs-2e-desktop-run3
# → run: 0rmczhh500400 (status=started)
# → cs: experiment artifact captured (terminal=completed, artifact=complete)，exit 0

# 2) 离线 evaluate
cargo run --bin cs -- experiment evaluate /tmp/cs-2e-desktop-run3 \
  --evaluation-dir /tmp/cs-2e-desktop-eval
# → correctness_status=pass，exit 0

# 3) 离线 benchmark verify
cargo run --bin cs -- experiment benchmark verify --suite chatspeed-smoke \
  --task smoke_reply_ok /tmp/cs-2e-desktop-run3 --verdict-dir /tmp/cs-2e-desktop-verdict --output json
# → score=1.0, safety=pass, infra=pass；绑定 run_id=0rmczhh500400、
#   chain_head=177ad5be…、dataset_digest=619ae2a2…、task_digest=3fcbf6ed…、
#   verifier_digest=612079ff…；exit 0

# 4) 篡改负向（复制后改 run.json 一字节，不刷新 manifest）
cargo run --bin cs -- experiment evaluate /tmp/cs-2e-desktop-tampered \
  --evaluation-dir /tmp/cs-2e-desktop-eval-tampered
# → cs: hash_mismatch: size mismatch for run.json；exit=1；无 sidecar 发布
```

应用日志证据：`[Budget][admission] reserved res_… for effect llm:0rmczhh500400:… (attempt 1)`
——预算 admission 在真实 desktop 链路中真实生效；artifact 实际使用模型 `deepseek-v4-flash`
（免费 `cs@free:ds-v4-flash`）。

### 4.3 fixture 资源 cap 校准记录（budget gate 端到端负向证据）

前两次 desktop run 因 admission 按设计 fail closed 被拒（provider 零调用）：

- run `0rmcvtxt80400`：`budget_exceeded: output_tokens projected 8192/128000 exceeds hard cap 2000`
  （language helper 与主 ReAct 请求均在发送前被拒）；
- run `0rmcye9w00400`：`budget_exceeded: input_tokens projected 30697 exceeds hard cap 20000`
  （首个 LLM effect 已成功 reserve，主请求发送前被拒）。

据此把 fixture caps 校准为 `input_tokens=65536`、`output_tokens=128000`（worst-case reserve 需
覆盖 agent 配置的 max tokens 与真实 prompt 规模），digest 同步更新并回写路线文档与 golden tests。
两次被拒 run 的 artifact 为 incomplete/cost unknown，verifier 如实给不可通过事实——这正是
"budget rejection 不是 correctness 成功"契约的真实体现。

### 4.4 离线 CLI 进程验证（desktop smoke 前先行完成，可复现）

```bash
cd src-tauri
# 无 discovery 文件时（证明无主进程/网络/DB/key）：
cargo run --bin cs -- experiment evaluate target/cs-2e-smoke/artifact \
  --evaluation-dir target/cs-2e-smoke/eval          # exit 0, correctness_status=pass
cargo run --bin cs -- experiment benchmark verify --suite chatspeed-smoke \
  --task smoke_reply_ok target/cs-2e-smoke/artifact \
  --verdict-dir target/cs-2e-smoke/verdict          # exit 0, score=1.0
# 篡改/未知 task/重复目标 → exit 1（hash_mismatch / unknown_task / target_exists），无发布
# symlink 父目录指向 artifact → exit 1（symlink_rejected），artifact 内无新增文件
```

### 4.5 隐私扫描

对发布的 `verdict.json`/`evaluation.json` 全文扫描：无 raw prompt、无 `Bearer `/`sk-`、无模型名、
无绝对路径（`path_hint` 仅目录文件名）；provenance 固定为
`artifact/benchmark_adapter/independent_verifier`（evaluation 为 `artifact/evaluator`）；
无任何 promotion 字段。

### 4.6 Verifier coverage clarification（2026-09-15）

初始 `chatspeed-smoke@1` 的 artifact v1 未持久化 `tool_calls`、`processes` 或 peak
`concurrency` 实际值，因此不能把 `score=1.0` 解读为这三项 admission cap 已被离线 artifact
独立验证。本次将 fixture/version 与 verifier 升为 `chatspeed-smoke@2` / `chatspeed-smoke-verifier@2`，
并将 verdict document schema 升为 v2：

- `metrics` 明确输出 `usage_within_{tool_calls,processes,concurrency}_cap=not_applicable`，附
  `actual=null` 与 `reason=not_recorded_in_artifact_v1`；它们不被伪装为 pass。
- `budget_facts` 明确划分 `admission_caps`（仍完整下发给 2C runtime）、
  `independently_verified_caps`（token/cache/wall time）及 `unverified_admission_caps`。
  因此 `score=1.0` 只代表可由可信 artifact 事实独立验证的 checks 均通过，不能暗示未记录资源被回验。
- `cargo test --bin cs verifier::tests` → **13 passed; 0 failed**；
  `cargo test --bin cs benchmark::tests::fixture_digests_match_golden_values -- --exact` →
  **1 passed; 0 failed**；v2 fixture/verifier digests 均由 golden tests 锁定。

sidecar 的既存 symlink/overlap fail-closed 防御保持不变。对于攻击者可并发替换任意输出父目录的
TOCTOU 威胁模型，跨平台 `std::fs` 的 path-based 操作无法在当前 CLI 契约内彻底消除该竞态；完全消除
需要 handle-relative 的 Unix/Windows writer/verifier 或将输出限制为攻击者不可写的可信根，均超出本次
2D+2E 风险收敛的最小兼容范围。

## 6. 2F Campaign / Candidate Stage 0（实现完成，真实 smoke 部分被 provider 限流阻塞，2026-09-16）

**状态：U-1..U-5 全部完成；V-1..V-8 通过。AC-1..AC-8 满足，阶段指针推进到 2G+2H。**
（首选用 `cs@free:ds-v4-flash` 因 provider 配额持续 429 无法完成三 campaign，经用户确认后改用
同为免费组的 `cs@free:qwen-3.8-flash` 完成真实交付；`ds-v4-flash` 的 429 记录保留在 6.3 作为
fail-closed 证据。）

### 6.1 环境与进程隔离（INV-7 / A-5）

- 环境中已存在**用户自己的** dev 实例（另一 checkout `/home/xc/dev/rust/chatspeed`，pid 454388 等，
  占用 Vite 默认端口 1420）。本次**未终止、未干扰**该实例：自动化审计在其启动前后都记录到同一组
  用户进程（5 个），全程未被 kill。
- 本次实例使用**独立** `CHATSPEED_HOME=/home/xc/dev/rust/chatspeed-cli/dev_data/p2f-home`，因此
  discovery 落在 `<repo>/dev_data/p2f-home/runtime/control-plane-v1.json`，不覆盖用户的
  `~/.chatspeed/runtime/control-plane-v1.json`。
- 启动命令（为避开用户实例占用的 1420，仅在**运行时**以 `--config` 覆盖 devUrl/beforeDevCommand，
  未修改仓库配置）：
  ```bash
  CHATSPEED_HOME=<repo>/dev_data/p2f-home pnpm tauri dev --config \
    '{"build":{"devUrl":"http://localhost:1431","beforeDevCommand":"pnpm exec vite --port 1431 --strictPort"}}'
  # wrapper(进程组) pid=533167，tauri.js pid=533183，app pid=552669，
  # instance d13a2e67cb9c7e67f4ffce6883077c9b，control plane 127.0.0.1:46731，ccproxy :11437
  cargo run -q --bin cs -- --discovery-file <repo>/dev_data/p2f-home/runtime/control-plane-v1.json doctor
  # → Connected ... instance d13a2e67..., protocol v1, pid 552669；认证/连通/协议 OK
  ```
- 退出审计：`kill -TERM -533167`（仅本次进程组）后 5 秒内记录的 5 个 PID 全部消失
  （`/proc/<pid>` 不存在），未使用无界 `pkill`；`kill -0` 语义复核确认 discovery 记录的
  pid 552669 已不存在；无残留 dev server 子进程（esbuild/sass/vite/tauri 全部退出）。用户的另一实例
  进程数在审计前后保持不变（5）。残留物只有 `dev_data/p2f-home/runtime/control-plane-v1.json`
  这一**指向已退出 pid 的陈旧 discovery**（位于本次隔离 home 内，不影响用户实例）。

### 6.2 真实 desktop 上已确证的行为（backend 权威路径）

在**真实 control plane**（非 mock）上实际执行并通过：

1. `experiment campaign create`：真实创建共享 campaign budget scope，返回
   `campaign_id=camp-3e46048f52c55ebbf5fc32b206f5f259`、`campaign_hash=cb2f688a…`、
   `envelope_hash=b2809c4c…`、`catalog_digest=1edb36fa…`、`status=active`；两个 candidate
   manifest sidecar 原子发布。
2. **确定性 campaign identity（AC-2/INV-6）**：用同一 plan 在另一个输出目录
   （`c1` → `c1b`）再次 `create`，**推导出完全相同的 campaign_id**，证明 campaign/plan 绑定
   可离线复现。
3. **重复发布拒绝（AC-6）**：在同一 `--out` 目录再次 `create` 稳定失败
   （`campaign_sidecar_exists`，非零退出）。
4. **真实 run 全链（AC-4/AC-5）**：`experiment campaign run` 提交真实 backend-owned run；
   模型调用因 provider 限流失败后，CLI 仍完成真实 artifact→evaluate→verify→consume 链并发布
   fail-closed sidecar：
   `run_id=0rmewdcvm0400, terminal_status=error, artifact_status=incomplete, score=0.0,
   safety_status=pass, infra_status=fail, consumed=false, chain_head=42afea4c…`，
   随后 `cs: campaign_stopped: candidate 'baseline' run 0rmewdcvm0400 terminated as error`（退出码 1）。
   即：**不伪造成功**，infra 失败即停止 campaign，并把已核实的事实写入不可变 sidecar。
5. **共享 campaign cap 在 provider 调用前生效（AC-1）**：
   ```text
   2026-09-16 00:21:40 [E] Workflow error: Ai(RawApiRequestFailed { status_code: 500,
   provider: "Internal Proxy", details: "内部服务器错误: experiment admission rejected
   (budget_exceeded: input_tokens dimension)" })
   ```
   这是一个真实 workflow 在**同一 campaign scope** 上因累计 input_tokens 超过 envelope 而在
   provider 之前被拒（此前的失败 run 的 reservation 已计入 campaign 级 committed/reserved），
   证明 CLI 事后累加成本并未被当作 gate，campaign cap 由 ledger 权威执行。
6. **普通 workflow 无 budget 对照**：同一实例上 `cs workflow run --model cs@free:ds-v4-flash`
   在 00:15:30 成功 `completed`（session `0rmewxk4c0400`），说明失败不是 desktop/control plane
   或模型路由故障。

### 6.3 `cs@free:ds-v4-flash` 配额阻塞与模型切换（用户确认）

固定首选免费模型 `cs@free:ds-v4-flash`（free 组 `deepseek-v4-flash`，provider 日日新）在
2026-09-16 00:13–00:35 窗口内**持续返回 429**，每次 campaign run 的 LLM effect 均在 provider 前
正常 reserve、随后被 provider 拒绝：

```text
2026-09-16 00:13:52 … Backend API error (alias: 'free:ds-v4-flash', model: 'deepseek-v4-flash',
  provider: '日日新') status_code=429 Too Many Requests
  response={"error":{"message":"inference exceeds tpm/rpm limit","type":"rate_limit_error","code":"429001"}}
2026-09-16 00:17:16 / 00:22:53 / 00:27:21 / 00:34:53 同 429（含 4 分钟与 7 分钟静默窗口）
```

共 6 次真实尝试全部 429（唯一一次成功是 00:15:30 的普通 workflow 对照，见 6.2 第 6 条）。
按用户确认：改用同为**免费组**的 `cs@free:qwen-3.8-flash` 完成三 campaign 真实交付（仍是免费模型，
不违反 D-5 的“首版不做收费正向测试”）；`cs@qwen3.8-flash` 作为最后回退未被使用。

### 6.4 三个独立 campaign 的真实交付（V-7，模型 `cs@free:qwen-3.8-flash`，concurrency=1）

每个 campaign：immutable plan → `create`（冻结共享 campaign scope + 两个 candidate sidecar）→
`run baseline` → `run prompt-a`（每个 run 真实完成 artifact→evaluate→verify→campaign consume）→
`close`（发布 campaign-summary）。所有 run 均达到 durable `completed`，无自动重试、无并发。

| campaign_key | campaign_id | baseline run | candidate run | 双方 score |
|---|---|---|---|---|
| p2f-smoke-21 | `camp-4f2bb0b4f90760e56bdf9853155fb4ab` | `0rmjnz81g0400` | `0rmjpak6c0400` | 1.0 / 1.0 |
| p2f-smoke-22 | `camp-22bea6c4b6d431dcffeadfed60c100ba` | `0rmjppza40400` | `0rmjq32a40400` | 1.0 / 1.0 |
| p2f-smoke-23 | `camp-8a039947a84e9a82d586a5145c8f72f9` | `0rmjqf7p00400` | `0rmjqv3e40400` | 1.0 / 1.0 |

三份 `campaign-summary.json` 均为 `close.status=closed`、`changed=true`、`missing_arms=[]`、
`produced_promotion=false`；每个 run 的 `safety_status=pass`、`infra_status=pass`、
`cost_status=known`，且 `chain_head`/`verdict_hash` 互不相同（独立证据，非复制）。

**Candidate surface 真实生效且不污染 defaults（AC-3/INV-4/INV-6）**：同一 campaign 内
baseline artifact 的 snapshot 为 `"experimentAgentPromptRef": null` /
`"experimentAgentPromptHash": null`（Agent defaults 未变），candidate artifact 中两者均为
非空 redacted 投影（`{len, sha256, type}`）——即 ref/hash 进入 workflow-local config，
而 prompt 原文不出现在 artifact（隐私边界成立）。

**负向（全部 fail closed，无伪造成功）**：

1. **campaign cap 在 provider 调用前拒绝（AC-1）**：`campaign-31`（input/output cap=1）的 run：
   ```text
   09:14:04.078 [W] Experiment admission rejected before send: budget_exceeded:
     scope camp-2b8fffd9cca5e8fd925e5606a148720e dimension input_tokens
     projected 30523 exceeds hard cap 1
   ```
   外部 provider 零调用；run 终态 `error`，campaign 停止并发布 fail-closed sidecar。
   另在 00:21:40 用小 envelope 复现了跨 run 聚合 cap 拒绝（同 campaign scope 累计超限）。
2. **重复消费同一 arm**：`campaign run --candidate baseline`（q1 已消费）→
   `campaign_run_already_consumed`，退出码 1，无新 run。
3. **close 后继续 run**：`close n1`（`status=closed`）后对未消费 arm 再 run →
   `server error 400: campaign … is closed (campaign_not_active)`，无新 workflow 创建。
4. **篡改 campaign sidecar**：复制 q1 后改写 `campaign.json` 数据文件（不刷新 manifest）→
   `campaign inspect` 返回 `hash_mismatch: sidecar data file hash mismatch`，退出码 1；
   未篡改的 q1 `inspect` 通过（`sidecar_verification: passed`），且 inspect 全程离线
   （不加载 discovery、不访问网络/DB）。

**已知限制**：mid-run 的 admission 拒绝信息只出现在 server 日志，不在 durable events/snapshot 中，
因此 CLI 对这类 run 以 `campaign_stopped`（exit 1）停止而非 exit 9；exit 9 仍适用于
创建期即被拒的 budget 错误（HTTP budget code → `CliError::budget`）。这与既有 2C 行为一致，
不构成 2F 回归。

### 6.5 退出审计（V-8）

- 所有已启动 run 均为 durable `completed|error`（6 个交付 run `completed`；负向 run `error`），
  无 polling timeout 冒充终态。
- 三个交付 campaign 均 `close`（`status=closed`），负向 campaign `n1` 亦显式 close。
- 结束后对**本次启动**实例（进程组 787361，含 wrapper/tauri-cli/vite/esbuild/sass/app/MCP 子进程
  共 12 个 PID）发送 `SIGTERM`：5 秒后组内全部 PID 消失，12 秒后复核仍为空，
  discovery 记录的 app pid 787551 不存在；未使用无界 `pkill`。
- 用户自己的另一 dev 实例（另一 checkout）进程在审计前后持续存活，未被误杀。
- 残留物：`dev_data/p2f-home/runtime/control-plane-v1.json` 为指向已退出 pid 的陈旧 discovery
  （位于本次隔离 CHATSPEED_HOME 内，不影响用户实例）。

### 6.6 本阶段已完成并复核的离线验证

```text
cd src-tauri && cargo fmt --all -- --check                 # clean
cargo check --bin chatspeed --bin cs                       # 0 warnings
cargo test --lib db::budget::                              # 24 passed
cargo test --lib migration                                 # 15 passed（无新 migration）
cargo test --lib workflow::react::campaign                 # 20 passed
cargo test --lib workflow::react::client                   # 45 passed（含 4 个 campaign HTTP 路由测试）
cargo test --lib workflow::react::experiment               # 10 passed
cargo test --lib commands::workflow                        # 67 passed
cargo test --bin cs                                        # 117 passed（含 7 个 2F campaign/verdict 测试）
pnpm test:workflow                                         # 62 passed
```

其中 2F 新增覆盖：strict plan/create/run-intent 契约与 canonical hash（unknown/forbidden/重复
candidate/非法 surface/未 allowlist ref/hash 不匹配/fixture digest 不匹配）、共享 campaign scope
的 parent linkage 与跨 run 聚合 cap、closed campaign 拒绝新 run 与 admission、envelope 不匹配、
request/candidate scope 复用规则、HTTP bearer+idempotency+stable machine code、verdict 独立复验
（manifest/内容 hash/artifact binding/fixture 身份/safety-infra-budget 事实/promotion 拒绝）、
sidecar 重复发布与篡改拒绝、campaign prompt ref 进入 workflow-local Agent 且 Agent defaults 不变、
catalog 漂移 fail-closed。

## 7. 2G+2H Headless + Durable Scheduler Smoke（2026-09-17，真实 Docker owner + 真实模型）

实现细节、缺陷根因与修复见 `work/agent-cli-phase-2-implementation-plan.md` §11.7。

- **实例**：`chatspeed-headless --data-dir dev_data/2gh-smoke/domain --base-repo <tmp repo>
  --api-key-file <operator key file>`；独立 DB/布局/marker/lease；**自带 loopback ccproxy**
  （`Serving chat completion proxy on http://127.0.0.1:11437`），discovery 在 domain 内。
- **提交**：`cs --discovery-file <domain discovery> experiment campaign schedule --plan plan.json
  --profile docker-smoke`（纯 HTTP；`campaign_schedule_accepted.v1`；1 个有序 job；concurrency=1）。
- **结果**（`dev_data/2gh-smoke/evidence.txt`）：job `succeeded`、marker `confirmed`、
  `run_id=0rmsf5a1c0400`；journal `workspace_acquired → environment_ready → dispatch_intent →
  workflow_started → artifacts_collected → cleanup_done`；快照 `Running → Completed`；
  输出补丁 + manifest 已发布（绑定 job/run/session/candidate/base_revision）。
- **模型顺序**（用户指定）：`cs@qwen-3.8-flash`（该域未配置该 alias，确定性不可用）→
  `cs@free:gemini-flash`（上游 503，已知失败）→ `cs@free:ds-v4-flash`（**成功**）。
  未使用付费回退 `cs@qwen3.8-flash`；全部失败均为**已知**失败，未出现 `unknown_manual`。
- **隔离与残留审计**：无遗留 owner 容器（按 `cs.owner_schema` label 过滤为空）、base repo
  `status` 干净且只有一个 worktree、HEAD 未变；SIGTERM 后进程退出、discovery 由实例自行删除、
  domain lease 归零。
- **desktop（AC-8）**：`pnpm tauri dev` 在改动后已自行重编译（运行二进制 mtime 00:51 > 源码 00:21），
  真实桌面实例控制面可连（`cs doctor` OK），并经新的共用 launcher 拥有自己的 ccproxy（127.0.0.1:11436）。
- **进程级 crash/restart 矩阵（V-3）**：`dev_data/2gh-crash/matrix.txt`
  - A **预派发重启恢复**：SIGKILL 时 job 为 `queued/not_dispatched` → 重启后 **`succeeded/confirmed`**，
    journal 完整 6 阶段（run `0rmsxwnwg0400`），run kernel 仅被调用 1 次；
  - B/C kill 于 dispatch intent 之后（`dispatching/intent_recorded`，无 run id）→ 重启后
    `unknown_manual` + `dispatch_uncertain`，**run kernel 调用次数保持 0**（INV-5 不重放）；
  - 崩溃 generation 的 domain lease 未过期时，重启以 `experiment_domain_locked` 拒绝启动（fail-closed 生效）。
- **退出残留 + 敏感内容审计（INV-6）**：`dev_data/2gh-smoke/gate-evidence.txt`
  - fixture instruction 在 experiment 三张表中出现 **0** 次；journal/artifact/log 中
    `sk-/ghp_/xoxb-/-----BEGIN/AKIA` 命中 **0**；discovery 消失、lease 释放、无进程、无 owner 容器、
    base repo 干净（崩溃遗留的 2 个 worktree 已按 path 校验后清理）。
- **AC-7 hand-off 修复**：adapter 的 capability manifest 现写在
  `<data-dir>/runtime/harbor-task-capability.json`（与 runtime 一致），并新增 2 个 focused test
  用真实 Python producer 断言路径/token 契约与 declared-roots↔task-image 一致性。
- **真实 Harbor 0.23.0 trial：通过（提交时最新一轮）**（`dev_data/2gh-harbor/harbor-trial.txt`、`jobs/2gh-trial/**`）：
  Harbor `Trials 1 / Mean 1.000 / Exceptions 0`；separate verifier `reward=1`，verdict
  `verified 2 declared artifact(s); all 2 job(s) reached 'succeeded'`；Harbor 收集 `/logs/artifacts` 成功，
  内含 `chatspeed-campaign.json`、`chatspeed-jobs.jsonl` 与两个 job 的 `patch.diff` + `patch-manifest.json`；
  durable rows：`job-2b49e2b8… baseline succeeded/confirmed run=0rmyapb1w0400`、
  `job-e03434a7… cand-a succeeded/confirmed run=0rmyapzj40400`，profile 均为 `harbor-task`。
  关键接线：任务镜像由 `ubuntu:26.04`（glibc 2.43）+ GTK3/WebKitGTK + `python3` 构建并携带 `/tests` verifier 入口；
  job 环境提供模型端点为宿主桌面实例 ccproxy 的 grouped 形状 `http://127.0.0.1:11436/cs/v1`
  （客户端拼接 `/chat/completions`，curl 实测该 URL 返回 200）；agent+模型经 0600 config package 注入 domain，
  不进入任何 artifact。此前记录的“base 镜像/Docker Hub 阻塞”已由用户提供的 `ubuntu:26.04` 解除，
  不再是当前阻塞项。
- **凭据通道（INV-6）**：模型 token 经 `environment.upload_file()` 以 0600 文件送入沙箱，
  **不进入任何被 Harbor 记录的命令**；扩展后的 `dev_data/2gh-smoke/gate-audit.sh` 会扫描 Harbor 通道
  （job.log/trial.log/agent logs/artifacts，24 个文件），修复后复验结果为 `markers=0`、`token occurrences=0`。
  扫描器实现为 `dev_data/2gh-smoke/scan_secrets.py`，其检测能力由阳性对照
  `dev_data/2gh-smoke/audit-selftest.sh` 验证（植入凭据可被发现、capability 文件名不误报、
  未提供 token 时拒绝报告干净），因此该 `0 命中` 是有检测能力背书的结论。

## 8. 2I Promotion / Paired Canary Smoke（2026-09-17，真实 local Git + 真实容器 canary）

**范围与口径**：2I 的自我改进闭环不含 LLM/tool/network effect（INV-9），候选生成仍在 2B/2F 边界之后，
因此本轮 smoke 是**确定性的端到端验证**：真实临时 Git 仓库 + 真实实验 domain（v20 库、真实 target/profile/
allowlisted bundle 注册）+ 真实 supervisor tick + 真实 checkpoint owner + 真实 digest-pinned 容器
paired canary。可执行形式为 `src-tauri/src/workflow/react/experiment_promotion/smoke.rs`（`cargo test --lib
experiment_promotion::smoke`），容器不可用或无本地 digest-pinned 镜像时**显式 skip**，不伪造通过。

**场景与结果**（本机 Docker + 本地 digest 镜像可用，实际执行，非 skip）：

1. `smoke_two_promotions_form_linear_commits_and_a_failure_does_not_advance`
   - 三个 candidate：prompt-a / prompt-b 改进（各加一个 improvement 文件）、prompt-c 回退第一个改进。
   - #1、#2 均走完 `queued → evidence_validating → checkpointing → checkpointed → canary_running →
     ready_to_advance → advancing → promoted`，实验分支 `refs/heads/experiment/2i` 两次前移，
     `rev-parse <head>^` 证明**连续成功节点形成线性本地 commits**。
   - checkpoint commit 为英文 subject `experiment(promotion): checkpoint <promotion_id>`，含全部
     trailers（Promotion-Id / Evidence-Hash / Patch-Sha256 / Base-Revision / Target-Ref）；
     `refs/chatspeed/checkpoints/<promotion_id>` 与分支一致。
   - #3 的 campaign 指标显示改进，但 workspace canary 实测回退 → `canary_failed`
     （machine code `canary_stage_failed`），**分支保持不动**，且其 checkpoint commit/ref
     **保留不删**（失败证据不丢失，INV-6）。
   - 审计：仓库无任何 `remote.*` 配置（无 push/remote effect 可能），`git status --porcelain` 干净，
     base repo 的 HEAD/index 未被 promotion 触碰（canary 与 checkpoint 均在独立 worktree/容器内）。
2. `smoke_a_restart_recreates_the_checkpoint_exactly_once`
   - 第一个 tick 后停在 `checkpointing`（intent 已持久、effect 未发生、ref 不存在，可证明未发生），
     丢弃 supervisor（模拟崩溃/重启），新 supervisor 继续 → 达到 `promoted`。
   - `rev-list --count base..head == 1`：**checkpoint commit 恰好一次**；journal 中
     `checkpoint_intent` 与 `checkpoint_created` 各恰好一条（INV-7）。
3. `smoke_the_audit_scanner_has_a_positive_control`
   - 密钥扫描阳性对照：植入 `sk-…` 的文件被发现（1 hit），干净目录 0 hit——扫描能力有背书，
     而非未验证的 grep。本轮任务产物（smoke 目录/日志/audit 文档）扫描 0 hit。

**本轮聚焦验证（全部通过）**：`cargo test --lib experiment_promotion`(41)、`experiment_owner`(51)、
`db::experiment_promotion`(8)、`db::sql::migrations`(15)、`db::experiment_schedule`(15)、`headless`(34)、
`workflow::react::campaign`(20)；`cargo fmt --all -- --check` 通过；三 binary
`chatspeed / chatspeed-headless / cs` check 0 error（仅存与本轮无关的既有 `private_bounds` warning）。

**未执行项（如实说明）**：真实进程级 SIGKILL 矩阵未跑（本轮以“丢弃 supervisor + 新 supervisor 继续”的
进程内重启覆盖同一恢复路径）；`pnpm tauri dev` + 真实模型的桌面端全链路未跑——2I 自身不新增 LLM effect，
真实模型链路属于 2B/2C/2F 的既有验证范围，用户提供的 `cs@qwen3.8-flash` 通道可用于后续候选生成侧验证。

### 8.1 终审修复（2026-09-17，canary 收敛 / CLI 等待预算 / 真实 SIGKILL 矩阵）

终审指出三项必须修复，全部已落实并以真实执行验证：

**1. canary 非成功结果全部收敛终态**（`scheduler.rs::converge_canary_failure`）：

- 门禁类失败（stage 回退、超时、结构化结果不可信——malformed/oversize/字段不匹配）→
  `canary_failed`（machine code 稳定：`canary_stage_failed` / `canary_result_invalid`），
  分支不动、checkpoint 证据保留；
- 环境类失败（容器运行时不可用等）→ `unknown_manual`（park），绝不盲目重试。
- 验证：`a_canary_that_cannot_be_trusted_converges_without_touching_the_branch`——
  (a) canary 程序输出垃圾 → `canary_failed`/`canary_result_invalid`，分支保持 base_head，
  error_code/branch_intent=not_started/checkpoint 保留；后续 tick 不再 re-claim（attempt 不变，终态收敛）；
  (b) 镜像 digest 本地不存在（owner 永不 pull）→ `unknown_manual`/`executor_unavailable`，分支不动。

**2. CLI `promotion run` 等待预算与服务器上界一致**（`promotion.rs`）：

- 新 `minimum_wait_budget_secs()` = `MAX_CANARY_STAGES(8) × MAX_CANARY_TIMEOUT_MS(900s) × 2 arms + 300s`
  覆盖开销 = 14,700s（原固定 900s 会在合法 canary 仍运行时提前放弃，导致 audit 不导出）；
  调用方只能上调不能缩短（`resolve_wait_budget_secs`）；轮询循环抽为
  `wait_for_terminal(initial, fetch, budget, poll)`（首次使用提交返回值，不重复请求；终态即返；
  超时报错携带 promotion_id 便于后续 `status` 查询）。
- 验证：`promotion::tests` 4 项（预算覆盖上界且 >900s、override 只能上调、终态立即返回不轮询、
  非终态轮询至终态、预算耗尽报超时且携带 id）。

**3. 真实进程 SIGKILL/restart 矩阵**（`experiment_promotion/sigkill.rs`，真实 headless 子进程 +
真实 SIGKILL；Docker 不可用或无本地 digest 镜像时显式 skip）：

- **checkpoint 边界**：child#1 启动真实 headless → 父进程轮询 DB 至 `checkpointing` → **SIGKILL** →
  等域/促销租约过期 → child#2 重建 → `promoted`；`rev-list --count base..branch == 1`
  （checkpoint 恰一次）、journal `checkpoint_intent`/`checkpoint_created`/`branch_advanced` 各恰一条、
  无孤儿容器/worktree、仓库无 remote。
- **canary 中途 SIGKILL**：慢速 canary 程序（sleep 8s）给出确定窗口 → `canary_running` 后 3s **SIGKILL**
  （双臂容器运行中）→ 重启 → `promoted`；重启侧 `cleanup_stale_arms` 回收死亡尝试的
  容器/registered worktree/未注册目录（命名由 backend-minted promotion id 派生，ownership 可证明），
  `rev-list --count == 1`，无孤儿。
- **branch 边界三态**（真实子进程执行恢复）：CAS 已应用但未记录（`advancing`+intent，分支已在
  checkpoint）→ **roll-forward**，分支不再移动，`branch_advanced` 恰一条；CAS 未应用（分支仍在 old）→
  **恰一次 CAS** 后 `promoted`；分支被外部移到第三值（`commit-tree` 生成孤儿提交对象）→
  **park `unknown_manual`**，分支绝不被覆盖，checkpoint 证据保留。
- 实现要点：子进程 = 测试二进制自执行（`--exact` 全限定名）并 `bootstrap::start` 真实 headless；
  `CHATSPEED_PROMOTION_LEASE_MS` 运维环境变量可缩短死实例租约窗口（默认 15min 不变）；
  各场景 patch 内容加 scenario 盐，避免 content-derived promotion id 在全局 docker 命名空间互相污染；
  Docker 密集测试用 `DOCKER_GATE` 串行化。

**修复后回归（全部通过）**：`experiment_promotion` 46、`experiment_owner` 51、
`db::experiment_promotion` 8、`db::sql::migrations` 15、`db::experiment_schedule` 15、`headless` 34、
`workflow::react::campaign` 20、CLI `promotion::tests` 4；`cargo fmt --all -- --check` 通过；
三 binary check 0 error。
