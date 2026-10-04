# Runtime 拆分交接报告

> 用途：新话题启动时的上下文恢复。本文记录当前工作区提交前后的架构状态、验证证据和下一步重点，不替代源码与测试。
>
> 仓库：`/home/xc/dev/rust/chatspeed-plugin`
> 分支：`feature/plugin`
> 记录日期：2026-10-04

## 1. 当前目标

本轮工作是把 ChatSpeed 的后台运行能力拆分为独立 runtime，并让 Tauri desktop 与 cscli 成为客户端：

```text
user -> tauri desktop -> runtime -> workflow / MCP / chat / ccproxy
user -> tauri desktop -> runtime -> workflow -> cscli -> MCP / skill / automation helper
```

职责边界：

1. `chatspeed-runtime` 是独立、持续维护、权威的运行服务中心，不依赖 Tauri desktop 或 cscli；
2. Tauri desktop 负责用户交互和展示，并临时提供 WebView-backed `web_fetch` / `web_search` MCP 服务；
3. `cscli` 是 AI/helper 的命令行入口，只通过 runtime control plane 工作，不拥有数据库、workflow executor 或服务状态。

## 2. 已完成的实现

### Runtime

- 新增独立 `src-tauri/runtime` binary/library，使用 loopback `/control/v1` HTTP/JSON + SSE 控制面。
- runtime 负责数据库、单实例锁、migration、workflow、chat/model、MCP capability、automation、ccproxy、terminal 和 background task 生命周期。
- discovery 只在 owner、数据库、工具注册、background 和 control plane 准备好后发布；启动失败 fail closed。
- runtime 不依赖 `tauri`、`wry`、`gtk` 或 `cscli`。
- built-in agents、Models.dev catalog、terminal PTY 和 capability recovery 归 runtime owner 管理。

### Tauri desktop

- Tauri 通过 `RuntimeSupervisor` 连接或启动 runtime，使用 lease/heartbeat，不再作为实际 runtime state owner。
- workflow、MCP、chat/model、automation、ccproxy 等通过 control-plane adapter 请求 runtime。
- desktop-only `runtime_web_mcp_provider.rs` 使用 `rmcp` 的 streamable HTTP server，绑定 `127.0.0.1:0/mcp`。
- provider 只允许两个固定工具：`web_fetch`、`web_search`；使用内存 proof token、Origin 校验和 lease 绑定。
- desktop 启动后自动向 runtime 注册 provider；runtime 通过 ToolManager 的正常 MCP registry 调用它，而不是 native client-bridge tool。
- provider/lease 断开、过期和 shutdown 会清理注册；runtime 未注册 provider 时不暴露 web 工具。

### cscli

- `src-tauri/cli` 是纯 HTTP/SSE client，读取 discovery、注册 lease、调用 runtime control plane、渲染输出。
- `cscli skill/mcp/automation/workflow` 不打开数据库、不创建 executor、不实现第二套服务逻辑。
- runtime workflow 的 shell tool 可以受控调用 bundled `cscli`；cscli 再回到同一个 runtime control plane，形成 helper 链路但不改变 runtime ownership。

### Web MCP / legacy bridge

- production web execution 已从 `ClientBridgeRegistry::enqueue` 移除。
- legacy client bridge 代码仍保留作兼容/测试边界，但不挂载生产路由，也不注册为生产 web native tool。
- provider 仍只提供固定 typed allowlist，不支持任意 endpoint、任意 tool 或多 desktop 隐式调度。

### Models.dev 迁移回归修复

修复了 runtime-owned `model_catalog_engine` 在迁移后暴露的三个问题：

- 没有对应 modality 时不再错误写入 `Some(false)`，避免覆盖原 catalog 结果；
- Models.dev 的 boolean `temperature` 不再覆盖已有 catalog 推荐温度；
- 无 provider 上下文时不再依赖 `HashMap` 任意顺序，稳定排序并优先选择有 pricing 的模型。

## 3. 关键源码位置

- 独立 runtime：`src-tauri/runtime/`
- runtime backend owner：`src-tauri/runtime-backend/src/owner.rs`
- runtime backend catalog：`src-tauri/runtime-backend/src/ai/model_catalog_engine.rs`、`model_catalog_service.rs`
- runtime client：`src-tauri/runtime-client/`
- contracts：`src-tauri/contracts/`
- desktop supervisor：`src-tauri/src/runtime_client.rs`
- desktop Web MCP provider：`src-tauri/src/runtime_web_mcp_provider.rs`
- runtime Web MCP registry：`src-tauri/runtime-backend/src/web_provider.rs`
- canonical MCP ToolManager：`src-tauri/src/tools/tool_manager.rs`
- CLI：`src-tauri/cli/`
- workflow shell -> cscli：`src-tauri/src/tools/shell.rs`
- runtime control plane：`src-tauri/src/workflow/react/client/http/server.rs` 和 `web_mcp_commands.rs`

## 4. 已验证证据

此前及本轮聚焦验证：

- `cargo test -p chatspeed-runtime-backend model_catalog --lib`：23 passed；
- `cargo test -p chatspeed-runtime-backend ai::chat::list_models::tests --lib`：3 passed；
- `cargo test -p chatspeed-runtime --lib`：25 passed；
- `cargo test -p chatspeed-cli --bin cscli`：57 passed；
- runtime Web MCP provider 使用 rmcp loopback server、fixed schema、registration、lease cleanup 的源码路径已核对；
- runtime 在 provider 未注册时不暴露 `web_fetch` / `web_search` 的测试已通过；
- terminal session authorization 已按 `(client_id, lease_id)` 隔离；
- production web path 不再进入 `ClientBridgeRegistry::enqueue`。

## 5. 当前用户反馈：`pnpm tauri` 出现大量 unused warning

用户观察到运行 `pnpm tauri` 时有无尽 unused warning。这个现象与本轮拆分目标相符，优先怀疑不是 runtime 架构本身失败，而是 **同一批 canonical shared source 在 desktop feature 下仍被编译，但其中大量 runtime-only symbols、tests、imports 或模块没有被正确切到 `cfg(feature = "desktop")` / `cfg(not(feature = "desktop"))` 边界**。

此前迁移过程已经大量增加 cfg，但当前仍有以下风险：

1. shared source 同时被 desktop 和 runtime-backend 通过 `#[path]` 编译，模块级 cfg 不完整会产生 desktop-only 或 runtime-only dead/unused items；
2. `src-tauri/src/tools/mod.rs`、workflow、MCP、sensitive、ccproxy、commands 等文件中的 imports 和 tests 可能只在 runtime 使用，但仍进入 Tauri crate；
3. `#[cfg(test)]` 与 `#[cfg(all(test, not(feature = "desktop")))]` 的边界可能不一致；
4. Tauri 构建默认 feature/依赖图仍需确认，特别是 desktop crate 是否意外编译了 runtime backend-only code；
5. 不能用全局 `#![allow(unused)]` 掩盖问题，应按 warning 文件/符号逐项判断 ownership，再补精确 cfg、移动 import 或拆分模块。

### 下一话题建议的第一步

在不继续扩大架构范围的前提下：

1. 运行 `pnpm tauri` 或等价的 `cargo check`，把完整 warning 输出保存到临时文件；
2. 按 warning 类型聚合：`unused import`、`dead_code`、`unused variable`、`unexpected cfg`、feature/依赖问题；
3. 先检查 desktop crate 的 `Cargo.toml` features 与 `src-tauri/src/lib.rs` module declarations；
4. 对每一组 warning 追踪对应符号在 runtime、desktop、CLI 的实际调用方；
5. 仅对确认属于拆分边界的 warning 增加精确 cfg 或调整模块归属；不删除仍由 runtime 使用的代码，不恢复 legacy bridge 生产路径；
6. 每组修复后分别运行 desktop `cargo check`、runtime backend/runtime tests 和 cscli tests。

建议保留一份 warning 基线，例如：

```bash
pnpm tauri 2> /tmp/chatspeed-tauri-warnings.log
rg "warning:|unused|dead_code|unused_imports" /tmp/chatspeed-tauri-warnings.log
```

实际命令应根据当前 pnpm script 和本机桌面环境调整；不要把 `/tmp` 日志提交到 git。

## 6. 已知限制

- 当前环境没有完成真实 Tauri WebView + 显示服务器下的页面抓取 smoke；rmcp/provider/control-plane/registry 边界已完成源码和 focused test 验证。
- 宽泛 `cargo test -p chatspeed-runtime-backend --lib` 仍包含与本轮无关的基线/环境失败；本轮只修复并验证 Models.dev 和 runtime split 直接相关的失败。
- `cargo fmt --all -- --check` 会被仓库其他共享源码的既有格式差异阻断；本轮目标文件已做 focused rustfmt 检查。
- 编译目录、target 输出、临时日志和缓存不应提交；本次提交只应包含源码、配置、测试和本交接报告。

## 7. Git 交接

本报告与当前 runtime 拆分源码一起提交。新话题开始时先执行：

```bash
git status --short
git log -1 --oneline
git show --stat --oneline HEAD
```

然后从第 5 节的 Tauri warning baseline 开始排查。
