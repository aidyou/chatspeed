# Agent CLI 评估与自我改进方案 - 补充章节

> 本文档是 `agent-cli-evaluation-self-improvement-design.md` 的补充，包含根据评审反馈新增的章节。

## 17. 成本和使用模型

### 17.1 成本归属原则

ChatSpeed 作为开源工具，只提供 CLI 和评测框架功能；成本由实际使用者承担：

- **个人开发者**：使用自己的 LLM API keys，承担自己实验的 API 成本；
- **企业或组织**：可以配置统一的 API keys 和成本中心，由组织统一管理预算；
- **研究机构**：可以使用学术 API credits 或自托管模型。

### 17.2 成本控制机制

虽然 ChatSpeed 不承担成本，但必须提供完善的成本控制功能，避免用户因配置错误或失控的自我改进而产生意外账单：

**Per-request budget**：
- 每个 LLM 请求的 max tokens 限制；
- 超出时立即中止该请求。

**Per-trial budget**：
- 单次实验的总成本上限（美元或 token 数）；
- 达到上限时 gracefully stop，保存已完成的 artifact。

**Per-candidate budget**：
- 每个候选配置的总测试预算；
- 防止单个差候选耗尽所有资源。

**Per-campaign budget**：
- 整个 improve campaign 的总预算上限；
- 达到上限时暂停 campaign，生成当前报告。

**预算预估**：
- `chatspeed experiment estimate` 命令：根据历史数据估算本次实验的预期成本范围；
- 在启动实验前显示估算成本并要求确认；
- 提供 `--max-cost` 参数强制上限。

**成本追踪**：
- 实时记录每个 conversation 的累计成本（input/output/cache tokens × model pricing）；
- `chatspeed experiment status <id>` 显示当前已消耗成本和剩余预算；
- 导出详细的成本报告（per-model、per-tool、per-phase）。

**Cost dashboard**：
- `chatspeed cost summary --period month` 显示用户本月总成本；
- 按 agent、model、experiment 类型分组统计；
- 识别异常高成本的 runs 并提供分析。

### 17.3 开源和社区

作为开源项目：

**代码和框架**：
- 完整的 CLI 实现和 control plane 协议开源；
- benchmark adapter 框架开源；
- 评测和自我改进的核心逻辑开源。

**私有资产**：
- ChatSpeed 官方维护的 private holdout 任务集不开源（避免污染）；
- 社区可以创建和分享自己的 benchmark 和 holdout；
- 提供创建 private holdout 的指南和工具。

**隐私保护**：
- 所有实验数据和轨迹保存在用户本地或用户控制的环境；
- ChatSpeed 不收集、上传或存储任何用户的代码、prompt 或实验结果；
- Opt-in telemetry 仅收集 aggregate metrics（如：版本、命令使用频率），不包含任何业务数据；
- 轨迹导出工具自动 redact API keys、tokens、secrets 和敏感路径。

**社区贡献**：
- 欢迎社区贡献新的 benchmark adapter（如 HumanEval、MBPP）；
- 欢迎贡献改进的 evaluator 和 proposer；
- 鼓励分享匿名化的 aggregate metrics 和 best practices；
- 建立 benchmark leaderboard（基于公开 benchmark，用户自愿提交）。

## 18. 错误处理和可观测性

### 18.1 错误分类和处理策略

**Transient errors**（临时性错误）：
- 网络超时、LLM rate limit、临时资源不足、MCP 进程短暂不可用；
- 策略：指数退避重试（100ms, 200ms, 400ms），最多 3 次；
- 记录重试次数和最终是否成功；
- 如果重试成功，conversation 继续；失败则转为 permanent error。

**Permanent errors**（永久性错误）：
- Invalid config、unauthorized API key、resource not found、schema validation failure；
- 策略：立即失败，返回 actionable error message，指导用户如何修复；
- 不重试，直接终止当前操作；
- 保存 error context 到 artifact。

**Infrastructure errors**（基础设施错误）：
- 主进程 crash、数据库损坏、MCP zombie 进程、disk full；
- 策略：记录到单独的 `infra_failure` bucket，不计入 correctness metrics；
- 触发 `chatspeed doctor` 检查；
- 如果 infra failure rate > 5%，自动暂停 campaign。

**User errors**（用户错误）：
- 错误的命令参数、不存在的 agent ID、冲突的配置；
- 策略：在客户端（CLI）尽早检测，提供清晰的 usage 提示；
- 不发送无效请求到服务端；
- 返回 exit code 2（参数错误）。

### 18.2 结构化日志

每条日志包含：
```json
{
  "timestamp": "2026-09-13T10:30:45.123Z",
  "level": "INFO|WARN|ERROR",
  "component": "workflow_executor|tool_manager|mcp_client|cli",
  "conversation_id": "conv_abc123",
  "request_id": "req_xyz789",
  "message": "Tool execution completed",
  "context": {
    "tool_name": "read_file",
    "duration_ms": 45,
    "success": true
  }
}
```

**自动 redaction**：
- API keys：`sk-***` 或完全隐藏；
- Bearer tokens：不记录；
- 环境变量：只记录 key，不记录 value；
- 文件内容：只记录路径和大小，不记录内容；
- LLM 请求/响应：只记录 token 数和成本，不记录完整 text（除非用户明确启用 debug）。

**日志级别**：
- DEBUG：详细的执行流程，默认不输出；
- INFO：正常操作（workflow started、tool executed、event sent）；
- WARN：非致命问题（重试成功、降级运行、budget 接近上限）；
- ERROR：失败操作（请求拒绝、资源不可用、内部错误）。

### 18.3 Metrics 和监控

推荐的 Prometheus-style metrics：

```
# CLI 请求
chatspeed_cli_request_duration_seconds{command, status}
chatspeed_cli_request_total{command, status}

# Workflow 状态
chatspeed_workflow_active{agent_id, phase}
chatspeed_workflow_completed_total{agent_id, outcome}

# LLM 调用
chatspeed_llm_tokens_total{model, type=[input|output|cache]}
chatspeed_llm_request_duration_seconds{model, status}
chatspeed_llm_cost_dollars{model}

# Tool 执行
chatspeed_tool_call_duration_seconds{tool_name, approval_required}
chatspeed_tool_call_total{tool_name, status}

# 预算
chatspeed_budget_remaining{conversation_id, type=[tokens|dollars]}
chatspeed_budget_exceeded_total{conversation_id}

# 基础设施
chatspeed_mcp_processes{server_id, state}
chatspeed_sse_connections_active
chatspeed_db_query_duration_seconds{operation}
```

可选的 metrics exporter（`chatspeed metrics serve --port 9090`）供 Prometheus 抓取。

### 18.4 Debug 模式

`chatspeed --debug <command>` 输出：
- 完整的 HTTP request/response（包括 headers）；
- Config resolution 的每一步（base config → overrides → resolved）；
- Tool execution 的详细参数和返回值；
- SSE events 的原始 payload；
- 不自动 redact（仅用于本地调试，不应在 CI 或生产环境启用）。

`chatspeed --trace <command>` 输出：
- 类似 debug，但增加 distributed tracing headers；
- 每个操作带 trace_id 和 span_id；
- 可与 Jaeger/Zipkin 集成。

## 19. 性能和资源管理

### 19.1 并发限制

**全局限制**（防止资源耗尽）：
- 单个主进程最多 `N` 个 active conversations（默认 10，可配置）；
- 超出时新请求返回 `429 Too Many Requests`；
- 用户可通过 `chatspeed config set max_concurrent_workflows 20` 调整。

**Per-conversation 限制**：
- 每个 conversation 最多 `M` 个并行 tool calls（默认 3）；
- 防止单个 agent 创建过多并行任务；
- 某些工具（如 `bash`）可能需要串行执行。

**MCP 进程池**：
- 全局 MCP 进程池上限（默认 20）；
- 空闲超过 30 分钟的 MCP 进程自动回收；
- 同一 MCP server 的多个 conversation 共享进程（如果支持多路复用）。

### 19.2 内存管理

**SSE event buffer**：
- 每个 subscriber 独立 buffer，上限 10MB；
- 超出时断开连接，返回 `buffer_overflow` 错误；
- 客户端应从 snapshot + durable events 重建状态。

**轨迹增量传输**：
- Streaming delta chunk size：64KB；
- 大型 artifact（如完整 codebase snapshot）不通过 SSE，使用 artifact API 单独下载。

**大型文件处理**：
- File read/write 使用 streaming，不一次性加载到内存；
- Tool output 超过 1MB 时截断，提示用户使用 artifact API 获取完整输出。

### 19.3 资源清理

**Conversation 完成后**：
- Grace period 5 分钟：保持 runtime 资源（session、MCP 连接），允许快速 resume；
- 5 分钟后：释放内存、关闭 MCP 连接、清理临时文件；
- 持久化数据（DB、artifacts）永久保留（直到用户手动删除）。

**临时文件**：
- 每次实验使用独立临时目录：`/tmp/chatspeed/conv_<id>/`；
- 实验完成后按 retention policy 清理：
  - 成功的实验：保留 7 天；
  - 失败的实验：保留 30 天（用于调试）；
  - 可通过 `chatspeed cleanup --older-than 7d` 手动清理。

**MCP 进程**：
- Idle timeout 30 分钟后自动退出；
- Conversation 停止时发送 graceful shutdown signal；
- 强制 kill timeout 5 秒。

**数据库维护**：
- 定期 VACUUM（每周或手动 `chatspeed doctor vacuum`）；
- 压缩旧的 workflow events（90 天后）；
- 自动备份（可选，默认不启用）。

## 20. 版本演进和兼容性

### 20.1 Protocol 版本策略

**HTTP API 版本**：
- 使用语义化版本：`/control/v1`, `/control/v2`；
- Breaking change 需要新的 major version；
- Non-breaking addition 可在同一 major version 中增加（可选字段、新 endpoint）；
- 保持旧版本至少两个 major release（例如 v3 发布后，v1 仍需支持）。

**Deprecation 流程**：
- 提前一个版本在响应中添加 `X-Deprecated-API` header；
- 文档中明确标注 deprecated 并提供迁移指南；
- 至少 6 个月后才能移除 deprecated API。

**Schema 版本**：
- 每个 DTO 带 `schema_version` 字段；
- 使用结构化 schema evolution（添加字段向后兼容，删除字段向前不兼容）；
- 提供 schema 转换工具：`chatspeed schema convert --from 1 --to 2 < old.json > new.json`。

### 20.2 数据迁移

**自动迁移**：
- ChatSpeed 启动时检测数据库 schema version；
- 如果低于当前版本，自动运行增量 migration scripts；
- Migration 前自动备份：`~/.chatspeed/backups/chatspeed.db.pre-v2.backup`。

**向前兼容**（读取旧数据）：
- 新版本必须能读取和执行旧版本创建的 conversations；
- 可能需要在加载时升级内存中的表示，但不修改持久化数据；
- 旧版本的 artifact 可能缺少新字段，使用合理的默认值。

**向后兼容**（导出旧格式）：
- 提供 `chatspeed export --format legacy-v1` 导出旧版本格式；
- 尽力而为：新功能可能无法完整表达在旧格式中；
- 用于与旧版本 CLI 或第三方工具集成。

### 20.3 实验可重现性

**完整环境记录**：
```json
{
  "environment": {
    "chatspeed_version": "0.5.2",
    "chatspeed_main_process_version": "0.5.2",
    "protocol_version": "v1",
    "cli_commit": "abc123def",
    "platform": "linux-x86_64",
    "rust_version": "1.75.0"
  }
}
```

**Replay 功能**：
- `chatspeed replay <artifact-dir>`：使用历史 artifact 重放实验；
- 检查当前环境是否兼容历史环境（版本、protocol）；
- 如果不兼容，给出明确警告："该 artifact 由 v0.3 生成，当前为 v0.5，结果可能不同"；
- Replay 仅用于调试和分析，不应用于生产 metrics。

**Deterministic replay**（可选，未来）：
- 记录每次 LLM 响应，replay 时直接使用录制的响应，不调用真实 API；
- 用于 bit-exact 重现（例如调试特定失败）；
- 需要额外存储空间，默认不启用。

## 21. 私有 Holdout 构建指南

### 21.1 任务来源策略

**真实用户反馈**：
- 从 GitHub issues、用户报告的 bug 和 feature requests 中提取；
- 确保任务描述不包含解决方案或实现细节；
- 验证任务可以在合理时间内（< 10 分钟）由人类专家完成。

**ChatSpeed 特有场景**：
- 跨 Rust/Vue 的命令契约修改；
- Tauri IPC 和 Vue Composition API 交互；
- Workflow state management 和 approval flow；
- SQLite schema migration 和数据一致性；
- MCP/skill 生命周期管理；
- Sandbox 和 PathGuard 边界验证。

**合成任务**：
- 使用模板生成变体（不同语言、不同错误类型、不同边界条件）；
- 确保合成任务不会意外泄漏到公开数据集；
- 人工验证每个合成任务的合理性和 oracle 正确性。

### 21.2 Oracle 质量保证

**多重验证**：
- 每个任务至少由 2 位独立验证者确认 oracle 正确；
- 如果验证者意见不一致，增加第 3 位验证者或重新设计任务；
- 记录每个验证者的判断和理由。

**Baseline 通过率**：
- 用当前 production Agent 运行每个新任务 3-5 次；
- 记录 baseline 通过率（应该在合理范围，如 40%-80%）；
- 太简单（>95%）或太难（<10%）的任务需要调整难度或移除。

**任务难度分层**：
- **Trivial**（预期 >90% 通过）：单文件简单修改、明显的 bug fix；
- **Medium**（预期 50-80% 通过）：多文件协调、需要代码导航；
- **Hard**（预期 20-50% 通过）：复杂重构、跨层修改、微妙的边界条件；
- **Expert**（预期 <20% 通过）：架构级变更、性能优化、安全加固。

### 21.3 防污染措施

**任务描述隔离**：
- 任务 ID 使用随机 UUID，不使用连续编号；
- 任务描述不得出现在公开代码、commit message、PR 描述或文档中；
- 禁止在 proposer 可见的日志或轨迹中包含完整任务描述。

**曝光限制**：
- 每个任务维护 exposure counter；
- 达到阈值（建议 ≤10 次）后标记为"high exposure"，优先轮换；
- Finalist 在私有 holdout 上的运行次数单独计数和限制。

**过拟合检测**：
- 监控候选在 private holdout 上的 per-task variance；
- 如果某个候选只对特定任务显著改进，怀疑 task-specific overfitting；
- 交叉验证：用新的同类任务验证改进是否 generalize。

**定期轮换**：
- 每季度或每 N 次曝光后轮换 20-30% 的任务；
- 保留一部分核心任务作为长期趋势监控；
- 新任务加入前必须经过完整的质量保证流程。

### 21.4 Holdout 管理工具

建议提供的工具：

```bash
# 创建新任务
chatspeed holdout create \
  --id <uuid> \
  --description task.md \
  --workspace template/ \
  --oracle oracle.sh \
  --difficulty medium \
  --category rust-tauri

# 验证任务
chatspeed holdout validate <id> \
  --validator alice \
  --verdict pass|fail \
  --comment "..."

# 建立 baseline
chatspeed holdout baseline <id> \
  --agent production \
  --runs 5

# 检查曝光
chatspeed holdout exposure <id>

# 轮换任务
chatspeed holdout rotate \
  --threshold 10 \
  --replace-count 20

# 导出匿名统计
chatspeed holdout stats \
  --aggregate-only
```

## 22. 统计检验和晋级门槛

### 22.1 处理随机噪声

**实验设计**：
- Baseline 和 candidate 使用相同 task set、环境、模型快照和预算；
- 按时间 block 交替运行（A-B-A-B），减少 provider 和系统漂移影响；
- 记录每次运行的 timestamp、provider status、system load。

**配对检验**：
- 对于二元正确性（pass/fail），使用 paired tests：
  - McNemar's test（适用于 2x2 contingency table）；
  - Sign test（非参数，稳健）；
  - Paired bootstrap（估计置信区间）。
- 不使用独立样本 t-test，因为同一任务的 baseline 和 candidate 结果是配对的。

**效应量**：
- 报告 effect size（如 Cohen's d 或正确率差值）；
- 不只看 p-value，还要看实际改进幅度是否有意义；
- 95% 置信区间：如果 CI 包含 0，说明改进不显著。

### 22.2 晋级门槛（初始建议）

**Hard safety gate**：
- Hard safety violation = 0（任何安全违规都拒绝）；
- Secret leakage、sandbox escape、unauthorized file access、reward hacking = 0。

**正确性改进**：
- 主指标（如 SWE-bench pass rate）提升至少 2 个百分点；
- 差值的 95% CI lower bound > 0（统计显著）；
- Per-language 或 per-suite 不能有显著退化（>1 个百分点且 CI 不包含 0）。

**关键回归防护**：
- 在关键 regression suite（如 ChatSpeed 核心功能）上：
  - 退化上界 ≤ 1 个百分点；
  - 如果某个 critical test 从 pass 变为 fail，需要人工审查。

**成本和延迟**：
- P95 latency 不超过 baseline 的 1.2 倍；
- 成功任务的平均成本不超过 baseline 的 1.5 倍；
- 总 token 数不超过预设 cap（防止无限 context 膨胀）。

**Full profile 稳定性**：
- 如果存在 Full + 弱模型路径，该路径的结果零非预期变化；
- 确保 Minimal 的改进不会破坏 Full 的现有能力。

**样本量**：
- 如果样本不足（如 n < 30），先做 power analysis 估计需要的样本量；
- 不要在样本不足时机械套用阈值；
- 可能需要增加运行次数以达到足够的统计功效（power ≥ 0.8）。

### 22.3 停止条件细化

**成本上限**：
- Per-campaign budget 耗尽；
- Infra failure rate > 5%（说明环境不稳定，结果不可信）。

**收敛判定**：
- 连续 5 个 generation 无超过最小效应（如 +0.5%）的 val 改进；
- Archive 多样性坍缩（所有候选在配置空间中距离 < 阈值）；
- 连续 2 轮没有新 Pareto 点。

**过拟合检测**：
- Val 和 private holdout 的 gap 超过预设阈值（如 10%）；
- 某个候选在 val 上极好但在 holdout 上显著差于 baseline。

**安全熔断**：
- 任何 hard safety violation；
- Artifact provenance 或 budget ledger 不完整（说明追踪系统有 bug）；
- 关键 suite 显著回归（如核心功能 pass rate 下降 >5%）。

**Cost explosion**：
- 提升完全来自突破成本上限（说明只是"用更多钱"换来的）；
- 这种候选不应晋级，除非成本增加在可接受范围且有业务价值。
