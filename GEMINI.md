# ChatSpeed Project Context

ChatSpeed is an open-source, high-performance AI assistant and smart desktop hub. It is built using **Rust** for the core engine and **Vue 3** with **Tauri v2** for the desktop application.

## Project Overview

- **Core Engine (`CCProxy`)**: A universal adapter written in Rust that handles protocol conversion (OpenAI, Claude, Gemini, Ollama), model enhancement (via prompt injection), and capability expansion (tool compatibility mode for models without native tool calling).
- **MCP Hub**: Centrally manages Model Context Protocol (MCP) tools and exposes them via a stable "Streamable HTTP" protocol.
- **Desktop Assistant**: Features a quick-summon interface (ALT+Z) for translation, mind maps, flowcharts, and general AI interaction.
- **Vision Support**: Supports image and file analysis by integrating vision models.

## Technology Stack

- **Frontend**: Vue 3 (Composition API), Pinia (State Management), Vue Router, Element Plus (UI Component Library).
- **Backend**: Rust, Tauri v2.
- **Local Server**: Axum (Web server for proxying and API conversion).
- **Database/Storage**: SQLite (`rusqlite`) and Sled (KV storage).
- **Styling**: SCSS, Element Plus.

## Building and Running

### Prerequisites

- **Node.js**: v22+ recommended.
- **Yarn**: Used for frontend package management.
- **Rust**: Stable toolchain.
- **vcpkg**: Required on Windows for native dependencies (`sqlite3`, `bzip2`).

### Key Commands

- **Development**:
  - `yarn dev`: Starts the Vite development server for the frontend.
  - `yarn tauri dev`: Starts the Tauri development environment (frontend + backend).
  - `scripts/win_dev.bat`: (Windows) Automatically configures the environment and starts `yarn tauri dev`.
  - `make dev`: Standardized development entry via Makefile.
- **Building**:
  - `yarn tauri build`: General build command.
  - `make build-win`: Build for Windows (requires environment setup).
  - `make build-mac`: Build for macOS (Universal binary).
  - `make build-linux`: Build for Linux.
- **Environment Setup**:
  - `scripts/setup-env.ps1`: (Windows PowerShell) Detects Visual Studio, Windows SDK, and configures `vcpkg` for static linking.

## Project Structure

- `src/`: Vue 3 frontend source code.
  - `components/`: UI components (chat, common, icon, setting, window, workflow).
  - `stores/`: Pinia stores for application state.
  - `libs/`: Frontend utility libraries (Tauri integration, file parsing, markdown parsing).
- `src-tauri/`: Rust backend source code.
  - `src/`: Main Rust application logic.
  - `i18n/`: Backend-managed internationalization files.
- `scripts/`: Platform-specific build and environment scripts.
- `tools/`: Python and shell scripts for i18n checking and icon generation.

## Project Development Guidelines & Standards

This section outlines the coding standards, architectural patterns, and workflow requirements for the Chatspeed project. All AI agents and developers must strictly adhere to these guidelines.

### 1. Core Principles
- **Maintainability**: Code must be clear, safe, readable, and professional.
- **Safety First**: Never introduce invalid syntax. Avoid potentially panicking code in production.
- **Context Awareness**: Always read the latest source code and configuration files (e.g., `Cargo.toml`, `package.json`) before making modifications.
- **Dependency Management**: Verify existing dependencies before adding new ones.

### 2. Backend Development (Rust & Tauri)
- **Framework**: Use **Tauri v2** standards and documentation.
- **Error Handling**: 
  - Strictly avoid `unwrap()` and `expect()` in production code. Use `Result` and the `?` operator.
  - `unwrap_or()` and `unwrap_or_default()` are permitted.
  - `unwrap()` and `expect()` is allowed only in test code, regex compilation, or fatal program initialization. When used, provide a clear error message.
- **Module Structure**:
  - `mod.rs` should only contain module declarations (`mod my_module;`) and exports (`pub use ...`).
  - Public structs should be in `types.rs`.
  - Public traits should be in `traits.rs`.
  - Public errors should be in `errors.rs`.
- **Naming Conventions**:
  - Variables and Functions: `snake_case`.
  - Structs and Enums: `PascalCase`.
- **Internationalization (i18n)**: 
  - All user-facing strings must be managed via the `i18n` crate. 
  - **No hardcoded strings** are allowed in Rust source code.
- **Documentation**:
  - File-level: Use `//!` at the beginning of the file.
  - Item-level (functions/structs): Use `///`.

### 3. Frontend Development (Vue 3)
- **Framework**: Vue 3 with the **Composition API** and `<script setup>` syntax.
- **UI Library**: Use **Element Plus** components and utility classes.
- **Styling**: Write CSS using **SCSS nested syntax**.
- **Package Manager**: Use **Yarn** for all package management tasks.
- **TypeScript Specifics**:
  - Use `unknown` instead of `any` for variables with unknown types.
  - Declarations inside a `switch` case must be enclosed in curly braces `{}` to prevent scope issues.

### 4. Database Standards
- **Naming**: Use `snake_case` for all table and field names.
- **Clarity**: Avoid abbreviations in database schemas.

### 5. Documentation & Comments
- **Language**: All code comments and documentation must be written in **English**.
- **Comment Style**: 
  - Focus on *why* logic exists rather than *what* it does.
  - Avoid redundant phrases like "This function...".
  - Detail parameters, return values, and error handling for complex logic.
- **Release Logs**:
  - For `RELEASE.md` and `RELEASE.zh-CN.md`, the language switcher link (e.g., `[简体中文](./RELEASE.zh-CN.md) | English`) must always remain at the absolute top of the file.

### 6. Development Workflow
- **Standard Process**:
  1. **Plan**: Propose a plan or feasibility study.
  2. **Confirm**: Wait for user approval if the change is significant.
  3. **Execute**: Implement the changes adhering to all project conventions.
  4. **Verify**: Check for errors, linting issues, and verify via tests.
  5. **Correct**: Iteratively fix any issues found during verification.
  6. **Report**: Provide a concise summary of the resolution.
- **Git Protocol**: Do not proactively stage or commit changes unless explicitly requested by the user.

### 7. Database Concurrency & Deadlock Risks
To ensure stability in production environments where background services (like CCProxy) and the main UI may access the database simultaneously, follow these rules:
- **Enable WAL Mode**: Always use `PRAGMA journal_mode=WAL;` to allow multiple readers and one writer to coexist without blocking.
- **Busy Timeout**: Always set a `busy_timeout` (e.g., 5 seconds) on the SQLite connection to handle transient locks gracefully.
- **Avoid "Double Open"**: Ensure the database file is opened only once per process. Redundant calls to `Connection::open` on the same file can lead to `readonly database` errors or file corruption in production.
- **Atomic Restoration**: When performing physical file replacement (restoration):
    1. **Disconnect**: Close all active connections (or swap with an in-memory DB) to release file handles.
    2. **Cleanup**: Delete temporary `-wal` and `-shm` files to prevent startup crashes due to file mismatch.
    3. **Replace**: Physically move/rename the new database file.
    4. **Reconnect**: Establish a fresh connection and re-enable WAL mode.
- **Locking Hierarchy**: Be mindful of the `Arc<RwLock<MainStore>>` (Outer) and `Mutex<Connection>` (Inner) hierarchy. Ensure that restoration and high-concurrency operations are performed as single atomic blocks within the outer lock to prevent Rust-level deadlocks.

### 8. Proxy Routing Stability & Priority
The routing structure in `src-tauri/src/ccproxy/router.rs` is highly sensitive to ordering. To prevent **Route Shadowing** (where a dynamic route captures a static one):
- **Strict Priority**: Fixed protocol prefixes (e.g., `/v1`, `/v1beta`, `/api`, `/mcp`) **MUST** be registered before any dynamic capture groups (e.g., `/{group_name}`).
- **No Structural Changes**: Do not modify the hierarchical order of route mounting without performing a full protocol test matrix. Breaking this order will cause standard clients (like OpenAI SDKs) to receive 404 or connection errors.
- **Verification**: Always run the API test suite (`python3 test/ccproxy/api_all_test.py`) after any routing or handler changes.

### 9. I18N Key Organization & Sorting
To maintain consistency and reduce merge conflicts in `src/i18n/locales/` and `src-tauri/i18n/`:
- **Alphabetical Sorting**: All keys must be sorted **alphabetically (natural order)** at every nesting level.
- **Grouping**: Keys should be grouped logically (e.g., `chat`, `settings`, `menu`), and within each group, the sub-keys must follow alphabetical order.
- **Consistency**: When adding a new key to one language, ensure it is added to all other supported languages in the same sorted position.

### 10. AI Proxy Protocol & Header Handling
To prevent connection drops and protocol errors (e.g., "Empty reply from server"):
- **Header Filtration**: When forwarding responses from an AI backend, always use the `filter_proxy_headers` utility. Never forward `Content-Length`, `Transfer-Encoding`, `Connection`, or `Content-Encoding` directly, as these must be managed by the local server framework (Axum/Hyper).
- **Metadata Passthrough**: Always preserve business-critical headers like `x-ratelimit-*`, `retry-after`, and internal IDs (`X-CS-*`).
- **Direct Forwarding**: Even in high-performance pass-through mode, headers must be filtered to ensure compatibility with the frontend's keep-alive and decompression logic.

### 11. Cross-Protocol Type Safety
To avoid panics when parsing non-standard or third-party AI responses:
- **Signed Indices**: Always use `i32` (instead of `u32` or `usize`) for fields like `index` in tool calls or embedding vectors. Some providers (e.g., Qwen, OneAPI) may return `-1` for indices, which will cause a deserialization failure if mapped to an unsigned type.
- **Optional Fields**: Use `Option<T>` for any field that is not strictly guaranteed by all supported protocols (e.g., `finish_reason`, `usage`, `logprobs`).

### 12. Protocol Field Mapping (CamelCase vs SnakeCase)
Be aware of the naming conventions required by different protocols:
- **Gemini**: Strictly uses **`camelCase`** for almost all fields (e.g., `functionDeclarations`, `usageMetadata`). When parsing Gemini input or generating Gemini output, ensure the structures are correctly tagged with `#[serde(rename_all = "camelCase")]`.
- **OpenAI/Ollama**: Generally use **`snake_case`**.
- **Adapter Responsibility**: Adapters must explicitly handle the translation between these naming styles during the `UnifiedRequest` / `UnifiedResponse` conversion.

### 13. Logging & Observability
To ensure clear debugging during cross-protocol proxying:
- **Labeling**: Logs should clearly distinguish between data source stages. Use `[Backend Raw Response]` for original data from the upstream provider and `[Adapted Response]` for the final data sent to the client.
- **Sensitive Data**: Avoid logging full API keys or raw sensitive user data in production logs.
