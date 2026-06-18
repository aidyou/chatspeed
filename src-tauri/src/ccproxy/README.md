[简体中文](./README.zh-CN.md) | English

# Module Introduction

## About ccproxy

`src-tauri/src/ccproxy` is ChatSpeed's AI proxy and protocol adaptation engine. It routes, forwards, and converts traffic between **OpenAI-compatible**, **Claude**, **Gemini**, and **Ollama** protocols.

## Core Features

### 1. Flexible Routing

ccproxy supports multiple access modes:

- **Direct routes**: `/v1/...`, `/v1beta/...`, `/api/...`
- **Grouped routes**: `/{group_name}/v1/...`
- **Dynamic switching**: `/switch/v1/...` resolves to the active proxy group
- **Compat mode**: `/compat/...` or `/compat_mode/...` enables tool compatibility for models without native tool calling

### 2. Multi-Protocol Adaptation

Requests flow through **Input -> Unified -> Backend -> Unified -> Output** adapters when protocol conversion is required.

### 3. OpenAI-Compatible API Surface

ccproxy exposes OpenAI-style endpoints including:

- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /v1/responses`
- `POST /v1/embeddings`

`/v1/responses` is now a first-class protocol path:

- Route variants include direct, grouped, `/switch`, and compat forms
- `handler/responses_handler.rs` is the entry point
- `adapter/input/openai_responses_input.rs` converts Responses requests into the unified request model when adaptation is needed
- `adapter/output/openai_responses_output.rs` converts unified responses and streams back into OpenAI Responses-compatible output
- If provider metadata sets `supports_responses_api` or `supportsResponsesApi`, ccproxy prefers direct forwarding to the upstream `/responses` endpoint
- Otherwise, ccproxy falls back to the unified chat pipeline and re-serializes the result as Responses output

### 4. Embedding Proxy

Standardized embedding endpoints:

- **OpenAI**: `/v1/embeddings`
- **Claude**: `/v1/claude/embeddings`
- **Gemini**: `:embedContent` or `/embedContent`
- **Ollama**: `/api/embed` and `/api/embeddings`

## How It Works

Requests enter protocol handlers and then follow one of two canonical paths:

1. **Direct forwarding**: used when client protocol matches backend protocol and compat mode is off
2. **Unified adaptation**: used when protocol conversion or compat behavior is required

In both cases, ccproxy still handles:

- model resolution
- auth and proxy header injection
- retry and stat recording
- protocol-safe response conversion
- header filtering through `filter_proxy_headers`

## Important Notes

- `router.rs` is order-sensitive. Keep the route hierarchy stable to avoid route shadowing.
- Static protocol prefixes must remain ahead of grouped dynamic routes.
- Transport headers such as `Content-Length`, `Transfer-Encoding`, `Connection`, and `Content-Encoding` must not be forwarded directly.
- For maintenance constraints and invariants, follow `CONSTITUTION.md`.
