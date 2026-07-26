# TAURI BACKEND

**Parent:** ../AGENTS.md

## OVERVIEW

Rust backend for ChatSpeed desktop app. Tauri v2 runtime with SQLite storage, MCP integration, and CCProxy AI engine.

## STRUCTURE

```
src-tauri/
├── src/
│   ├── lib.rs           # App setup, state registration, window lifecycle
│   ├── main.rs          # Binary entry (calls chatspeed_lib::run)
│   ├── ccproxy/         # AI proxy engine (multi-protocol)
│   ├── commands/        # IPC command handlers
│   ├── db/              # SQLite + Sled storage
│   ├── mcp/             # MCP client/server
│   ├── workflow/        # DAG workflow engine
│   ├── sensitive/       # Data filtering (PII redaction)
│   └── scraper/         # WebView-based web scraping
├── i18n/                # Backend localization files
├── Cargo.toml           # Rust dependencies
└── tauri.conf.json      # Tauri configuration
```

## WHERE TO LOOK

| Task | Location |
|------|----------|
| Add Tauri command | `src/commands/` → register in `lib.rs` invoke_handler |
| Database operations | `src/db/main_store.rs` |
| State management | `src/lib.rs` setup() section |
| Window creation | `src/window.rs` (emit events, don't create directly) |
| App lifecycle | `src/lib.rs` on_window_event handler |

## CONVENTIONS

- **Error handling**: `Result<T, AppError>` via `?`. No `unwrap()` in production.
- **i18n**: All user-facing strings via `t!` macro from `rust_i18n`
- **Module structure**: `mod.rs` for exports, `types.rs` for structs, `errors.rs` for errors
- **Commands**: Use `#[tauri::command]` macro, inject `State` for shared state

## ANTI-PATTERNS

| Pattern | Reason |
|---------|--------|
| Direct window creation in IPC | Deadlock on Windows; emit events instead |
| Unmanaged database connections | SQLite lock conflicts; database work must use the MainStore/DbRuntime facade. The runtime-owned single writer and controlled readers are allowed. |
| Hardcoded locale strings | Must use `t!` macro |

## KEY DEPENDENCIES

- `tauri` v2.9 — Desktop framework
- `rusqlite` — SQLite with WAL mode
- `axum` — HTTP server for CCProxy
- `rmcp` — MCP protocol implementation
- `tokio` — Async runtime
