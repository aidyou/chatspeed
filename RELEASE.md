[简体中文](./RELEASE.zh-CN.md) ｜ [English](./RELEASE.md)

# Release Notes

## [1.2.5]

### 🪄 Improvements

- **Startup Robustness & Fail-safe**: Implemented a "No-Panic" initialization strategy. If the database fails to load due to file system permissions or corruption, the app now gracefully falls back to an in-memory database. This ensures the application can still launch and provide diagnostic info instead of crashing during the boot sequence.

### 🐞 Bug Fixes

- **Windows 11 Startup Fix**: Resolved a critical panic on Windows 11 by reverting certain SQLite open flags (`SQLITE_OPEN_FULL_MUTEX`) that conflicted with specific threading models in production builds.

---

## [1.2.4]

### 🪄 Improvements

- **Database Architecture Hardening**: Upgraded the database engine to use **WAL (Write-Ahead Logging)** mode and implemented a 5-second **Busy Timeout**. These changes significantly improve concurrency, allowing the background proxy (CCProxy) and the main chat interface to access the database simultaneously without locking issues.
- **Data Integrity for Backups**: Implemented a mandatory **SQL Checkpoint (TRUNCATE)** before any backup operation. This ensures that all latest messages (previously held in the `-wal` log file) are properly flushed to the main database file before encryption, guaranteeing that your backups always contain the most recent data.
- **Atomic Restoration with State Preservation**: Completely refactored the database restoration process. The system now performs a safe, atomic "disconnect-replace-reconnect" sequence that prevents file corruption. It also preserves machine-specific settings (window positions, sizes, and network proxy configurations) during restoration, ensuring a seamless experience when migrating data.
- **Mission-Critical Stability Hardening**: Performed a comprehensive audit and refactoring of the application's startup sequence. All hardcoded `.expect()` and `.unwrap()` calls in critical paths (logging, database path resolution, and window management) have been replaced with graceful error handling. This ensures the application remains operational even in highly restricted environments like Windows Server 2019.
- **Graceful Logging Fallback**: The logging system now automatically degrades to console-only output if the designated log directory is unwritable or inaccessible, preventing immediate startup crashes.

### 🐞 Bug Fixes

- **Production Database Locking**: Resolved a critical "attempt to write a readonly database" error in production environments caused by redundant file handle requests during initialization.
- **Proxy Routing Precedence**: Resolved a critical routing conflict where generic group paths (e.g., `/{group}/...`) could incorrectly intercept specific functional prefixes like `/switch`, `/compat`, or `/compat_mode`. This fix ensures correct dispatching for all access modes and resolves 404 errors when using combined paths.
- **Compatibility Mode Alias**: Introduced the `compat` shorthand alias as a convenient alternative to `compat_mode` (e.g., `/group/compat/v1/messages`), improving API call ergonomics.
- **Proxy Statistics Calibration**: Fixed a critical issue where output tokens were reported as 0 in `tool_compat_mode` and `direct_forward` modes. The system now accurately estimates tokens for **Reasoning/Thinking content** and **Tool Call Arguments** across all supported protocols (OpenAI, Claude, Gemini, Ollama).
- **Unified Cache Token Tracking**: Implemented a standardized mapping for cached tokens. Cached data from various protocols (e.g., OpenAI's `prompt_cached_tokens`, Claude's `cache_read_input_tokens`, and Gemini's `cached_content_tokens`) is now correctly captured in stream logs and persisted to the database.
- **Path Resolution Panic**: Resolved potential panics during environment detection when the current working directory or application data directory cannot be resolved by the OS.
- **Window Handler Race Condition**: Added safety checks to window event listener registration to prevent crashes during the early initialization phase.

---

## [1.2.2]