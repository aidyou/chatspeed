# Module Introduction

## About ccproxy

The @src-tauri/src/ccproxy module is an AI chat proxy that can convert between any of the following protocols: OpenAI-compatible, Gemini, Claude, and Ollama.

## How It Works

We'll use an example of a request from a Claude-protocol client to an OpenAI-compatible server to illustrate the data flow and the module's basic working principle. The data flow is as follows:

1. A user sends a request to the module via the Claude protocol. The data is routed to the `handle_chat_completion` function in @src-tauri/src/ccproxy/handler/chat_handler.rs.
2. The data is processed by the `from_claude` function in @src-tauri/src/ccproxy/adapter/input/claude_input.rs, converting it to a unified format.
3. The data is then processed by the `adapt_request` function in @src-tauri/src/ccproxy/adapter/backend/openai.rs, which converts the unified format to the OpenAI-compatible protocol format.
4. The converted data is sent to the OpenAI-compatible server. The returned data, depending on the request mode (synchronous or asynchronous), is processed by the `adapt_response` (sync) or `adapt_stream_chunk` (async) function in @src-tauri/src/ccproxy/adapter/backend/openai.rs and converted back to the unified format.
5. Finally, the data is processed by the `adapt_response` (sync) or `adapt_stream_chunk` (async) function in @src-tauri/src/ccproxy/adapter/output/claude_output.rs to be converted into the Claude-specific output format and sent to the client.

This completes the entire proxy request process. The entire data flow can be represented as:

client -> claude protocol request -> from_claude (claude -> unified) -> adapt_request (unified -> openai) -> openai server -> `adapt_response | adapt_stream_chunk` (openai -> unified) -> `adapt_response | adapt_stream_chunk` (unified -> claude) -> client

### Tool Compatibility Mode

Models deployed on OpenAI-compatible servers or Ollama often have incomplete native support for tool calls, which can limit the utility of certain free models. To enhance the capabilities of these models, we have added a Tool Compatibility Mode. The principle is as follows:

1.  Tool call definitions are parsed into an XML-formatted description, which is passed to the AI along with the prompt. The AI is instructed to return an XML format if it needs to call a tool.
2.  The returned XML data is then parsed, and the tool call data within the XML is converted into a standard tool call format.
