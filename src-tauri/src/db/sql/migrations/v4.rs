/// Version 4 migration SQL statements
/// Adds Proxy Stats table for tracking proxy requests and token usage.
pub const MIGRATION_SQL: &[(&str, &str)] = &[
    // Proxy stats table
    (
        "ccproxy_stats",
        "CREATE TABLE IF NOT EXISTS ccproxy_stats (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            client_model TEXT NOT NULL,
            backend_model TEXT NOT NULL,
            provider TEXT NOT NULL,
            protocol TEXT NOT NULL,
            tool_compat_mode INTEGER DEFAULT 0,
            status_code INTEGER NOT NULL,
            error_message TEXT,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            cache_tokens INTEGER DEFAULT 0,
            request_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )"
    ),
    (
        "idx_ccproxy_stats_request_at",
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_request_at ON ccproxy_stats(request_at DESC)"
    ),
    (
        "idx_ccproxy_stats_provider",
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_provider ON ccproxy_stats(provider)"
    ),
    (
        "idx_ccproxy_stats_status_code",
        "CREATE INDEX IF NOT EXISTS idx_ccproxy_stats_status_code ON ccproxy_stats(status_code)"
    ),
];
