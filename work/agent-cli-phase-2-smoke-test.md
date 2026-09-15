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

## 4. 2D+2E 冒烟记录（deterministic evaluator + chatspeed-smoke@1 adapter/verifier，2026-09-15）

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
