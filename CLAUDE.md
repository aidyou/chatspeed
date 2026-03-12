# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ChatSpeed is an open-source AI assistant desktop application built with Rust (Tauri v2) backend and Vue 3 frontend. Its core engine is CCProxy, a multi-protocol AI proxy that enables tool compatibility mode for models without native tool-calling support.

## Architecture

### Two-Entry Architecture
- **Frontend Entry**: `src/main.js` - Vue 3 SPA with Composition API
- **Backend Entry**: `src-tauri/src/main.rs` / `src-tauri/src/lib.rs` - Tauri desktop wrapper

### Key Components

| Component | Location | Purpose |
|-----------|----------|---------|
| CCProxy | `src-tauri/src/ccproxy/` | AI proxy engine (OpenAI/Claude/Gemini/Ollama protocol conversion) |
| Workflow Engine | `src-tauri/src/workflow/react/` | ReAct-based autonomous agent system |
| MCP Hub | `src-tauri/src/mcp/` | MCP client/server implementation |
| Database | `src-tauri/src/db/` | SQLite + Sled storage layer |
| Commands | `src-tauri/src/commands/` | Tauri IPC command handlers |
| Frontend Stores | `src/stores/` | Pinia state management |

## Development Commands

```bash
# Development (requires Rust toolchain + Node.js)
make dev                    # Standard dev entry (copies lang files + starts Vite dev server)
yarn tauri dev              # Full Tauri dev (frontend + backend)
yarn dev                    # Vite dev server only (frontend only)

# Building
make build-mac              # macOS universal binary
make build-win              # Windows (requires vcpkg - see below)
make build-linux            # Linux
yarn tauri build            # General build for current platform

# Testing
cargo test --manifest-path src-tauri/Cargo.toml   # Run Rust tests
cargo test --manifest-path src-tauri/Cargo.toml --lib -- <test_name>  # Run single test

# Utilities
make copy-lang-file         # Sync i18n files from Rust to frontend
make clean                  # Clean build artifacts
```

## Project Conventions

### Rust Backend

#### Module Structure
`mod.rs` contains **only** `mod` declarations and `pub use` re-exports. Implementation goes in separate files.

```rust
// In mod.rs - ONLY declarations and re-exports
mod types;
mod errors;
pub use types::{Foo, Bar};
pub use errors::MyError;
```

#### Code Organization
- Public structs/enums → `types.rs`
- Traits → `traits.rs`
- Module-specific errors → `errors.rs`
- Implementation → named files (e.g., `handler.rs`, `router.rs`)

#### Error Handling
Use the unified `AppError` type via `?` operator. All module-specific errors implement `From` to convert to `AppError`.

```rust
// In error.rs for each module
#[derive(Error, Debug, Serialize)]
pub enum MyModuleError { ... }

// In root error.rs
#[derive(Error, Debug, Serialize)]
#[serde(tag = "module", content = "details")]
pub enum AppError {
    #[error(transparent)]
    MyModule(#[from] crate::mymodule::MyModuleError),
    ...
}
```

- **Never** use `unwrap()`/`expect()` in production code
- Use `?` or `unwrap_or_default()` for error propagation

#### Internationalization
All user-facing strings via i18n (`t!` macro). No hardcoded strings.

```rust
use rust_i18n::t;
let msg = t!("module.error_message", param = value);
```

#### Documentation
- `//!` for file-level documentation
- `///` for item-level documentation

### Frontend (Vue 3)

#### API Style
Composition API with `<script setup>`

#### Type Safety
Use `unknown` instead of `any` for unknown types

#### Switch Cases
Must use `{}` blocks:
```typescript
switch (value) {
  case 'a': {
    // code
    break
  }
}
```

#### Store Pattern
Pinia stores use `useXStore` naming with `defineStore('x', () => {})`:

```typescript
export const useChatStore = defineStore('chat', () => {
  const conversations = ref([])
  // ...
  return { conversations, loadConversations }
})
```

### Code Style
- **TypeScript**: Target ES2022, strict mode enabled
- **Prettier**: semi=false, singleQuote=true, printWidth=100, trailingComma=none

## Critical Anti-Patterns (DO NOT DO)

| Pattern | Location | Reason |
|---------|----------|--------|
| Route order modification | `ccproxy/router.rs` | Causes route shadowing, breaks API clients |
| `unwrap()`/`expect()` | Production code | Use `?` or `unwrap_or_default()` |
| Hardcoded i18n strings | Rust code | Must use `t!` macro |
| Double database open | `db/` | Causes lock conflicts; use single connection |
| Direct window creation from IPC | `commands/window.rs` | Deadlock risk on Windows; emit events instead |
| SSE endpoint usage | MCP integration | Deprecated v1.3.0; use `/mcp/http` (Streamable HTTP) |
| State registration after listeners | `lib.rs` | Event handlers panic when accessing unmanaged state |

## Platform-Specific Build Notes

### Windows (vcpkg required)
```powershell
# Install vcpkg dependencies
vcpkg install

# Set environment variables
$env:VCPKG_DEFAULT_TRIPLET="x64-windows-static"
$env:RUSTFLAGS="-Ctarget-cpu=x86-64 -Ctarget-feature=+crt-static"
```

### macOS
```bash
# Install dependencies
brew install sqlite3 bzip2
```

### Linux (Ubuntu/Debian)
```bash
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf sqlite3 libsqlite3-dev
```

## Key Technical Details

### Database
- SQLite with WAL mode + busy_timeout (5s)
- Single connection per process
- Dev path: `src-tauri/.chatspeed/dev_data/chatspeed.db`
- Prod path: OS-specific app data directory

### Development Data Directory
In development, all data (database, logs, configs) is stored in `src-tauri/.chatspeed/dev_data/` to keep the OS app data directory clean. Controlled by `STORE_DIR` in `constants.rs`.

### HTTP Server Ports
| Service | Default Port | Config Key |
|---------|--------------|------------|
| Internal HTTP Server | 21912 | Auto-finds available if taken |
| CCProxy | 11435 | `CFG_CCPROXY_PORT` |

### CCProxy Protocol Support
- Input: OpenAI-compatible, Claude, Gemini, Ollama
- Output: OpenAI-compatible chat completions
- Tool compatibility mode injects prompts + XML parsing for non-native tool-calling models

### State Registration Order (Critical)

States **must** be registered in this exact order in `lib.rs` setup:

1. **MainStore** (SQLite) - Required by all other states
2. **ChatState** - Depends on MainStore
3. **FilterManager** - Self-contained
4. **ScraperPool** - Depends on AppHandle
5. **UpdateManager** - Depends on AppHandle
6. **TsidGenerator** - Self-contained
7. **TauriGateway** - Self-contained
8. **SubAgentFactory** - Depends on all above

**Why order matters**: Event listeners registered after setup may immediately access states via `handle.state()`. If states aren't registered yet, this causes a panic.

### Window Management
- Windows defined in `tauri.conf.json` with `"create": false` (prevents auto-creation)
- Manually created in `lib.rs` setup to prevent race conditions
- Windows: main, assistant, workflow, proxy_switcher, settings, note
- Assistant window has special focus/hide behavior with timers (see `lib.rs` window event handlers)

### MCP Endpoints
- `/mcp/http` - Streamable HTTP (recommended, stable)
- `/mcp/sse` - **DEPRECATED** (removing in v1.3.0)

## Testing

Rust tests use `tauri::test::mock_builder()` for mocking. See `src-tauri/src/test.rs` for setup.

```rust
#[cfg(test)]
mod tests {
    use crate::test::get_app_handle;

    #[test]
    fn test_something() {
        let app = get_app_handle();
        // test code
    }
}
```

Tests can be run with the test helper that sets up the mock app handle with all required states.

## Where to Look

| Task | Location |
|------|----------|
| Add AI model support | `src-tauri/src/ccproxy/adapter/backend/` |
| Fix IPC command | `src-tauri/src/commands/` (see mod.rs for all commands) |
| Add frontend state | `src/stores/` - use `useXStore` pattern |
| MCP integration | `src-tauri/src/mcp/` |
| Routing issues | `src-tauri/src/ccproxy/router.rs` - DO NOT modify order |
| Workflow tools | Frontend: `src/pkg/workflow/tools/`, Backend: `src-tauri/src/tools/` |
| i18n strings | `src-tauri/i18n/` (Rust), `src/i18n/locales/` (frontend) |
| Error types | Module: `src-tauri/src/<module>/errors.rs`, Unified: `src-tauri/src/error.rs` |
| Constants | `src-tauri/src/constants.rs` |
