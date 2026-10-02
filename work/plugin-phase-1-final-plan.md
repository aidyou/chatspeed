# ChatSpeed Runtime 与插件开发分期方案

> 状态：方案确认，进入 runtime 独立工作阶段前的基线记录
> 日期：2026-10-01
> 范围：runtime 独立运行、客户端接入与后续插件能力

## 1. 当前版本的总体目标

当前 `feature/plugin` worktree 的总体目标不是继续维护 headless 或自我改进运行链路，而是建立 ChatSpeed 的客户端与运行核心边界，并在此基础上实现插件能力：

```text
Tauri Client ─────┐
                  │ HTTP/JSON + SSE
cscli Helper ─────┼────> ChatSpeed Runtime
                  │       ├── workflow
Future Clients ───┘       ├── model and session state
                          ├── MCP and Skills
                          ├── permissions and approvals
                          ├── persistence
                          └── capability facade

Plugin ─────────────────> Runtime capability facade
```

各部分职责如下：

- **Runtime**：ChatSpeed 的独立运行核心，负责后台任务、状态、持久化、模型、workflow、MCP、Skills、权限和能力治理；
- **Tauri**：桌面客户端，负责 UI、用户交互、窗口、WebView 以及桌面专属能力，不负责直接实现后台业务；
- **`cscli`**：面向 AI 和自动化调用的 runtime helper，通过与其他客户端相同的 HTTP/JSON + SSE 协议调用 ChatSpeed；`help` 只是其中一个离线子命令，不代表 CLI 的整体职责；
- **未来客户端**：即使当前实际客户端只有 Tauri，协议和生命周期仍按多客户端设计；
- **Plugin**：第二期接入 runtime capability facade，不直接依赖 Tauri 或 runtime 内部模块。

当前版本不再把 headless 作为客户端、运行形态、兼容性目标或验收范围。headless 和自我改进模块已经通过 `with-headless` tag 固化，并从当前主线移除。

## 2. 两期开发范围

### 2.1 第一期：Runtime 独立运行时

第一期的目标是将当前由 Tauri 进程承载的后台运行能力抽离为独立 runtime，并让现有客户端通过统一协议接入。

第一期交付范围：

1. **Runtime 独立进程**
   - 抽离 workflow、模型调用、会话状态、持久化、MCP、Skills、权限和其他后台能力；
   - runtime 独立启动、运行和退出，不依赖 Tauri 窗口或 WebView；
   - Tauri 不直接访问 runtime 的数据库、内部服务或后台模块。

2. **统一 HTTP/JSON + SSE 协议**
   - runtime 通过 loopback HTTP 提供请求/响应接口；
   - 通过 SSE 提供流式结果和运行事件；
   - discovery、认证、版本、错误、取消、超时和兼容性边界由 runtime 统一管理；
   - 现有 `/control/v1` 作为协议演进基础，逐步成为 runtime 的稳定客户端边界，而不再只是 Tauri 内部的 workflow 控制面。

3. **Tauri 客户端化**
   - Tauri 启动时自动发现或启动 runtime；
   - Tauri 通过 HTTP/JSON + SSE 使用 runtime 能力；
   - Tauri 只保留 UI、窗口、用户交互和客户端专属能力；
   - 用户审批等需要桌面交互的流程，通过客户端与 runtime 的协议协作完成。

4. **`cscli` helper 化**
   - `cscli` 作为 runtime 的 HTTP/SSE 客户端；
   - 面向 AI、脚本和自动化提供 workflow、agent、capability、Skill、MCP、诊断等入口；
   - 不直接打开数据库、不直接启动 workflow、不直接管理 MCP 进程，也不复制 runtime 的权限和审批逻辑；
   - 提供 human、JSON 和 JSONL 等适合人类与 AI 消费的输出格式。

5. **多客户端生命周期**
   - 第一个客户端启动时发现并启动 runtime；
   - 后续客户端连接已有 runtime；
   - 每个客户端通过注册和租约/heartbeat 表明存活状态；
   - 客户端正常退出时主动注销，崩溃时由租约超时清理；
   - 没有活跃客户端后，runtime 经过可配置的 idle grace period 自动退出；
   - 多个客户端并发启动时，通过 discovery、锁和 ready handshake 保证 runtime 单实例启动。

6. **客户端专属 Web 能力边界**
   - runtime 不依赖 Tauri/Wry/WebView；
   - 第一期由 Tauri 客户端提供内置 Web MCP 服务，承载现有依赖 WebView 的网页工具；
   - runtime 通过受控的客户端能力/MCP 边界调用该服务，并继续服从权限、审批和审计；
   - 其他客户端暂不要求实现 Web 工具，但协议应允许未来客户端声明和提供自己的 Web 能力。

第一期不包含插件安装、插件进程管理、插件 SDK 或插件 capability contract 的最终设计；第一期只建立能够承载这些能力的 runtime 边界。

### 2.2 第二期：插件能力

第二期在第一期 runtime 独立运行和客户端协议稳定后，实现插件到 runtime 的单向能力调用：

```text
Plugin
  -> runtime plugin capability boundary
  -> approved ChatSpeed capability
  -> result / stream / error
  -> Plugin
```

第二期交付范围：

1. 明确插件运行形态、宿主边界和插件到 runtime 的接入方式；
2. 定义一期最小 capability 清单；
3. 为每项 capability 定义名称、版本、请求、响应、流式、取消、超时和结构化错误协议；
4. 设计 capability facade、版本策略、能力发现和能力降级；
5. 设计插件身份、能力白名单、权限和用户审批模型；
6. 设计插件生命周期、崩溃恢复、资源回收和运行状态；
7. 明确插件与 MCP、Skills、CLI、Tauri 客户端能力的关系；
8. 最后确定 SDK、示例插件和测试矩阵。

第二期不新增一套插件专用工具扩展体系。插件需要使用外部工具时，优先复用现有 MCP 的安装、发现、配置、启停、权限和调用边界。

## 3. 一期架构不变量

### 3.1 Runtime 是唯一后台运行核心

所有客户端都通过 runtime 协议访问 ChatSpeed 后台能力。客户端不得复制以下逻辑：

- workflow 执行内核；
- 会话和任务状态管理；
- 数据库和持久化实现；
- MCP 客户端管理；
- Skills 管理；
- 权限、审批和能力治理；
- 模型调用和流式响应的后台协调。

客户端可以提供客户端专属能力，但必须通过明确的客户端服务边界暴露，不能反向成为 runtime 对 Tauri 实现细节的直接依赖。

### 3.2 所有客户端使用统一协议

Tauri、`cscli` 和未来其他客户端通过相同的 HTTP/JSON + SSE 协议接入 runtime。协议至少需要统一处理：

- runtime discovery 和实例身份；
- 客户端注册、heartbeat、注销和租约；
- 认证和本机访问边界；
- 请求、响应、流式事件和事件恢复；
- 超时、取消和资源回收；
- 结构化错误；
- 协议版本和能力版本；
- runtime 启动中、就绪、不可用和退出状态。

### 3.3 Runtime 不绑定 Tauri

以下内容默认属于客户端能力，不得直接成为 runtime 的必需依赖：

- Tauri、GTK、Wry 或桌面窗口对象；
- WebView；
- 剪贴板、通知和桌面快捷键；
- UI 审批界面。

Tauri 可以通过客户端内置 MCP 或其他明确的客户端服务提供 WebView 等能力。runtime 只依赖抽象的、经过权限治理的客户端能力协议。

## 4. 第二期插件不变量

### 4.1 插件不是新的主 Agent 工具面

插件调用 runtime 能力，不等于插件获得 ChatSpeed 的全部内部工具。以下内容默认不自动暴露：

- 内部 workflow 状态机；
- 数据库实现细节；
- Tauri、GTK、Wry 或桌面窗口对象；
- 未声明的内部工具注册表；
- 其他插件的私有状态；
- 任意文件、进程或网络权限。

插件只能依赖经过筛选、版本化和权限检查的 capability facade。

### 4.2 能力由 runtime 统一治理

所有插件调用都应经过 runtime 的统一边界，以便集中处理：

- 能力白名单；
- 参数校验；
- 权限与审批；
- workspace/path 安全约束；
- 超时、取消和资源回收；
- 结构化错误；
- 版本兼容性和能力降级。

### 4.3 不为插件复制执行内核

插件应复用 runtime 的能力和生命周期机制，不在插件侧重新实现：

- 一套 workflow runtime；
- 一套审批系统；
- 一套工具扩展系统；
- 一套 MCP 客户端管理；
- 一套会话或流式响应协议。

只有在现有 runtime 边界无法满足明确需求时，才评估增加 runtime capability；不以插件需求为由复制基础设施。

## 5. 当前基线与边界

- `with-headless` tag 保存了此前包含 headless 和自我改进模块的版本；
- 当前 `feature/plugin` worktree 基于移除 headless 和自我改进模块后的主线；
- 当前分支基线为 `1ed6b717`；
- runtime、contracts 和 CLI 的初始拆分来自 `94e18674`，但 runtime 仍需继续从 Tauri 主 crate 中抽离；
- 当前已有独立 `contracts` workspace crate、`cscli` workspace crate 和 loopback `/control/v1` HTTP/JSON + SSE 控制面；
- 当前已有的 `db/plugin.rs` 只视为历史插件元数据持久化基础，不视为已完成的插件运行框架；
- `src-tauri/assets` 不纳入本次 runtime/插件分期设计的实现基线，除非具体任务明确涉及；
- 第一期的当前工作重点是 runtime 独立运行，不提前实现第二期插件协议或插件 SDK。

## 6. 后续执行顺序

当前进入第一期 runtime 独立工作，按以下顺序推进：

1. 盘点现有 runtime 后台职责与 Tauri 依赖，划定独立 runtime crate/process 的边界；
2. 确定 runtime、Tauri 和 `cscli` 的 workspace/package 结构；
3. 将现有 application service、control plane 和必要 contracts 迁移到 runtime；
4. 实现 runtime 的独立启动、discovery、认证和 ready handshake；
5. 实现多客户端注册、heartbeat、注销和 idle shutdown；
6. 将 Tauri 改为 runtime client，并保留 Tauri 专属 Web MCP 能力；
7. 将 `cscli` 对齐为 runtime helper，验证 workflow、capability 和流式调用；
8. 建立第一期的进程、协议、生命周期和客户端集成测试矩阵；
9. 第一期验收完成后，再进入第二期插件 capability 设计与实现。

在第一期完成前，不把 runtime 内部模块直接固化为公共插件 API，不新增独立工具扩展体系，也不重新引入 headless 或自我改进运行链路。
