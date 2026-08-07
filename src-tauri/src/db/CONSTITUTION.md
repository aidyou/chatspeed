# Database Persistence Rules

- Do not explicitly assign a TSID or any other value that may exceed JavaScript's safe-integer range (`Number.MAX_SAFE_INTEGER`) to an integer auto-increment ID column, except for a documented exceptional requirement.
- Let SQLite allocate values for auto-increment primary keys. Persist TSIDs separately only when the schema and public contract explicitly require them.
- Large integer IDs can lose precision when they cross the Rust/SQLite-to-frontend boundary. Use string serialization for any externally exposed identifier that cannot be safely represented as a JavaScript number.
