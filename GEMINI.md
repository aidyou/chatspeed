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
