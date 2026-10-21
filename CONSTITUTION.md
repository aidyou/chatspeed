# ChatSpeed Constitution

## Scope

These rules apply to logging and error diagnostics throughout the repository.
Read them together with `AGENTS.md` and the applicable module constitution.
Module-specific rules continue to protect their existing runtime, persistence,
and protocol contracts. This document does not authorize unrelated migrations.

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
