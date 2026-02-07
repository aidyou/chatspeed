# Module Introduction

## About ccproxy

The `@src-tauri/src/ccproxy` module is a high-performance AI proxy and conversion engine. It focuses on full-duplex conversion, routing, and feature enhancement between **OpenAI-compatible**, **Gemini**, **Claude**, and **Ollama** protocols.

## Core Features

### 1. Flexible Routing System

ccproxy utilizes a hierarchical routing design supporting multiple access modes:

- **Direct Access**: e.g., `/v1/chat/completions`, uses global default settings.
- **Grouped Routing**: e.g., `/{group_name}/v1/...`, isolates model configurations for different scenarios or clients.
- **Dynamic Switching**: Using the `/switch` prefix (e.g., `/switch/v1/...`) automatically routes requests to the currently "Active" group set in the application.
- **Tool Compatibility Mode**: Enabled via `/compat` or `/compat_mode` (e.g., `/switch/compat/v1/...`), bringing function-calling capabilities to models that lack native support.

### 2. Multi-Protocol Adapters

The module implements protocol conversion through a tri-layer adapter architecture: **Input -> Backend -> Output**.
**Data Flow Example (Claude Client -> OpenAI Backend):**
`Client -> Claude Input (Unified format) -> OpenAI Backend (OpenAI format) -> Upstream AI -> OpenAI Backend (Unified format) -> Claude Output (Claude format) -> Client`

### 3. Tool Compatibility Mode (Compat Mode)

For models without native function-calling support, ccproxy injects specific prompts and utilizes XML interception to parse and convert text output back into protocol-standard tool calls, making the process seamless for clients.

### 4. Embedding Proxy

Standardized embedding endpoints across protocols:

- **OpenAI**: `/v1/embeddings`
- **Claude**: `/v1/claude/embeddings` (Provided by ccproxy for protocol consistency)
- **Gemini**: Supports `:embedContent` or `/embedContent` actions.
- **Ollama**: `/api/embed` and `/api/embeddings`.

## How It Works

Requests enter the `handle_chat_completion` function and undergo the following:

1. **Model Resolution**: Locates the backend model instance using aliases or `X-CS-*` headers.
2. **Direct Forwarding**: If the client and backend protocols match and Compat Mode is off, it performs high-performance direct pass-through with header filtering.
3. **Adaptation**: If protocols differ, it uses specialized `Adapters` for bi-directional formatting.
4. **Header Filtering**: All responses are processed by `filter_proxy_headers` to remove transport-level headers (`Content-Length`, `Connection`, etc.), ensuring compatibility with the Axum framework.

## Important Notes

- **Route Integration**: While the `ccproxy/router.rs` mounts routes for other components like MCP, those are independent business modules and not part of the `ccproxy` proxy engine's core logic.
- **DO NOT modify the route mounting order.** The current hierarchy (Fixed Prefixes > Direct Routes > Global Compat > Grouped Routes) is meticulously ordered to prevent "Route Shadowing".
