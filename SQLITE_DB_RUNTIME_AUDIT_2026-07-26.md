# SQLite DbRuntime Audit

Authority: approved plan supplied from workflow session `0r3cp0fxc0400`.

## Completed In This Pass

- Removed production CCProxy statistics commands that synchronously waited on `read_blocking` and moved every command to the existing async DbRuntime reader path without changing SQL, IPC names, parameters, or response fields.
- Removed duplicate, unreferenced synchronous DAO wrappers for note and chat CRUD. The async runtime variants remain the only production paths.
- Kept workflow synchronous helpers only under `cfg(test)` where test compilation proved they are still fixture dependencies. They are excluded from production.
- Preserved `CcproxyQuery.debug` as a deserialized external compatibility parameter. It has no behavioral consumer, but removing it would change accepted HTTP query shape; the targeted lint exemption documents this boundary.
- Extended maintenance drain to wait for all reader workers after telemetry and writer queues have drained, so checkpoint/restore cannot overlap an active reader job.
- Added low-noise DbRuntime observability for slow queue enqueue, slow reader/writer jobs, telemetry batch persistence, and worker shutdown.
- Added a reproducible ignored benchmark covering 100k, 500k, and 1m statistics rows. It asserts the range predicate uses `idx_ccproxy_stats_request_at` and that legacy local-date filtering returns the same count as the UTC half-open range.

## Verified

- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
- `git diff --check`
- `cargo check --manifest-path src-tauri/Cargo.toml` with no project warning
- `rg` confirms removed production sync wrappers have no remaining definitions.

## Remaining Acceptance Evidence

- Rust test runner in this workspace starts the selected `chatspeed_lib` test binary but does not return its final test summary or `--nocapture` benchmark output to the current terminal integration. The process exits afterward. Re-run the ignored benchmark in a normal terminal to capture host-specific latency evidence:

  `cargo test --manifest-path src-tauri/Cargo.toml --lib db::ccproxy::tests::statistics_range_benchmark_reports_indexed_query_evidence -- --exact --ignored --nocapture`

- `pnpm test:workflow`, `pnpm build`, the CCProxy API suite, and Windows pressure validation remain required by V-8. Windows must remain explicitly unverified until exercised on Windows; do not claim that the original Windows freeze is fully resolved.
