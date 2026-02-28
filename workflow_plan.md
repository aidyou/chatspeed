# ChatSpeed Agent Runtime 开发计划 (Rust 核心重构版)

## 0. 开发总指导原则 (Development Principles)
*   **模块化隔离 (Module Isolation)**：核心引擎、存储、工具、网关必须作为独立的 Rust 模块，通过 Trait (如 `Gateway`, `Tool`) 进行解耦。
*   **测试驱动 (TDD)**：每个 Phase 必须包含配套的测试用例。基础工具集必须通过 100% 的路径安全与审计测试。
*   **无猜测交付 (No-Guess Delivery)**：Phase 描述需达到中级开发者无需询问即可实现逻辑的详细程度。
*   **万物皆工具 (Everything is a Tool)**：除 Thinking 过程外，所有输出（包括回答用户、提问、结束任务）必须通过工具完成。
*   **可观测性优先 (Observability First)**：所有核心路径必须有日志、指标、追踪，便于问题诊断和性能优化。
*   **渐进式交付 (Incremental Delivery)**：每个 Phase 必须能独立部署和验证，降低集成风险。
*   **降级设计 (Graceful Degradation)**：关键路径必须有失败降级方案，确保系统在异常情况下仍能提供基本服务。
*   **安全边界明确 (Clear Security Boundaries)**：所有可能的危险操作必须明确标记和审批，防止安全漏洞。
*   **国际化**：所有输出给用户看到的信息都必须采用国际化语言输出（i18n），前期开发只要配置简体中文即可，其他的语言可以在后期统一翻译
*   **错误处理**：所有的错误统一通过 @src-tauri/src/error.rs 处理

## 1. 核心架构详述

### 1.1 数据持久化 (Storage Layer)
*   **Schema 来源**:
    *   `agents`: 见 `src-tauri/src/db/sql/migrations/v2.rs`。
    *   `workflows` & `workflow_messages`: 见 `v5.rs`。
*   **Schema 扩展需求**:
    *   `workflow_messages`: 增加 `step_type` (Enum: Think, Act, Observe) 和 `step_index` 字段。
    *   `workflows`: 增加 `allowed_paths` (JSON Array) 存储授权目录白名单。
*   **持久化策略**:
    *   **At-Once Persistence**: 每一个 Step 执行完毕后，立即将 `WorkflowMessage` 序列化并存入 SQLite。
    *   **Recovery**: 重启后，根据 `session_id` 加载消息流，重新填充 `WorkflowExecutor` 的上下文。

### 1.2 网关协议标准 (ChatSpeed Gateway Protocol v1)
为了支持多端接入（Tauri, WebSocket, HTTP 网关），定义一套中立的 JSON 消息标准。执行器通过 `Gateway` Trait 发送和接收这些消息。

#### 1.2.1 输出事件 (Output Events - JSON Payload)
*   **TextChunk**: `{ "type": "text", "session_id": "...", "content": "..." }`
    *   用于 `answer_user` 工具的流式输出内容提取。
*   **StateChanged**: `{ "type": "state", "session_id": "...", "state": "thinking|executing|paused|completed|error", "metadata": {} }`
*   **ConfirmationRequired**: `{ "type": "confirm", "session_id": "...", "request_id": "...", "action": "shell|fs_write", "payload": { "cmd": "...", "diff": "..." } }`
    *   触发安全审批 UI，包含执行详情或 Diff。

#### 1.2.2 外部指令 (Input Signals)
*   **UserInput**: `{ "type": "input", "session_id": "...", "content": "..." }`
*   **ConfirmationResponse**: `{ "type": "confirm_res", "request_id": "...", "approved": true, "reason": "..." }`
*   **SystemControl**: `{ "type": "control", "session_id": "...", "command": "stop|pause|resume" }`

#### 1.2.3 协议扩展性 (Protocol Extensibility)
基于现有的路由协议和消息格式设计。

*   **版本协商**:
    *   **协议版本号**: 在消息中包含 `protocol_version` 字段（如 `"protocol_version": "1.0"`）。
    *   **版本兼容性**: 客户端和服务端握手时协商共同支持的最高版本。
    *   **降级策略**: 若客户端不支持服务端最低版本，返回错误消息并断开连接。
*   **事件类型注册**:
    *   **扩展事件**: 支持自定义事件类型（如 `"type": "custom_progress"`），需在注册表中声明。
    *   **事件注册表**: 维护已知事件类型的全局注册表（`src-tauri/src/gateway/event_registry.rs`），包含事件名称、字段定义、验证规则。
    *   **向后兼容**: 新事件类型不影响旧客户端，旧客户端忽略未知事件类型。
*   **第三方实现验证**:
    *   **协议测试套件**: 提供标准化测试用例验证实现正确性（如消息格式、事件顺序、错误处理）。
    *   **兼容性认证**: 第三方 Gateway 实现需通过认证测试（位于 `tests/gateway_compatibility/`）。
    *   **文档规范**: 详细的协议文档和实现指南（`docs/gateway_protocol.md`）。

### 1.3 运行时层 (ReAct Executor) 详述
*   **核心模块**: `WorkflowExecutor`。独立于 Tauri `AppHandle`，通过 `Arc<dyn Gateway>` 通信。
*   **嵌套模式 (Nested ReAct)**:
    *   主代理通过 `spawn_agent` 工具启动子代理。
    *   每个子代理拥有独立的 `WorkflowExecutor` 实例。
    *   **上下文共享**: 子代理启动时，其 `System Prompt` 会由父代理自动注入“任务摘要”和“意图引导”。
*   **观察强化 (Observation Reinforcement)**:
    *   **结果评估器 (Result Evaluator)**: 在工具返回给 LLM 前，注入启发式提示。
    *   **错误纠正引导**: 若 `shell_execute` 权限不足，返回更具指导性的观察结果而非原始错误。
    *   **结构化摘要**: 对于复杂的工具返回（如网页抓取），执行器会先将其转换为 Markdown 摘要。

### 1.4 上下文压缩 (Context Compression) 详述
*   **策略**:
    *   **重要性筛选**: 标记核心指令、最新消息及关键结论为“永不压缩”。
    *   **语义压缩**: 当 Token 总量超过阈值时，将历史冗长的 `role='tool'` (Observation) 消息发送给轻量级模型。
    *   **Fact 提取**: 生成简短的陈述句（Fact）并替换原消息内容。

### 1.5 基础工具集 (Foundation Toolset) - 工业级复刻
参考 Claude Code 设计，工具集需具备处理大型工程的能力，所有工具需适配 `PathGuard` 安全校验。

#### 1.5.1 核心读写 (File I/O)
*   **`read_file(path, offset, limit)`**: 
    *   **职责**: 分页读取文本文件。
    *   **特性**: 默认限制 2000 行，返回带行号的内容（cat -n 格式）。
*   **`write_file(path, content)`**: 覆盖写入，自动备份。
*   **`edit_file(path, old_string, new_string)`**: 基于精确匹配的局部替换。校验不唯一则报错。
*   **`bash(command, timeout)`**: 别名 shell_execute。

#### 1.5.2 搜索与探索 (Search & Explore)
*   **`glob(pattern, path)`**: 模式匹配文件列表。
*   **`grep(pattern, path, glob, output_mode)`**: 高性能正则搜索，支持上下文。
*   **`list_dir(path, recursive)`**: 目录快照。

#### 1.5.3 调度与交互 (Orchestration & UI)
*   **`task(description, prompt, subagent_type)`**: 启动专业子代理。
*   **`ask_user(question, options)`**: 交互提问。
*   **`answer_user(text)`**: 流式回答。
*   **`finish_task(summary)`**: 任务结项。

#### 1.5.4 技能执行 (Skills)
*   **`skill_execute(name, args)`**: 调用技能插件。

### 1.6 安全审计增强 (Security Audit Enhancements)
基于现有的敏感信息过滤和工具管理扩展。

*   **敏感信息过滤**:
    *   **日志脱敏**: 扩展现有 `SENSITIVE_REGEX`，自动识别并替换日志中的 API Key、密码、Token 等敏感信息（如将 `sk-xxxx` 替换为 `sk-****`）。
    *   **工具参数脱敏**: 工具调用日志中的敏感参数（如文件路径中的用户名）自动脱敏后再记录。
    *   **返回值脱敏**: 工具返回值若包含敏感信息（如配置文件中的密码），自动过滤后再传递给 LLM。
*   **工具调用链审计**:
    *   **调用链追踪**: 为每个工具调用生成唯一 `call_id`，记录调用者（Agent ID）、被调工具、参数摘要、返回状态。
    *   **权限边界检查**: 检测跨权限边界的工具调用（如低权限 Agent 调用高权限工具），记录审计日志并拒绝执行。
    *   **异常检测**: 检测异常的工具调用模式（如短时间内大量文件读取、重复执行相同命令），触发安全预警。
*   **用户权限模型**:
    *   **Agent 权限级别**: 定义四级权限：`read-only`（仅读取文件）、`standard`（标准工具）、`elevated`（文件写入、Shell 执行）、`admin`（系统配置修改）。
    *   **工具权限映射**: 每个工具定义所需最低权限级别（如 `safe_read_file: read-only`，`shell_execute: elevated`）。
    *   **动态权限提升**: 临时提升权限执行特定操作，需用户确认并记录审计日志（如用户批准后临时提升到 `elevated` 执行 Shell 命令）。

#### 1.5.4 安全审计增强 (Security Auditing)
*   **敏感信息过滤**:
    *   日志脱敏: 使用正则表达式自动过滤日志中的 API Key、密码、Token 等敏感信息（基于现有 `SENSITIVE_REGEX` 实现）。
    *   工具参数脱敏: 工具调用日志中的敏感参数（如密码、密钥）自动替换为 `***REDACTED***`。
    *   返回值过滤: 工具返回值若包含敏感信息模式，在存储前自动脱敏。
*   **工具调用链审计**:
    *   调用链追踪: 为每个工具调用分配唯一 `call_id`，记录调用者 Agent ID、时间戳、参数摘要。
    *   权限边界检查: 检测跨权限边界的工具调用（如低权限 Agent 尝试调用高权限工具）。
    *   异常检测: 检测异常的工具调用模式（如短时间内大量文件删除操作）并触发安全告警。
*   **用户权限模型**:
    *   Agent 权限级别: `read-only`（只读工具）、`standard`（基础工具）、`elevated`（文件写入/Shell）、`admin`（无限制）。
    *   工具权限映射: 每个工具声明所需权限级别，执行前校验 Agent 权限。
    *   动态权限提升: 临时提升权限执行特定操作，需通过 Gateway 发送权限提升请求，用户批准后生效。

### 1.6 错误处理与恢复策略 (Error Handling & Recovery)
基于现有的 `AppError` 统一错误类型体系扩展。

*   **LLM API 调用失败处理**:
    *   **重试策略**: 指数退避重试（初始延迟 1s，最大延迟 30s，最多 3 次）。基于现有 `WorkflowError::is_retryable()` 方法扩展。
    *   **超时处理**: 配置化超时时间（默认 120s），使用 `tokio::time::timeout` 包装异步请求。
    *   **限流处理**: 实现 Token Bucket 算法控制每个 Provider 的请求速率。根据 API 响应头 `X-RateLimit-Remaining` 动态调整。
    *   **错误分类**:
        *   **临时错误**: 网络超时、5xx 状态码、限流 → 自动重试。
        *   **永久错误**: 4xx 状态码（除 429）、认证失败、模型不存在 → 立即失败，返回错误给用户。
    *   **降级方案**: 主 Provider 失败时，自动切换到备用 Provider（若配置）。
*   **工具执行失败处理**:
    *   **降级策略**: 文件读取失败时尝试其他编码；网络请求失败时使用缓存数据。
    *   **错误分类**: `RecoverableError`（可重试）vs `FatalError`（不可恢复）。
    *   **错误传播**: 子代理失败时，将错误摘要注入父代理上下文，父代理决定是否重试或调整策略。
*   **执行器崩溃恢复**:
    *   **At-Once Persistence**: 每个 Step 执行完毕后立即持久化 `WorkflowMessage` 到 SQLite。
    *   **状态机快照**: 每 10 个 Step 或执行超过 5 分钟时，保存完整的执行器状态快照（当前 Step、挂起的工具调用等）。
    *   **恢复流程**: 重启后从 SQLite 加载消息流，定位到最后一个成功的 Step，重新构建上下文并继续执行。
    *   **幂等性保证**: 工具调用设计为幂等操作，避免重复执行副作用。

### 1.7 监控与可观测性 (Monitoring & Observability)
基于现有的 `fern` 日志系统扩展。

*   **结构化日志标准**:
    *   **关键事件**: 工具调用开始/结束、LLM 请求/响应、状态转换、错误发生、压缩触发。
    *   **日志格式**: JSON 结构化日志，必填字段：`timestamp`, `level`, `session_id`, `agent_id`, `event_type`, `message`。
    *   **日志级别映射**:
        *   `ERROR`: 系统错误（LLM API 失败、工具执行异常）。
        *   `WARN`: 重试/降级触发、资源接近限制。
        *   `INFO`: 工具调用、状态转换、压缩触发。
        *   `DEBUG`: 详细执行信息（参数、返回值摘要）。
*   **性能指标收集**:
    *   **工具执行时间**: 直方图统计（P50/P95/P99），按工具名称分组。
    *   **LLM 调用延迟**: 首字节时间（TTFT）、总响应时间，按 Provider 和模型分组。
    *   **Token 消耗**: 累计输入/输出 Token，当前会话 Token 速率（tokens/min）。
    *   **并发度监控**: 当前活跃 Agent 数量、待执行任务队列长度。
    *   **内存使用**: 上下文内存占用、工具返回缓存大小。
*   **分布式追踪**:
    *   **Trace Context**: 主代理生成 `trace_id`，子代理继承并创建子 `span_id`。
    *   **Span 记录**: 每个 Think-Act-Observe 循环作为一个 Span，记录开始时间、结束时间、工具名称、状态。
    *   **追踪导出**: 支持 OpenTelemetry 协议导出到 Jaeger/Zipkin，便于可视化分析。
*   **用户进度反馈**:
    *   **实时状态推送**: 通过 Gateway 的 `StateChanged` 事件推送当前状态（thinking/executing/paused）。
    *   **进度估算**: 基于历史执行数据（相似任务的 Step 数量）预估剩余时间。
    *   **执行树可视化**: 展示主代理和子代理的调用关系树，每个节点显示状态和关键信息。

### 1.8 并发控制与资源管理 (Concurrency & Resource Management)
基于现有的 `tokio` 异步运行时和 `Semaphore` 并发控制扩展。

*   **并发度控制**:
    *   **子代理数量限制**: 全局 `Semaphore` 限制最大并发 Agent 数（默认 5），通过配置 `max_concurrent_agents` 调整。
    *   **优先级队列**: 任务分为 `high`（用户交互）、`normal`（默认）、`low`（后台任务）三个优先级。高优先级任务插队执行。
    *   **公平调度**: 使用 `tokio::task::yield_now()` 定期让出执行权，防止子代理饥饿。
*   **LLM API 速率限制**:
    *   **Per-Provider 限流**: 每个 LLM Provider 独立的速率限制器（如 OpenAI 限制 60 req/min）。
    *   **Token Budget**: 基于 Token 数量而非请求数量的限流（如每分钟最多 100k tokens）。
    *   **动态调整**: 根据API响应头 `X-RateLimit-Reset` 动态调整请求速率，避免硬性限制。
*   **SQLite 并发控制**:
    *   **写入队列**: 使用 `tokio::sync::Mutex` 序列化写操作，避免写入冲突。
    *   **读优化**: 启用 SQLite WAL 模式，支持多个并发读操作。
    *   **批量写入**: 累积多条消息（如 10 条或 1 秒后）后批量插入，减少磁盘 I/O。
*   **内存管理**:
    *   **工具返回大小限制**: 默认限制 1MB，超过阈值自动截断或压缩（基于现有 `LruCache`）。
    *   **上下文内存限制**: 使用 LRU 淘汰策略，最多保留最近 50 条消息在内存中，更早的从数据库按需加载。
    *   **大文件处理**: 文件读取使用流式处理（`tokio::io::BufReader`），避免一次性加载到内存。
*   **资源隔离**:
    *   **资源配额**: 每个 Agent 独立的资源配额（最大内存、最大执行时间、最大工具调用次数）。
    *   **优雅降级**: 资源超限时，暂停低优先级 Agent，保留高优先级 Agent 继续执行。
    *   **资源监控**: 定期检查资源使用情况，超过 80% 阈值时触发预警日志。

### 1.9 调试与开发体验 (Debugging & Developer Experience)
基于现有的日志分级和配置系统扩展。

*   **开发模式配置**:
    *   **前台运行**: 环境变量 `CHATSPEED_DEV=1` 启用开发模式，禁用后台服务，所有日志输出到控制台。
    *   **详细日志**: 自动开启 `DEBUG` 级别日志，包含工具参数和返回值的完整内容。
    *   **工具调用详情**: 打印工具名称、完整参数、返回值摘要、执行时间。
*   **Dry-Run 模式**:
    *   **工具模拟**: 配置 `dry_run: true` 后，工具不实际执行，返回模拟结果（如文件读取返回固定内容）。
    *   **安全检查**: 检测危险操作（如删除文件），记录日志但不实际执行。
    *   **成本预估**: 预估 Token 消耗（基于历史数据）和执行时间，帮助用户评估任务成本。
*   **上下文检查接口**:
    *   **实时查询**: 通过 Tauri Command `get_current_context(session_id)` 查看当前上下文内容（消息列表、Token 数量）。
    *   **消息历史**: 通过 `get_message_history(session_id, limit)` 查看完整的消息流。
    *   **工具调用链**: 通过 `get_tool_call_chain(session_id)` 可视化工具调用关系（JSON 树结构）。
*   **执行回放**:
    *   **历史重建**: 从数据库加载历史 `workflow_messages`，重建执行过程。
    *   **Step-by-step 重放**: 提供 `replay_step(session_id, step_index)` 接口，重放到指定 Step 并暂停。
    *   **断点调试**: 在特定 Step 设置断点，执行到断点时自动暂停并等待开发者检查。
*   **本地测试工具**:
    *   **Mock LLM API**: 提供 `MockLLMProvider`，根据输入关键词返回预定义的响应，避免真实 API 调用。
    *   **工具测试框架**: 每个工具提供 `test()` 方法，独立测试工具逻辑（如 `safe_read_file_test()`）。
    *   **场景录制**: 提供 `record_scenario(session_id)` 接口，录制真实执行场景（输入、输出、工具调用），用于回归测试。

### 1.10 配置管理策略 (Configuration Management)
基于现有的 `MainStore` 配置存储扩展。

*   **配置项定义**:
    *   **Token 压缩阈值**: `compression_threshold_tokens` (默认: 8000) - 超过此阈值触发上下文压缩。
    *   **工具执行超时**: `tool_timeout_seconds` (默认: 300) - 单个工具最大执行时间。
    *   **并发限制**: `max_concurrent_agents` (默认: 5) - 最大并发子代理数量。
    *   **审批规则**: `approval_rules` (JSON 配置) - 定义哪些操作需要用户审批（如 `{"shell_execute": "always", "file_write": "if_not_in_whitelist"}`）。
    *   **安全策略**: `security_policy` (JSON 配置) - 包含路径白名单 `allowed_paths`、命令黑名单 `forbidden_commands` 等。
    *   **日志级别**: `log_level` (默认: "info") - 支持 trace/debug/info/warn/error。
*   **配置存储**:
    *   **持久化位置**: SQLite `config` 表（基于现有 `MainStore`），键值对存储 JSON 格式配置。
    *   **配置分组**: 分为 `global`（全局配置）、`agent:{agent_id}`（Agent 特定配置）、`tool:{tool_name}`（工具特定配置）。
    *   **热更新**: 运行时修改配置后立即生效，无需重启执行器（通过 `ConfigWatcher` 监听配置变化）。
*   **配置访问**:
    *   **类型安全**: 使用强类型配置结构体（如 `CompressionConfig`, `SecurityConfig`），避免字符串键值对。
    *   **默认值**: 为所有配置项提供合理默认值，定义在 `src-tauri/src/config/defaults.rs`。
    *   **环境变量覆盖**: 支持通过环境变量覆盖配置（如 `CHATSPEED_MAX_AGENTS=10` 覆盖 `max_concurrent_agents`）。
*   **配置验证**:
    *   **启动时验证**: 检查配置合法性（如 `max_concurrent_agents` 必须大于 0）。
    *   **范围检查**: 确保数值在合理范围内（如 `tool_timeout_seconds` 不能超过 3600）。
    *   **依赖检查**: 验证配置项之间的依赖关系（如启用压缩必须配置压缩模型）。
*   **配置版本管理**:
    *   **配置迁移**: 升级时自动迁移旧配置（如 v1.0 配置格式转换到 v2.0），迁移脚本放在 `src-tauri/src/config/migrations/`。
    *   **配置回滚**: 支持回滚到历史配置（保留最近 10 个配置快照）。
    *   **配置导出/导入**: 支持 JSON 格式导出/导入配置，便于配置共享和备份。

## 2. 实施路线图 (Detailed Phases)

### Phase 0: 测试基础设施 (Testing Infrastructure)
*   **任务 0.1**: 搭建 Mock LLM API 框架。实现 `MockLLMProvider`，支持预定义响应和场景录制。
*   **任务 0.2**: 实现工具测试框架。为每个基础工具提供独立测试接口。
*   **任务 0.3**: 搭建集成测试框架。支持端到端工作流测试和并发场景测试。
*   **验收标准**: 单元测试覆盖率 >80%，集成测试覆盖核心场景，安全测试 100% 通过。

### Phase 1: 存储适配与数据建模 (Storage & Model)
*   **任务 1.1**: 更新 `v5.rs` 或新增 `v6.rs` 迁移脚本。完善 `workflow_messages` 和 `workflows` 表字段。
*   **任务 1.2**: 在 `src-tauri/src/db/workflow.rs` 中完善 Rust CRUD 接口，支持上下文内存还原。
*   **验收标准**: 单元测试验证崩溃重启后的上下文 100% 还原。

### Phase 2: 网关 Trait 与流式 JSON 解析 (Gateway & Stream)
*   **任务 2.1**: 定义 `Gateway` Trait。实现 `TauriGateway` 适配器。
*   **任务 2.2**: 开发 `StreamParser`，从流式 JSON 中提取 `answer_user` 文本块并推送。
*   **验收标准**: 模拟流式碎片 JSON，前端能实时显示文本内容。

### Phase 3: 核心 ReAct 执行器 (The Executor)
*   **任务 3.1**: 实现 `WorkflowExecutor` 主循环。支持 Think -> Act -> Observe 闭环。
*   **任务 3.2**: 开发 `ObservationReinforcer` 和上下文压缩逻辑。
*   **验收标准**: 执行器能自主完成多轮工具调用循环并处理上下文溢出。

### Phase 4: 安全工具集与 Skills 动态加载 (Safe Tools)
*   **任务 4.1**: 实现支持多目录白名单的 `PathGuard`。
*   **任务 4.2**: 升级文件与 Shell 工具，接入安全审计与审批流程。
*   **任务 4.3**: 实现 Skills 动态扫描与加载。
*   **备注 (Shell 安全增强设计)**:
    *   **分级决策体系**: 借鉴 Gemini CLI 的 `PolicyEngine`，实现 `Allow` (白名单直接运行)、`NeedsReview` (敏感操作需 UI 确认)、`Deny` (系统致命命令直接阻断) 三级决策。
    *   **模式匹配支持**: 允许在 Agent 配置中使用 **正则表达式 (Regex)** 或 **通配符 (Glob)** 来定义审计规则。
    *   **多维审计**:
        *   **命令匹配**: 匹配整行命令或 Token 基名（如正则 `^git commit .*` 自动通过）。
        *   **路径专项拦截**: 针对路径参数支持 Glob 匹配（如禁止对 `/etc/*` 或用户指定的敏感目录 `/private/data/*` 进行任何 `rm` 或重定向操作）。
    *   **递归与展开**: 审计前必须执行递归嵌套检测（针对 `$()`）和环境变量展开（针对 `$HOME`），防止通过变量隐藏敏感路径。
*   **验收标准**: 安全边界测试 100% 通过；Shell 执行需 UI 确认。

### Phase 5: 子代理并行调度 (Orchestrator)
*   **任务 5.1**: 实现 `spawn_agent` 工具及多 Agent 异步调度逻辑。
*   **任务 5.2**: 实现结果汇总与报告生成。
*   **验收标准**: 成功演示主代理调度多个子代理并行协作完成复杂任务。

### Phase 6: Tauri Command & 系统集成 (Integration)
*   **任务 6.1**: 封装 `workflow_start`, `workflow_signal`, `workflow_get_tasks` 等 Tauri 命令。
*   **任务 6.2**: 实现 `TauriGateway` 的单例管理与生命周期绑定。
*   **任务 6.3**: 注册所有新命令并解决跨模块依赖导致的编译错误。
*   **验收标准**: `cargo check` 无错误，新命令可通过 `tauri-invoke` 调通。

### Phase 7: 前端 UI 适配与流式展示 (Frontend UI)
*   **任务 7.1**: 修改 `WorkflowView.vue` 及相关组件，适配 ReAct 异步信令模式。
*   **任务 7.2**: 实现“挂起待审” UI 组件，支持用户对 Shell/FS 操作进行通过/拒绝。
*   **任务 7.3**: 实现消息流的实时渲染，支持 Thinking 状态的动态折叠。
*   **验收标准**: 用户可在界面启动 Agent，并实时看到其思考、工具调用及审批弹窗。
