# ChatSpeed Workflow Engine - Current State Summary (Industrial ReAct Edition)

## 1. 核心哲学：工具驱动 (Tool-Driven)
- **强制工具调用**: LLM 的每一轮输出必须包含至少一个工具调用。若无工具调用，系统会自动生成错误观察并强制 AI 重写，确保任务不中断。
- **系统提示词强制注入**: 实现了 `CORE_SYSTEM_PROMPT`，确立了“一切皆工具”的法律级约束。

## 2. 工业级安全审计 (Gemini CLI Clone)
- **Shell 策略引擎**: 实现了 `ShellPolicyEngine`，支持多指令拆分审计。
- **重定向拦截**: 自动检测 `>` 和 `<` 操作，并对目标路径执行强制边界校验。
- **去混淆技术**: 自动展开 `$ENV` 变量、波浪号 `~`，并执行 Token 去引号处理，防止字符串伪装攻击。
- **递归嵌套审计**: 支持对 `$(...)` 和 `` `...` `` 等嵌套指令的无限递归安全扫描。

## 3. 增强型环境感知 (Environment Awareness)
- **动态注入**: 在每轮对话顶部注入 PWD、Git 状态、授权目录、操作系统版本及 Shell 类型。
- **路径卫士 (PathGuard)**: 支持对虚空路径（尚不存在的文件）进行递归父目录校验，严禁逃逸授权根目录。

## 4. 专业化多代理与任务管理
- **Task 编排**: 全量复刻 Claude Code 的 `task` 工具流，支持后台并行执行与结果回收。
- **Todo 管理**: 实现 `todo_create`, `todo_list`, `todo_update`, `todo_get`，用于会话内进度的结构化固化。
- **模型路由**: 支持 Programming, Writing, Browsing, Vision, Planning 专属模型，并具备智能 Fallback 到 Act 模型的机制。

## 5. 交互与持久化
- **Pending 消息队列**: 支持在执行期间接收用户消息，并在周期结束时插队进入上下文。
- **统一模型字段**: 数据库 `v5` 迁移已支持 `models` JSON 字段，实现配置的灵活扩展。

## 6. 后续待办 (Phase 6)
- **Tauri Commands 封装**: 将 Rust 引擎暴露给 Vue 前端。
- **细粒度 Shell 白名单**: 建议在 Agent 配置中增加 `shell_whitelist` 字段，允许特定安全命令跳过人工审批。

**Status**: Core Engine is Locked and Verified.
