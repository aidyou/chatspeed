# ChatSpeed Bash 沙箱执行设计草案

> 状态：暂缓实施，保留设计与调研结论  
> 调研日期：2026-07-23  
> 调研版本：Microsandbox CLI v0.6.6  
> 目标：后续恢复该任务时，能够直接从本文件进入细化与实施阶段。

## 1. 背景

ChatSpeed 当前允许 Agent 通过 `bash` 工具运行命令，并已有以下安全能力：

- `ShellPolicyEngine` 对命令进行 `Allow`、`Review`、`Deny` 分级；
- `PathGuard` 约束允许访问的目录；
- Workflow runtime 提供结构化审批队列、Smart 审核和一次性批准缓存；
- `ShellExecute` 负责本机进程启动、超时和 stdout/stderr 流式输出。

但只要命令最终运行在宿主机，命令仍可能访问宿主环境、凭据、进程和网络。计划引入 Microsandbox（CLI 命令为 `msb`），优先在本地 microVM 中运行 Agent 发起的 Bash 命令；无法使用沙箱时，保留本机执行能力，但将本机执行提升为强制人工审核。

本设计涉及命令路由、审批语义、运行环境、镜像供应链、跨平台兼容和用户设置，改动范围较大，因此先记录设计，不立即实现。

## 2. 产品定位

Microsandbox 应被定义为：

> ChatSpeed 首选但可选的 Bash 安全执行后端。环境满足时自动使用；环境不满足时，经用户明确审核后才允许在本机执行。

它不应成为：

- ChatSpeed 的安装或启动硬依赖；
- 唯一命令执行后端；
- `ShellPolicyEngine` 或 `PathGuard` 的替代品；
- 对所有 macOS、Linux、Windows 设备均可用的承诺；
- macOS/Windows 原生构建环境的替代品。

推荐提供三种用户模式：

| 模式 | 建议定位 | 行为 |
| --- | --- | --- |
| `auto` | 默认，适合普通用户 | 沙箱就绪时使用沙箱；否则本机执行前强制审核 |
| `sandbox_only` | 高安全需求 | 沙箱不可用、镜像缺失或命令不兼容时阻止执行 |
| `host_only` | 兼容模式 | 始终在本机执行，所有 Bash 命令强制审核 |

不要把“Full 审批等级”和“允许无审核本机执行”绑定在一起。本机执行风险应由独立安全约束控制。

## 3. 已确认的 Microsandbox 能力与限制

### 3.1 官方能力

Microsandbox 官方说明包括：

- 使用 microVM 提供硬件级隔离；
- 支持标准 OCI 镜像；
- 无需常驻 daemon；
- 支持 macOS、Linux 和 Windows 宿主；
- 支持临时与长生命周期沙箱；
- 支持目录挂载、网络规则、资源限制和超时；
- 官方宣称 M1 上 guest boot 平均低于 100ms。

官方资料：

- <https://github.com/superradcompany/microsandbox#readme>
- <https://github.com/superradcompany/microsandbox/blob/main/SECURITY.md>
- <https://docs.microsandbox.dev/cli/overview>

### 3.2 平台前提

Microsandbox 当前要求：

| 宿主平台 | 前提 |
| --- | --- |
| macOS | 仅 Apple Silicon，不支持 Intel Mac |
| Linux | KVM 已启用，且当前用户有使用权限 |
| Windows | WHP 已启用，硬件虚拟化可用 |

因此“跨平台”表示同一个产品支持三类宿主，不表示每台设备都能使用。企业设备、虚拟机中的系统、未开启虚拟化的 Windows、无 KVM 权限的 Linux，以及 Intel Mac 都可能无法运行。

### 3.3 Beta 风险

Microsandbox 官方仍明确标记为 beta，并提示可能存在破坏性变更、功能缺失和使用粗糙之处。

由此得出以下约束：

- 第一阶段通过 CLI 适配，不直接链接 Microsandbox Rust SDK；
- 锁定经过验证的 CLI 版本范围；
- 超出验证范围时显示兼容性警告，不盲目继续；
- CLI 参数和 JSON 输出必须有适配测试；
- 不由 ChatSpeed 静默安装或升级 Microsandbox；
- 不把 Microsandbox 的安全声明等同于 ChatSpeed 完整的安全证明；
- 保留替换执行后端的代码边界。

初期可以把 `>=0.6.6,<0.7.0` 作为候选兼容范围，但实施时必须重新核对最新 CLI 和发布说明。

### 3.4 Linux guest 限制

Microsandbox 在各宿主上运行的是 Linux OCI guest。它适合：

- Python、Node、Rust 的大部分脚本、检查和测试；
- Git、curl、awk、rg 等通用 CLI；
- Linux 目标的构建；
- 不依赖宿主 GUI 和系统服务的任务。

它不能透明替代：

- macOS 原生 Tauri 打包、签名、公证；
- Windows 原生 Tauri 打包和安装器生成；
- 宿主 GUI 启动与窗口交互；
- 钥匙串、证书存储、系统浏览器等宿主集成；
- 依赖宿主 Docker socket、SSH agent 或原生工具链的命令。

因此应把镜像命名为 `tauri-linux`，而不是含义模糊的 `tauri`。需要宿主原生能力的命令进入“本机执行 + 强制审核”路径。

### 3.5 本机验证结果

在 macOS Apple Silicon、Microsandbox CLI v0.6.6 上完成了以下验证：

- `msb doctor` 正确报告 CLI、`libkrunfw`、架构和宿主准备状态；
- `msb image list --format json` 可结构化读取本地镜像缓存；
- 使用缓存的 `python:3.12-slim` 成功启动临时 microVM；
- 项目目录可只读挂载到 `/workspace`；
- `--workdir /workspace` 生效；
- `--security restricted` 与 `--no-net` 可组合使用；
- guest 中 Python 可正常运行；
- 命令结束后 `msb list --format json` 为空，没有残留实例；
- 完整的一次启动、挂载、执行和退出在该机器上约为 3.5 秒。

最后一项是端到端观测，不等同于官方所说的 guest boot 时间。

## 4. 核心安全不变量

后续实现必须保持以下不变量。

### 4.1 硬性拒绝始终优先

现有 `ShellPolicyEngine::Deny`、路径越界和规划模式限制不因使用沙箱而放宽。沙箱只降低宿主逃逸风险，不能避免命令删除或破坏已挂载的项目目录。

决策优先级应为：

```text
hard deny
  > host execution review floor
  > shell policy review
  > shell policy allow
```

### 4.2 本机执行必须审核

当最终执行后端为宿主机时：

- 即使 shell policy 已显式 `allow`，仍然审核；
- 即使命令曾在沙箱内被批准过，仍然审核；
- 即使 workflow 使用 Full 审批等级，默认仍然审核；
- 审批必须绑定确切的 `tool_call_id` 和原始命令；
- 审批 UI 必须明确显示本机执行及其原因。

将来如需允许专家跳过本机审核，必须是独立、明显标记为高风险的设置，不能复用普通 shell allowlist。

### 4.3 沙箱失败不得静默降级

禁止以下流程：

```text
msb run failed -> automatically execute on host
```

攻击性命令可能故意触发镜像、挂载或启动失败，以诱导系统进入宿主执行。

正确流程是：

```text
msb run failed
  -> report structured sandbox failure
  -> stop current execution
  -> offer a new host-execution approval
  -> execute once only after explicit approval
```

“执行前已知无法使用沙箱”和“已经尝试沙箱但运行失败”必须区别处理：

- 前者可直接生成本机执行审批；
- 后者不得在同一次工具调用中自动切换后端。

### 4.4 解析一次并保持执行目标稳定

执行后端必须在审批前解析，并作为结构化元数据进入 pending approval。用户审批之后，不应因镜像状态或检测缓存变化而悄悄更换执行位置。

审批至少应包含：

```json
{
  "execution_backend": "host",
  "fallback_reason": "sandbox_image_missing",
  "sandbox_profile": "node",
  "sandbox_image": "ghcr.io/chatspeed-ai/sandbox-node:2026.07"
}
```

这些数据由后端生成，不能信任模型传入同名字段。

### 4.5 沙箱不替代路径与数据保护

只挂载用户已授权的工作目录。默认禁止挂载：

- 用户主目录；
- Docker/Podman socket；
- SSH agent 与 SSH 私钥；
- `.npmrc`、Cargo credentials、云服务配置；
- 系统钥匙串或证书私钥；
- ChatSpeed 自身数据库和配置目录。

项目目录内的 `.env`、私钥或凭据一旦可被 guest 读取，仍可能被联网命令泄露。挂载范围和网络策略必须共同考虑。

## 5. 推荐架构

### 5.1 执行后端模型

建立一个小而明确的后端边界，不在第一阶段设计复杂插件系统：

```rust
enum ShellExecutionBackend {
    Microsandbox(SandboxExecutionPlan),
    Host(HostFallbackReason),
}

struct SandboxExecutionPlan {
    profile: SandboxProfile,
    image: String,
    working_dir: String,
    mounts: Vec<SandboxMount>,
    network: SandboxNetworkPolicy,
    limits: SandboxResourceLimits,
}
```

主要职责拆分为：

```text
SandboxDetector
  -> 检测平台、msb、版本、doctor、镜像

SandboxProfileResolver
  -> 根据项目和用户配置选择 profile/image

ShellExecutionResolver
  -> 生成稳定的 Microsandbox 或 Host 执行计划

ShellPolicyEngine
  -> 继续做命令与路径安全决策

Approval Interceptor
  -> 合并 shell decision 与 backend risk floor

ShellProcessRunner
  -> 根据已确认的计划启动 msb 或宿主 shell
```

### 5.2 推荐执行流

```text
Model proposes bash command
  -> validate command and hard security policy
  -> resolve project sandbox profile
  -> resolve sandbox availability and exact backend
  -> combine shell decision with backend risk floor
  -> if review required, persist structured pending approval
  -> execute the already-resolved backend
  -> stream stdout/stderr
  -> return structured result including actual backend
```

实际执行后端也应进入 tool result 元数据和审计日志，方便定位“为什么这条命令在本机运行”。

### 5.3 与现有代码的结合点

当前主要结合点：

- `src-tauri/src/tools/shell.rs`
  - `ShellPolicyEngine`：保留安全判定；
  - `ShellExecute::call`：当前非流式本机启动入口；
  - `ShellExecute::call_with_streaming`：当前流式本机启动入口；
- `src-tauri/src/workflow/react/interceptors.rs`
  - `should_intercept_for_approval`：审批等级与 allowlist 判定；
  - `handle_bash_security_intercept`：Shell 安全拦截；
- `src-tauri/src/workflow/react/engine.rs`
  - Shell tool 注册与 session 配置注入；
- `src-tauri/src/workflow/react/policy.rs`
  - 现有 Workflow 审批等级，不宜直接承载全部沙箱配置；
- `src-tauri/src/db/agent.rs`
  - 当前 Agent shell policy 与 approval level 的持久化模型；
- `src/components/workflow/ApprovalDialog.vue`
  - 需要显示结构化执行位置、profile 和降级原因；
- Agent 设置与 Workflow 输入区
  - 需要显示模式、状态、镜像和项目 profile。

修改 workflow runtime 时必须重新阅读并遵守：

- `src-tauri/src/workflow/react/CONSTITUTION.md`

尤其要保证执行计划和审批详情使用结构化状态，不从 transcript 文本恢复。

## 6. 可用性检测

### 6.1 状态模型

不要只用 `command -v msb` 得出可用结论。建议状态：

```rust
enum SandboxAvailability {
    UnsupportedPlatform,
    NotInstalled,
    InstalledButUnhealthy,
    UnsupportedVersion,
    ReadyMissingImage,
    Ready,
}
```

状态还应携带：

- `msb_path`；
- `detected_version`；
- `doctor_summary`；
- `available_images`；
- `required_image`；
- `checked_at`；
- 面向 UI 的结构化 reason code。

### 6.2 检测顺序

1. 判断宿主平台和架构是否可能支持。
2. 从应用环境及完整登录 shell PATH 查找 `msb`。
3. 执行 `msb --version`，设置短超时。
4. 校验版本是否在支持范围。
5. 每次应用启动最多运行一次 `msb doctor`。
6. 执行 `msb image list --format json`。
7. 判断目标 profile 镜像是否存在。

Tauri GUI 进程的 PATH 可能不完整，macOS 上尤其需要复用项目已有的完整 shell PATH 发现逻辑。

### 6.3 缓存与刷新

- 检测结果在应用生命周期内缓存；
- 不要每条 Bash 命令都执行 `doctor`；
- 设置页提供“重新检测”；
- 用户安装 CLI 或拉取镜像后允许立即刷新；
- 执行前可做轻量版本/文件存在检查；
- 若执行时发现状态已失效，返回结构化错误，不静默降级。

### 6.4 普通用户提示

- Intel Mac：直接显示“不受 Microsandbox 支持”；
- Linux KVM 不可用：显示 KVM/权限诊断，不笼统显示“安装失败”；
- Windows WHP 未启用：说明需要启用虚拟化/WHP；
- CLI 缺失：链接到安装教程，但不自动执行在线脚本；
- 镜像缺失：显示体积、版本和用户主动下载操作；
- beta/版本不兼容：显示经过验证的版本范围。

## 7. Profile 与镜像设计

### 7.1 Profile 选择

第一阶段建议在 workflow 启动时根据项目特征选择 profile，而不是只看命令的第一个 token。复合命令可能同时需要多个工具链，例如：

```bash
pnpm build && cargo test
```

候选 profile：

| Profile | 典型项目特征 | 包含能力 |
| --- | --- | --- |
| `common` | 无明确 runtime | Bash、Git、curl、jq、rg、GNU 工具 |
| `python` | `pyproject.toml`、`requirements.txt` | common + Python + pip/uv |
| `node` | `package.json` | common + Node LTS + npm/corepack |
| `rust` | `Cargo.toml` | common + Rust stable + native build tools |
| `tauri_linux` | `src-tauri` + `package.json` | rust + node + Linux Tauri 依赖 |

自动优先级候选：

```text
tauri_linux > rust > node > python > common
```

实施前需验证混合项目、monorepo 和子目录项目。高级用户应能为项目固定 profile 或指定自定义 OCI 镜像。

仓库内配置只能提出运行需求，不能自动获得额外挂载、网络或秘密访问权限。可信权限配置应保存在 ChatSpeed 本地设置中。

### 7.2 镜像基线

虽然 Alpine 体积较小，但存在以下便利性问题：

- BusyBox 与 GNU 参数和行为差异；
- musl 与 glibc 兼容差异；
- Node 原生模块兼容问题；
- Rust/Tauri 系统依赖排查成本更高。

为了让 Agent 生成的常见命令更稳定，初期优先考虑统一使用 Debian slim。若 `common` 最终选择 Alpine，应显式安装 Bash、GNU coreutils、findutils、sed、gawk 等工具，并建立命令兼容测试。

镜像候选命名：

```text
ghcr.io/chatspeed-ai/sandbox-common:2026.07
ghcr.io/chatspeed-ai/sandbox-python:2026.07
ghcr.io/chatspeed-ai/sandbox-node:2026.07
ghcr.io/chatspeed-ai/sandbox-rust:2026.07
ghcr.io/chatspeed-ai/sandbox-tauri-linux:2026.07
```

镜像发布要求：

- 同时发布 `linux/amd64` 和 `linux/arm64`；
- 固定基础镜像和主要工具链版本；
- 不以浮动 `latest` 作为 ChatSpeed 默认；
- UI 可显示 tag，实际运行尽量锁定 digest；
- 发布 Dockerfile；
- 生成 SBOM；
- 对镜像签名并验证来源；
- 定期重建安全更新；
- 默认使用非 root 用户；
- 为每个镜像维护 capability manifest。

### 7.3 自定义环境

给高级用户保留以下能力：

- 自定义 profile 名称；
- 自定义 OCI image reference/digest；
- 自定义 CPU、内存、磁盘和超时；
- 自定义网络策略；
- 经审批增加额外挂载；
- 选择默认 profile 或为项目覆盖。

自定义配置必须经过后端校验，不能允许仓库中的不可信文件直接挂载宿主任意路径或注入宿主 secrets。

## 8. CLI 调用设计

### 8.1 正确理解 `msb run`

Microsandbox v0.6.6 的形式是：

```text
msb run [OPTIONS] [IMAGE] [-- <COMMAND>...]
```

因此 `msb run python ...` 中的 `python` 是 image reference，不是 ChatSpeed 内部 profile 名称。ChatSpeed 应先将 profile 映射为确切镜像。

示意命令：

```bash
msb run \
  --quiet \
  --no-tty \
  --security restricted \
  --no-net \
  --volume "/host/project:/workspace" \
  --workdir /workspace \
  "ghcr.io/chatspeed-ai/sandbox-python:2026.07" \
  -- sh -lc "python script.py"
```

实际 Rust 实现禁止拼接并交给宿主 shell。所有参数必须通过 `Command::args` 单独传递：

```rust
Command::new(msb_path)
    .args([
        "run",
        "--quiet",
        "--no-tty",
        "--security",
        "restricted",
        "--no-net",
        "--volume",
        mount_spec,
        "--workdir",
        "/workspace",
        image,
        "--",
        "sh",
        "-lc",
        original_command,
    ]);
```

原始命令只作为 guest `sh -lc` 的一个参数，不参与宿主命令行重新解析。

### 8.2 超时与清理

同时设置三层约束：

1. Microsandbox guest command timeout；
2. `--max-duration`，确保 VM 最终退出；
3. ChatSpeed Tokio 外层 timeout。

还应设置：

- CPU 数量；
- 内存上限；
- `nproc`、`nofile` 等 rlimit；
- OCI writable upper 大小；
- 最大连接数；
- 合理的 stdout/stderr 上限。

取消、应用退出和 workflow stop 时，要验证杀死 `msb` CLI 后不会留下 VM。`--max-duration` 作为最后保险，不能替代正常清理。

### 8.3 流式输出

`msb run` 的 stdout/stderr 可通过子进程管道读取，现有流式输出、ANSI 清理、截断和 frontend build stderr 处理原则上可以复用。

需要验证：

- guest exit code 是否稳定映射为 `msb` exit code；
- CLI 自身错误和 guest stderr 如何区分；
- 超时和信号退出的状态码；
- CLI 日志是否污染 Agent 命令输出；
- Windows 下管道与进程树终止行为。

## 9. 文件系统与路径

### 9.1 默认挂载

第一阶段只挂载当前项目根目录：

```text
host project root -> /workspace
```

- Planning 或纯检查场景可优先只读挂载；
- 需要构建或生成文件时使用读写挂载；
- 读写挂载前仍遵循现有审批和路径策略；
- 额外 authorized roots 不应自动全部挂载；
- Skill 目录如需执行，应单独建模和审核。

### 9.2 跨平台路径语义

Windows 宿主上的模型可能生成 `C:\project`、PowerShell 或 CMD 语法，而 guest 是 Linux。启用沙箱时，Agent 的运行环境信息必须改为类似：

```text
Execution OS: Linux sandbox
Shell: /bin/bash
Working directory: /workspace
Host OS: Windows
```

建议引导模型使用相对路径。不要通过字符串替换把 Windows 路径或宿主绝对路径改写为 `/workspace`，否则容易破坏引号、脚本和安全判断。

PathGuard 后续需要同时理解：

- 宿主授权路径；
- guest 中的受控映射路径；
- 二者之间由后端生成、不可由模型伪造的映射。

### 9.3 构建缓存和产物污染

不要直接把宿主缓存假定为跨平台缓存：

- Linux guest 生成的 `node_modules` 原生模块可能破坏 macOS/Windows 本机环境；
- Rust `target` 包含目标平台产物；
- Python virtualenv 不可跨平台复用；
- 包管理器缓存可能包含架构和 ABI 相关内容。

后续应使用 Microsandbox named volume，并至少按以下维度隔离：

```text
profile + image digest + guest architecture + project identity
```

第一阶段可以不做持久缓存，以正确性优先。

## 10. 网络与 Secrets

### 10.1 网络默认策略

不要依赖不同 Microsandbox 版本的隐式网络默认值。ChatSpeed 应显式指定：

- 默认 `--no-net`；
- 请求网络的命令进入审核；
- 审批详情显示网络模式；
- 支持 `none`、`public`、`allowlist` 三种结构化策略；
- 未来允许用户为项目配置域名白名单；
- 默认禁止访问宿主私有网络和 localhost 服务；
- 开端口或 ingress 必须单独审核。

包安装和 `git fetch` 需要网络，但 `npm test`、构建脚本或下载后的二进制同样可能联网。不能只按顶层命令名断言整个执行过程安全。

### 10.2 环境变量与凭据

- guest 默认不继承宿主环境变量；
- 不自动传递 API Key、代理凭据、Git 凭据；
- 本机降级执行目前会继承宿主环境，属于需要后续单独加固的风险；
- 第一阶段不自动挂载 SSH agent、`.npmrc` 或 credential store；
- Microsandbox 的 host-bound secret 能力可作为后续专题研究，不进入首版。

任何 secret 注入能力必须绑定允许访问的目标域名，并在 UI 中明确展示。

## 11. 审批与用户体验

### 11.1 审批显示

用户应审核原始命令和真实执行环境，不需要审核内部的 `msb` 包装参数。审批详情建议显示：

```text
Command: pnpm test
Execution: Local computer
Reason: Required Node sandbox image is not installed
Sandbox profile: node
Network: Host network
Workspace access: Read/write
```

沙箱内执行可显示：

```text
Execution: Microsandbox
Image: chatspeed sandbox-node 2026.07
Network: Disabled
Workspace access: Read/write
Limits: 2 CPU / 2 GiB / 10 min
```

### 11.2 缺失镜像

普通用户路径：

1. 提示所需 profile 镜像未下载；
2. 展示预计大小、版本和用途；
3. 提供用户主动触发的下载；
4. 下载失败时保留错误和重试；
5. 在 `auto` 模式下允许选择经审核的本机执行；
6. 在 `sandbox_only` 模式下阻止本机执行。

不要在第一次 Bash 调用时静默下载大型镜像。

### 11.3 设置页建议

设置页至少需要：

- 执行模式；
- Microsandbox 检测状态；
- CLI 路径与版本；
- `doctor` 结果摘要；
- 已安装/缺失镜像；
- 重新检测；
- 安装教程入口；
- 默认 profile；
- 默认网络策略；
- 默认资源限制；
- 项目覆盖配置；
- beta 与平台限制提示。

状态和错误文案必须进入 i18n，各语言 locale 结构保持一致并排序。

## 12. 生命周期策略

### 12.1 第一阶段：每条命令一个临时 VM

推荐第一阶段使用一次性 `msb run`：

- 与当前每次 Bash 调用不保留 shell 状态的语义一致；
- 执行结束自动清理；
- 不引入 sandbox session 恢复；
- 不需要处理 workflow 暂停、热恢复和 child workflow 实例所有权；
- 容易与现有流式输出结合。

### 12.2 后续阶段：每个 workflow 一个沙箱

只有在明确需要以下能力时再考虑 detached sandbox：

- 长期运行的开发服务器；
- 多命令复用同一环境；
- 增量编译；
- 持久缓存；
- 端口转发和预览。

这会引入：

- sandbox 与 workflow 的结构化所有权；
- stop/pause/resume/recovery；
- 崩溃和应用退出后的清理；
- child workflow 并发隔离；
- stale sandbox 回收；
- 数据库中的生命周期状态。

根据 Workflow Constitution，这些状态必须以后端结构化状态为权威，不能依赖 transcript 或实例名称推断。

## 13. 分阶段实施建议

### Phase 0：重新确认外部状态

- 核对 Microsandbox 最新稳定版本和 beta 状态；
- 核对 CLI 是否仍兼容本文件中的参数；
- 检查公开安全公告；
- 确定最低/最高支持版本；
- 明确 ChatSpeed 威胁模型和本机降级策略。

### Phase 1：后端执行边界

- 实现 `SandboxDetector`；
- 实现 `ShellExecutionBackend` 和 resolver；
- 抽取统一的 streaming process runner；
- 使用官方测试镜像验证 `msb run`；
- 本机执行增加不可绕过的 review floor；
- 沙箱失败返回结构化错误，不自动降级；
- 将实际 backend 写入审批、tool result 和日志。

### Phase 2：设置与状态 UI

- 增加 `auto`、`sandbox_only`、`host_only`；
- 展示平台、版本、doctor 和镜像状态；
- 增加重新检测；
- 增加结构化本机降级审批信息；
- 补齐 i18n；
- 编写安装与排错教程。

### Phase 3：官方 ChatSpeed 镜像

- 发布 `common`、`python`、`node`、`rust`；
- 建立 multi-arch CI；
- 生成 SBOM 和签名；
- 固定 digest；
- 实现 profile 自动识别；
- 提供显式镜像下载与状态管理。

### Phase 4：Tauri Linux 与高级配置

- 发布 `tauri-linux`；
- 明确原生打包限制；
- 支持项目 profile 覆盖；
- 支持自定义镜像和资源限制；
- 评估 named volume 缓存；
- 评估受控网络 allowlist 和 secret 能力。

### Phase 5：持久 workflow sandbox（可选）

- 仅在一次性 VM 的性能或长任务限制成为真实问题后开展；
- 需要独立设计文档和 Workflow Constitution 对照审查。

## 14. 测试计划

### 14.1 单元测试

- 平台和架构能力判断；
- `msb --version` 解析；
- 版本范围判断；
- `doctor` 成功、失败和超时；
- image list JSON 解析；
- profile 选择和用户覆盖；
- backend resolution；
- host review floor 不被 allowlist/Full 绕过；
- sandbox failure 不自动切换 host；
- mount 和 network 参数构建；
- reason code 与审批 metadata。

### 14.2 集成测试

- 无 `msb`；
- 不支持的平台；
- CLI 存在但 `doctor` 失败；
- CLI 版本过低/过高；
- 镜像缺失；
- 沙箱只读工作区；
- 沙箱读写工作区；
- `--no-net` 生效；
- CPU/内存/时间限制；
- stdout/stderr 交错流式输出；
- guest non-zero exit；
- `msb` 自身失败；
- 用户取消和 workflow stop；
- 应用异常退出后无长期残留；
- 路径中包含空格和非 ASCII 字符；
- macOS Apple Silicon、Linux KVM、Windows WHP。

### 14.3 安全回归测试

- 试图挂载 workspace 外路径；
- 试图使用 `~`、环境变量和 symlink 越界；
- 沙箱内删除 workspace 根目录；
- 试图访问主目录、SSH agent、Docker socket；
- 通过故意触发沙箱失败诱导本机执行；
- 通过 shell allowlist 绕过本机审核；
- 通过 Full 模式绕过本机审核；
- 审批后更换 backend/image/mount/network；
- 网络禁用时访问 public、private、localhost；
- command、image、mount 参数中的注入字符。

建议提供 fake `msb` executable 进行稳定的适配测试，不让大多数 CI 用例依赖真实虚拟化。真实 microVM 测试放到具备相应能力的专用 runner。

## 15. 首版验收标准

首版至少满足：

- 支持状态能够区分平台不支持、未安装、不健康、版本不兼容、缺镜像和就绪；
- 默认模式下沙箱就绪时优先使用沙箱；
- 本机 Bash 无论原 shell policy 和审批等级如何都进入人工审核；
- 用户能看到真实执行位置和降级原因；
- 沙箱执行失败不会自动在本机重试；
- 审批绑定原始命令和稳定执行计划；
- 只挂载当前授权项目目录；
- 默认不向 guest 传递宿主 secrets；
- 默认显式禁用网络；
- 超时、取消和应用退出不会留下无限期运行的实例；
- 流式输出和现有 Bash tool result 行为不回归；
- macOS Apple Silicon、Linux KVM、Windows WHP 至少完成一次真实验证；
- Intel Mac 和缺少虚拟化能力的设备有清楚的兼容路径；
- 所有新增用户文案进入 i18n。

## 16. 恢复实施前的开放问题

1. 本机 review floor 是否在 Full 模式下绝对不可关闭，还是提供独立的专家级开关？
2. `auto` 模式下，镜像缺失是否立即进入本机审批，还是先询问用户下载镜像？
3. 默认网络是否坚持 `none`，包管理命令如何提供低摩擦的临时网络授权？
4. workspace 默认读写，还是根据命令/阶段选择只读与读写？
5. 如何保护 workspace 中的 `.env`、私钥和其他敏感文件？
6. 项目 profile 是 workflow 固定，还是允许单条命令显式选择？
7. 混合 monorepo 如何选择最小但足够的镜像？
8. Linux guest 写入的 `node_modules`、`target` 和 virtualenv 如何与宿主隔离？
9. 自定义 OCI 镜像是否需要签名要求或首次使用审核？
10. 镜像下载、更新和删除由 ChatSpeed UI 管理到什么程度？
11. 是否需要给 read-only Bash 提供只读 workspace profile，以进一步减少破坏面？
12. 本机执行时是否同步引入环境变量清理和进程隔离？
13. 原生 Tauri 构建的识别规则如何定义，避免错误送入 Linux guest？
14. 是否需要支持用户已有的其他沙箱后端，还是仅保留内部替换边界？

## 17. 当前结论

Microsandbox 方向值得继续，但应以“可检测、可降级、失败不越权的可选安全层”落地，而不是作为所有设备都可用的唯一执行环境。

建议恢复任务时从 Phase 0 和 Phase 1 开始，先完成后端执行计划、强制本机审核以及失败不降级三个基础不变量，再进入镜像和 UI 工作。不要先构建完整镜像体系后再补审批边界。
