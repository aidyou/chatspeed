# 模块介绍

## 关于ccproxy

@src-tauri/src/ccproxy 模块是一个高性能 AI 代理转换引擎，专注于在 **OpenAI 兼容协议**、**Gemini**、**Claude** 和 **Ollama** 协议之间进行全双工转换、请求路由及功能增强。

## 核心特性

### 1. 灵活的路由系统

ccproxy 采用层次化路由设计，支持多种访问模式：

- **直接访问**: 如 `/v1/chat/completions`，使用全局默认配置。
- **分组路由 (Grouping)**: 如 `/{group_name}/v1/...`，为不同场景/客户端隔离模型配置。
- **动态切换 (Switching)**: 使用 `/switch` 前缀（如 `/switch/v1/...`）自动路由到应用后台当前设定的“活动”分组。
- **工具兼容模式 (Compat Mode)**: 通过 `/compat` 或 `/compat_mode` 路径（支持组合，如 `/switch/compat/v1/...`）为原生不支持工具调用的模型开启增强功能。

### 2. 多协议适配器

模块通过“输入-后端-输出”三层适配器架构实现协议转换。
**数据流示例 (Claude 客户端 -> OpenAI 后端):**
`Client -> Claude Input (转换为 Unified 格式) -> OpenAI Backend (转换为 OpenAI 格式) -> Upstream AI -> OpenAI Backend (转回 Unified) -> Claude Output (转回 Claude 格式) -> Client`

### 3. 工具兼容模式 (Tool Compat)

针对原生不支持函数调用的模型，ccproxy 通过 Prompt 注入和 XML 拦截技术，将模型输出的文本实时解析并转换为协议标准的工具调用格式，对客户端完全透明。

### 4. 嵌入 (Embedding) 代理

统一了各协议的嵌入接口：

- **OpenAI**: `/v1/embeddings`
- **Claude**: `/v1/claude/embeddings` (由代理提供，用于协议一致性)
- **Gemini**: 支持 `:embedContent` 或 `/embedContent` 动作
- **Ollama**: `/api/embed` 和 `/api/embeddings`

## 模块原理

数据经过路由进入 `handle_chat_completion` 函数后，会经历以下步骤：

1. **解析模型**: 根据别名或 X-CS 头部定位具体的后台模型实例。
2. **直连检查**: 如果客户端协议与后端一致且未开启兼容模式，则执行高性能头部过滤透传（Direct Forward）。
3. **适配转换**: 若协议不一致，通过对应的 `Adapter` 进行双向格式化。
4. **头部过滤**: 所有响应都会经过 `filter_proxy_headers` 处理，移除 `Content-Length`、`Connection` 等传输级头部，确保与 Axum 框架兼容。

## 注意事项

- **路由挂载**: 虽然本项目在 `ccproxy/router.rs` 中同步挂载了 MCP 等其他模块的路由，但那些属于独立业务模块，不属于 `ccproxy` 代理引擎的核心逻辑。
- **严禁随意修改路由挂载顺序**: 当前的顺序（固定前缀 > 直接路由 > 全局兼容模式 > 动态分组）是经过精心设计的，旨在防止路由遮蔽（Route Shadowing）。
