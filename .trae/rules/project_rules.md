# Rules for Rust and Frontend Development

## Rust Code Guidelines

- **Avoid `unwrap` and `expect` in production code**, as they may cause a panic. Instead, return an appropriate error. These methods are acceptable in test code.
- **Allowed Cases for `unwrap` and `expect`**:
  1. **Compile-time Constants**: Examples include Regex patterns and static configurations. Their validity is determined at compile time, and they will not change at runtime.
  2. **Program Initialization**: Examples include global variables and static resource loading. If these fail, the program cannot run correctly, so panic is acceptable.
  3. **Test Code**: Test failures are part of the expected behavior.
- **Recommendation**: Prefer `expect` over `unwrap` in allowed cases, and provide clear error messages for better debugging.
- **Manage all prompt information using i18n**; do not hardcode strings directly into Rust code.
- **The `mod.rs` file is only for defining and exporting modules**.
- **Trait definitions** in Rust should be placed in `traits.rs`, and **struct definitions** can be placed in `types.rs` if there are many.
- **Refer to Tauri v2 documentation** before generating code, as there is a significant difference between Tauri v1 and v2.
- **Review Rust code syntax carefully** after generating code. Ensure it adheres to proper lifetimes and variable usage to avoid errors.
- **Rust documentation comments** should placed at file start and start with `//!`

## Frontend Development Guidelines

- **Use Vue 3** for the project frontend, and follow the **Composition API** approach.
- **Prefer `<script setup>` style** within the Vue pages.
- **Use Yarn** as the package manager for frontend development. Provide package installation instructions using Yarn commands.
- **Implement styles** using **Element Plus components and classes**.
- **Use SCSS nested syntax** for CSS code to ensure a clearer and more organized structure.

## Code Comments and Documentation

- **Generate method, function, and code block comments in English**. Avoid starting comments with phrases like "this function" or "this class". Ensure comments are clear and concise, providing context and explanations where necessary.
- **Function comments** should comprehensively cover aspects such as a detailed description of the function's purpose, definitions of parameters and return values, how exceptions are handled, and any relevant notes.
- **Do not remove necessary comments** from the original code.
- **All comments, including code comments, must be in English**.
