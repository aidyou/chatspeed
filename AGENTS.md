@GEMINI.md

---

# PROJECT KNOWLEDGE BASE

**Generated:** 2026-02-17
**Commit:** bc693b1
**Branch:** main

## OVERVIEW

ChatSpeed — Open-source AI assistant desktop app. Rust/Tauri v2 backend + Vue 3 frontend. Core engine: CCProxy (multi-protocol AI proxy with tool compatibility mode).

## STRUCTURE

```
chatspeed/
├── src/                    # Vue 3 frontend (Composition API, Pinia, Element Plus)
│   ├── main.js             # Frontend entry point
│   ├── stores/             # Pinia state management (12 stores)
│   ├── components/         # UI components
│   ├── pkg/workflow/       # Workflow engine & tool registry
│   └── libs/               # Frontend utilities
├── src-tauri/              # Rust backend (Tauri v2)
│   └── src/
│       ├── lib.rs          # Backend entry, state registration, window lifecycle
│       ├── ccproxy/        # AI proxy engine (OpenAI/Claude/Gemini/Ollama)
│       ├── commands/       # Tauri IPC command handlers
│       ├── db/             # SQLite + Sled storage layer
│       ├── mcp/            # MCP client/server implementation
│       └── workflow/       # DAG-based workflow engine
├── index.html              # SPA entry HTML
└── vite.config.js          # Vite build config
```

## WHERE TO LOOK

| Task | Location | Notes |
|------|----------|-------|
| Add new AI model support | `src-tauri/src/ccproxy/adapter/backend/` | Protocol adapters |
| Fix IPC command | `src-tauri/src/commands/` | See mod.rs for all commands |
| Add frontend state | `src/stores/` | Pinia stores, useXStore pattern |
| MCP integration | `src-tauri/src/mcp/` | Client + server |
| Routing issues | `src-tauri/src/ccproxy/router.rs` | DO NOT modify order |
| Workflow tools | `src/pkg/workflow/tools/` | ToolDefinition pattern |

## CONVENTIONS

### Rust Backend
- `mod.rs` only contains `mod` declarations + `pub use` re-exports
- Public structs �� `types.rs`, traits → `traits.rs`, errors → `errors.rs`
- Error handling: Use `Result<T, AppError>` via `?` operator. Never `unwrap()`/`expect()` in production
- All user strings via i18n (`t!` macro). No hardcoded strings.
- File comments: `//!` for file-level, `///` for items

### Frontend
- Vue 3 Composition API + `<script setup>`
- Use `unknown` instead of `any` for unknown types
- Switch case declarations must be in `{}` blocks
- Pinia stores: `useXStore` naming, `defineStore('x', () => {})` pattern

### Formatting
- TS target: ES2022, strict mode enabled
- Prettier: semi=false, singleQuote=true, printWidth=100, trailingComma=none

## ANTI-PATTERNS (THIS PROJECT)

| Pattern | Location | Reason |
|---------|----------|--------|
| Route order modification | `ccproxy/router.rs` | Causes route shadowing, breaks API clients |
| `unwrap()`/`expect()` | Production code | Use `?` or `unwrap_or_default()` |
| Hardcoded i18n strings | Rust code | Must use `t!` macro |
| Double database open | `db/` | Causes lock conflicts, use single connection |
| Direct window creation from IPC | `commands/window.rs` | Deadlock risk on Windows; emit events instead |
| SSE endpoint usage | MCP integration | Deprecated v1.3.0; use `/mcp/http` |

## UNIQUE STYLES

- **Two-entry architecture**: SPA frontend (`src/main.js`) + Tauri desktop wrapper (`src-tauri/src/main.rs`)
- **CCProxy**: Tri-layer adapter (Input → Backend → Output) for protocol conversion
- **Tool Compat Mode**: Injects prompts + XML parsing for non-native tool-calling models
- **Cross-window sync**: `sendSyncState` + `handleSyncStateUpdate` pattern (see MCP store)

## COMMANDS

```bash
# Development
yarn dev                    # Vite dev server only
yarn tauri dev              # Full Tauri dev (frontend + backend)
make dev                    # Standardized dev entry

# Build
yarn tauri build            # General build
make build-mac              # macOS universal binary
make build-win              # Windows (requires vcpkg)
make build-linux            # Linux

# Test
cargo test --manifest-path src-tauri/Cargo.toml   # Rust tests
```

## NOTES

- **Database**: SQLite with WAL mode + busy_timeout (5s). Single connection per process.
- **Deprecation**: `/mcp/sse` removed in v2.0.0 → use `/mcp/http`
- **Cross-protocol safety**: Use `i32` for index fields (some providers return `-1`)
- **Gemini naming**: All fields use `camelCase` (`#[serde(rename_all = "camelCase")]`)