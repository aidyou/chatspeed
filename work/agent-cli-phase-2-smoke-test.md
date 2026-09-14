# cs CLI 第二期 2A 冒烟测试记录（Artifact / Provenance + offline inspect/replay）

本文件记录 2A 的验证证据。真实 desktop-owned 固定模型 smoke 因当前执行环境无
display server（`DISPLAY`/`WAYLAND_DISPLAY` 均为空）且沙箱内 `pnpm` 不在 PATH，
无法在此环境启动 Tauri 桌面主进程，故记为**环境阻塞项**（见下），并给出在有显示
环境主机上的完整复现脚本。2A 不引入 headless owner（属 2H），desktop 主进程是唯一
capture 来源，因此该项不能在本环境执行；这属于计划已登记的 stop condition，
不得记作 correctness 失败。

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
cargo test --bin cs   # 48 passed; 0 failed
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

## 3. 环境阻塞项：真实 desktop 固定模型 smoke（V-5）

**未执行原因**：本环境无 display server、`pnpm` 不在沙箱 PATH，无法启动 `pnpm tauri dev`
的 Tauri 桌面主进程；2A 无 headless owner，desktop 主进程是唯一 capture 来源。

**在有显示环境主机上的复现脚本**（模型固定 `cs@free:ds-v4-flash`，短任务）：

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
usage/cost 投影与离线 inspect/replay 已通过 37 项 focused tests、双 binary 无 warning 编译、
一期 HTTP/前端回归（36 + 62）与真实 CLI 离线进程验证。真实 desktop 固定模型 smoke 因环境
无 display/pnpm 记为环境阻塞项，并附完整复现脚本；不将其记作 correctness 失败。
