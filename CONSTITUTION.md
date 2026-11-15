# ChatSpeed Constitution

## Scope

These rules apply to platform compatibility, logging, and error diagnostics
throughout the repository.
Read them together with `AGENTS.md` and the applicable module constitution.
Module-specific rules continue to protect their existing runtime, persistence,
and protocol contracts. This document does not authorize unrelated migrations.

## Platform Compatibility

### 1. macOS 12 runtime support is mandatory

- Desktop releases must remain runnable on macOS 12, including Intel macOS 12.7.6.
  Do not raise the effective minimum OS version through dependencies, native
  symbols, frontend syntax, or build tools without explicit approval.
- Tauri uses the system WKWebView on macOS. A successful run in current Chrome,
  Node.js, or a newer Safari does not prove compatibility with macOS 12 WebKit.
  Treat development-server output and packaged frontend assets as separate
  compatibility paths; both must remain usable on the supported WebView.

### 2. Frontend modules must parse before capability detection can help

- Do not use regular-expression lookbehind (`(?<=...)` or `(?<!...)`) in
  browser-executed source, including regex literals and `RegExp` constructors.
  Use ordinary matching with explicit boundary checks or equivalent scanning.
  A literal inside an unused function still prevents the importing route from
  loading on older WebKit; a runtime guard around that function cannot fix it.
- Review third-party modules in the affected import chain for unsupported syntax
  and browser APIs. Lazy loading delays a parse failure; it does not make the
  dependency compatible. Do not assume Vite/esbuild targets rewrite lookbehind
  or polyfill missing runtime APIs.
- Compatibility rewrites must preserve matching semantics, punctuation,
  whitespace, offsets, and link boundaries as applicable. Add focused behavior
  regressions and a guard against reintroducing the unsupported syntax. Modern
  Node-only reference expressions may live in test files that are never shipped
  to the WebView.

### 3. Optional native capabilities must not prevent startup

- Newer macOS APIs must be dynamically resolved or demonstrably weak-linked,
  with runtime OS and capability checks before use. A version check alone does
  not remove a hard-linked symbol or make it available to an older SDK.
- For WebView proxies, keep Wry/Tauri's hard-linking `mac-proxy` / `macos-proxy`
  features disabled. Reuse the shared dynamic adapter, configure supported
  proxies before the first navigation, and retain system networking when the
  required APIs are unavailable. Keep Windows/Linux and Rust HTTP-client proxy
  behavior unchanged unless separately requested.
- Prefer retaining the ability to compile on macOS 12 where practical, but never
  trade away macOS 12 runtime support merely to simplify builds on newer SDKs.

### 4. Build dependencies and verification are part of compatibility

- Check the OS requirements of native executables shipped by build dependencies,
  not just their Node.js package requirements. The native Sass/Dart VM previously
  required macOS 14 and failed on macOS 12, leaving Vite with
  `Tried writing to closed dispatcher`. Preserve the JS-only Sass resolution
  unless a replacement is verified on the supported systems; check what Vite
  actually resolves, including optional peers, rather than only `package.json`.
- For compatibility changes, run focused regressions and inspect the resolved
  dependency/features as relevant. Verify macOS 12 startup and affected routes
  in both development and packaged builds when a suitable machine is available;
  verify newer-system behavior when adding a capability fallback. Explicitly
  report unavailable or skipped checks. Compilation, formatting, source guards,
  and tests on a modern engine alone do not prove old-system runtime support.

## Logging and Error Diagnostics

### 1. Levels and output routing

- Use `error` for a failed operation requiring diagnosis, `warn` for a recoverable
  abnormal condition or degraded capability, `info` for meaningful lifecycle or
  business milestones, and `debug`/`trace` for execution details. Do not log routine
  initialization, polling, payloads, or repeated progress at `info` merely for
  visibility.
- Keep first-party logs distinguishable from dependency logs by their original
  target. Normal project output may include `debug` during development and `info`
  in production; third-party `info`/`debug`/`trace` must not pollute normal console
  or application-file output. Preserve third-party `warn`/`error` diagnostics.
- Apply filtering consistently at the producer and forwarding/sink boundaries.
  Do not suppress only console output while leaving the same dependency noise in
  the application log file. Dedicated diagnostic targets such as
  `error_diagnostic` must remain observable even without a project-name prefix.
- Keep dedicated CCProxy request/response logs separate from the application
  log. Do not duplicate their payloads into normal operational logs or bypass
  their existing enablement and privacy controls.

### 2. Child-process forwarding

- Preserve the original level and target when forwarding structured runtime or
  dependency records. Never re-log every child stderr line as first-party `info`
  or promote it to `error` solely because it arrived on stderr.
- Treat a dependency's embedded level/target as log metadata only. Parsing it must
  not influence application status, retry policy, readiness, or exit behavior.
- Keep unstructured child stderr available for native crash diagnosis through an
  appropriate sanitized diagnostic channel; do not disguise unknown output as
  project `info` to bypass filters. Do not silently discard crash evidence.

### 3. Preserve causes; separate diagnostics from presentation

- Capture typed causes before converting errors to strings. Reuse
  `FailureMessage`, `ErrorOrigin`, and `ErrorProjection` under
  `src-tauri/crates/backend/src/error_diagnostics` rather than rebuilding a cause
  from `Display` text or stacking wrapper prefixes.
- Emit structured diagnostics through the existing `log_error_diagnostic` sink
  at an explicit operation/handling boundary. Include the originating cause and
  relevant module, operation, stable code/status, correlation identifiers, and
  bounded context/source chains when available. Do not invent missing context.
- Assign one owner for reporting a handled failure. Do not log the same full
  diagnostic at every conversion or rethrow. Retry logs may record attempt and
  outcome context without duplicating the entire cause chain on every layer.
- Keep developer diagnostics separate from the safe user projection
  (`UserErrorPresentation` / `user_error` / `presentation`, as appropriate to the
  existing boundary). Frontend display sites must reuse the shared error
  formatter, retain localization and safety fallbacks, and not expose internal
  wrappers or diagnostic chains. Live events, message cards, ledgers, and history
  reloads must use the same safe reason.
- Preserve legacy serialization, HTTP/IPC envelopes, provider pass-through,
  model-visible content, event ordering, persistence, and exit-code contracts.
  Improving logs or user feedback must not silently change these contracts.

### 4. Typed state and failure semantics

- Determine failure, retry, reconnect, cancellation, and lifecycle state from
  stable enums, booleans, status fields, or exact machine codes. Never infer them
  from localized messages, `Display`, wrapper prefixes, or an `Error:` substring.
- A string-based exception is allowed only at a third-party boundary with no
  reliable typed status or code. Isolate and document it there, with focused
  compatibility tests; do not propagate text matching into first-party logic.
- Logging severity does not determine fatality. A recoverable optional feature
  failure, such as a conflicting global shortcut, must not abort startup solely
  because it was logged as an error. Conversely, filtering a log must never hide
  or convert a real startup failure into a successful/ready state.

### 5. Privacy, bounds, and verification

- Sanitize before emission or forwarding, for both console and file output.
  Reuse existing redaction helpers; never rely on the final desktop formatter as
  the only defense. Do not log credentials, authorization headers, cookies,
  private keys, full prompts, or arbitrary raw request/response bodies in normal
  diagnostics. `debug` is not an exemption from privacy rules.
- Keep diagnostics bounded and machine-readable. Reuse the existing single-line
  JSON serializer, chain/depth/byte limits, cycle protection, and truncation
  marker. Do not truncate serialized JSON into an invalid fragment.
- For changes to these boundaries, add focused regressions for typed cause
  propagation, safe user projection, redaction/bounds, and level/target routing
  as applicable. Child-log changes must cover dependency initialization noise,
  preserved warnings/errors, and unstructured crash output. Verify the affected
  console/file sinks, not merely the formatter in isolation.

Implementation anchors: `src-tauri/src/logger.rs`,
`src-tauri/src/runtime_client.rs`, `src-tauri/crates/daemon/src/main.rs`,
`src-tauri/crates/backend/src/error_diagnostics`, and `src/libs/error.js`.
The implementation and verification ledger is
[`docs/development/error-feedback.md`](docs/development/error-feedback.md);
its historical checkpoints are evidence, not a replacement for these rules.
