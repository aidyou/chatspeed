# Database Persistence Rules

- Do not explicitly assign a TSID or any other value that may exceed JavaScript's safe-integer range (`Number.MAX_SAFE_INTEGER`) to an integer auto-increment ID column, except for a documented exceptional requirement.
- Let SQLite allocate values for auto-increment primary keys. Persist TSIDs separately only when the schema and public contract explicitly require them.
- Large integer IDs can lose precision when they cross the Rust/SQLite-to-frontend boundary. Use string serialization for any externally exposed identifier that cannot be safely represented as a JavaScript number.

## Naming

- Use `snake_case` for table and column names.
- Avoid abbreviations in the schema; spell names out.

## Concurrency and Deadlock Risks

CCProxy and the UI reach the database at the same time, so every access goes through the
runtime facade:

- Talk to the database through `MainStore` and its `DbRuntime` only. The runtime owns the single writer thread and a fixed set of reader connections; a `Connection` opened next to it brings back exactly the locks and `readonly database` failures the facade exists to prevent.
- Keep WAL mode (`PRAGMA journal_mode=WAL`) and a `busy_timeout` on every connection the runtime opens, including the ones opened while a file is replaced.
- Reader connections are opened `query_only`. Every write belongs to the runtime writer, never to a read path.
- Touch the files themselves only through the maintenance path — `checkpoint_for_maintenance`, `pause_runtime`, `resume_runtime` — which the maintenance lock serializes against in-flight jobs.
- Restore through `MainStore::atomic_restore`: it preserves the machine-specific keys and rolls back to the previous file when the replacement cannot be finalized. Do not hand-roll a file swap.
- Keep the work inside one runtime job short. A slow enqueue or a slow job is reported as a warning, and every other database caller waits behind it.
