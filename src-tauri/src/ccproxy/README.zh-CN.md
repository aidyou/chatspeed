[简体中文](./README.zh-CN.md) | English

# 模块说明

## 关于 ccproxy

`src-tauri/src/ccproxy` 是 ChatSpeed 的 AI 代理与协议适配引擎，负责在 **OpenAI 兼容协议**、**Claude**、**Gemini** 和 **Ollama** 之间进行路由、直通和协议转换。

## 核心能力

### 1. 灵活路由

ccproxy 支持多种访问模式：

- **直连路由**：`/v1/...`、`/v1beta/...`、`/api/...`
- **分组路由**：`/{group_name}/v1/...`
- **动态切换**：`/switch/v1/...` 会解析到当前激活的代理分组
- **兼容模式**：`/compat/...` 或 `/compat_mode/...` 可为不支持原生 tool calling 的模型开启工具兼容能力

### 2. 多协议适配

当请求需要协议转换时，会经过 **Input -> Unified -> Backend -> Unified -> Output** 这条适配链路。

### 3. OpenAI 兼容接口面

ccproxy 提供 OpenAI 风格接口，包括：

- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /v1/responses`
- `POST /v1/embeddings`

`/v1/responses` 现已成为一等协议路径：

- 路由覆盖直连、分组、`/switch` 和 compat 变体
- 入口处理器是 `handler/responses_handler.rs`
- `adapter/input/openai_responses_input.rs` 在需要适配时会把 Responses 请求转换为统一请求模型
- `adapter/output/openai_responses_output.rs` 会把统一响应和流式事件重新转换为 OpenAI Responses 兼容输出
- 当 provider metadata 设置了 `supports_responses_api` 或 `supportsResponsesApi` 时，ccproxy 会优先直通上游 `/responses` 端点
- 否则会回退到统一 chat 流水线，再将结果重新序列化为 Responses 输出

### 4. Embedding 代理

统一的 embedding 端点：

- **OpenAI**：`/v1/embeddings`
- **Claude**：`/v1/claude/embeddings`
- **Gemini**：`:embedContent` 或 `/embedContent`
- **Ollama**：`/api/embed` 和 `/api/embeddings`

## 工作方式

请求进入协议处理器后，只会走两条规范路径之一：

1. **直通转发**：客户端协议与后端协议一致，且 compat mode 关闭
2. **统一适配**：需要协议转换或工具兼容时使用

无论走哪条路径，ccproxy 都会继续负责：

- 模型解析
- 鉴权与代理头注入
- 重试与统计记录
- 协议安全的响应转换
- 通过 `filter_proxy_headers` 进行响应头过滤

## 重要说明

- `router.rs` 对顺序敏感，调整路由层级时必须避免 route shadowing。
- 静态协议前缀必须始终排在动态分组路由前面。
- `Content-Length`、`Transfer-Encoding`、`Connection`、`Content-Encoding` 等传输层响应头不能直接透传。
- 维护本模块时，以 `CONSTITUTION.md` 中的约束和不变量为准。
