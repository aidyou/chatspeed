# Project Development Guidelines & Standards

This document outlines the coding standards, architectural patterns, and workflow requirements for the Chatspeed project. All AI agents and developers must strictly adhere to these guidelines.

## 1. Core Principles
- **Maintainability**: Code must be clear, safe, readable, and professional.
- **Safety First**: Never introduce invalid syntax. Avoid potentially panicking code in production.
- **Context Awareness**: Always read the latest source code and configuration files (e.g., `Cargo.toml`, `package.json`) before making modifications.
- **Dependency Management**: Verify existing dependencies before adding new ones.

## 2. Backend Development (Rust & Tauri)
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

## 3. Frontend Development (Vue 3)
- **Framework**: Vue 3 with the **Composition API** and `<script setup>` syntax.
- **UI Library**: Use **Element Plus** components and utility classes.
- **Styling**: Write CSS using **SCSS nested syntax**.
- **Package Manager**: Use **Yarn** for all package management tasks.
- **TypeScript Specifics**:
  - Use `unknown` instead of `any` for variables with unknown types.
  - Declarations inside a `switch` case must be enclosed in curly braces `{}` to prevent scope issues.

## 4. Database Standards
- **Naming**: Use `snake_case` for all table and field names.
- **Clarity**: Avoid abbreviations in database schemas.

## 5. Documentation & Comments
- **Language**: All code comments and documentation must be written in **English**.
- **Comment Style**: 
  - Focus on *why* logic exists rather than *what* it does.
  - Avoid redundant phrases like "This function...".
  - Detail parameters, return values, and error handling for complex logic.
- **Release Logs**:
  - For `RELEASE.md` and `RELEASE.zh-CN.md`, the language switcher link (e.g., `[简体中文](./RELEASE.zh-CN.md) | English`) must always remain at the absolute top of the file.

## 6. Development Workflow
- **Standard Process**:
  1. **Plan**: Propose a plan or feasibility study.
  2. **Confirm**: Wait for user approval if the change is significant.
  3. **Execute**: Implement the changes adhering to all project conventions.
  4. **Verify**: Check for errors, linting issues, and verify via tests.
  5. **Correct**: Iteratively fix any issues found during verification.
  6. **Report**: Provide a concise summary of the resolution.
- **Git Protocol**: Do not proactively stage or commit changes unless explicitly requested by the user.
