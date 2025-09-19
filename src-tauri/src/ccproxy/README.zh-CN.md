# 模块介绍

## 关于ccproxy

@src-tauri/src/ccproxy 模块是一个 ai 聊天代理，可以在 openai兼容协议、gemini、claude、ollama 之间的任意协议进行转换。

## 模块原理

我们以 claude 协议输入，请求到 openai 兼容协议服务器为例说明数据流向，通过数据流向来说明模块基本工作原理。数据流向如下：

1. 用户通过 claude 协议请求模块，数据经过路由进入 @src-tauri/src/ccproxy/handler/chat_handler.rs 的 `handle_chat_completion` 函数
2. 数据通过 @src-tauri/src/ccproxy/adapter/input/claude_input.rs 的 `from_claude` 函数处理，转换为 unified 格式
3. 数据通过 @src-tauri/src/ccproxy/adapter/backend/openai.rs 的 `adapt_request` 函数处理，将 unified 格式转换为 openai 兼容协议格式
4. 转换后的数据被发送到 openai 兼容协议服务器，返回的数据根据请求模式（同步或者异步）通过 @src-tauri/src/ccproxy/adapter/backend/openai.rs 的 `adapt_response`（同步）或 `adapt_stream_chunk`（异步）函数处理，转换为统一格式
5. 数据通过 @src-tauri/src/ccproxy/adapter/output/claude_output.rs 的 `adapt_response`（同步）或 `adapt_stream_chunk` 函数转为 claude 专用的输出格式发送给客户端

至此整个代理请求过程完成。整个数据流表示如下：

client -> claude protocol request -> from_claude (claude -> unified) -> adapt_request (unified -> openai) -> openai server -> `adapt_response | adapt_stream_chunk` (openai -> unified) -> `adapt_response | adapt_stream_chunk` (unified -> claude) -> client

### 工具兼容模式说明

openai 兼容格式服务器或者 ollama 部署的模型，对工具调用的原生支持往往不够完善，这会导致某些免费模型的用途受限。为了提高模型的能力，我们新增了工具兼容模式，原理就是：

1.  将工具调用解析成 xml 格式的说明文件，并作为prompt 一起传给ai，同时要求 ai 如果需要调用工具时返回 xml 格式
2.  解析返回的 xml 格式的数据，将 xml 中的工具调用数据转为标准的工具调用
