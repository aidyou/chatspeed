use rusqlite::params;
use crate::db::{error::StoreError, types::CcproxyStat, MainStore};

impl MainStore {
    /// Records a new proxy statistic entry in the database.
    pub fn record_ccproxy_stat(&self, stat: CcproxyStat) -> Result<i64, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        
        match conn.execute(
            "INSERT INTO ccproxy_stats (client_model, backend_model, provider, protocol, tool_compat_mode, status_code, error_message, input_tokens, output_tokens, cache_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                stat.client_model,
                stat.backend_model,
                stat.provider,
                stat.protocol,
                stat.tool_compat_mode,
                stat.status_code,
                stat.error_message,
                stat.input_tokens,
                stat.output_tokens,
                stat.cache_tokens,
            ],
        ) {
            Ok(_) => Ok(conn.last_insert_rowid()),
            Err(e) => {
                log::error!("Failed to insert into ccproxy_stats: {}", e);
                Err(StoreError::Query(e.to_string()))
            }
        }
    }

    /// Retrieves daily proxy statistics for a specific date range.
    /// Returns a list of daily summaries.
    pub fn get_ccproxy_daily_stats(&self, days: i32) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        
        let mut stmt = conn.prepare(
            "SELECT 
                DATE(request_at) as date,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(cache_tokens), 0) as total_cache_tokens,
                COUNT(DISTINCT provider) as provider_count,
                COUNT(*) FILTER (WHERE status_code != 200) as error_count,
                COALESCE((SELECT client_model FROM ccproxy_stats s2 WHERE DATE(s2.request_at) = DATE(s1.request_at) GROUP BY client_model ORDER BY COUNT(*) DESC LIMIT 1), '-') as top_model
             FROM ccproxy_stats s1
             WHERE request_at >= DATE('now', '-' || ?1 || ' days')
             GROUP BY date
             ORDER BY date DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([days], |row| {
            Ok(serde_json::json!({
                "date": row.get::<_, String>(0)?,
                "totalInputTokens": row.get::<_, i64>(1).unwrap_or(0),
                "totalOutputTokens": row.get::<_, i64>(2).unwrap_or(0),
                "totalCacheTokens": row.get::<_, i64>(3).unwrap_or(0),
                "providerCount": row.get::<_, u32>(4).unwrap_or(0),
                "errorCount": row.get::<_, u32>(5).unwrap_or(0),
                "topProvider": row.get::<_, String>(6).unwrap_or_else(|_| "-".to_string()),
            }))
        }).map_err(|e| StoreError::Query(e.to_string()))?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
        }
        Ok(stats)
    }

    /// Aggregates backend model usage for a specific day range.
    pub fn get_ccproxy_model_usage_stats(&self, days: i32) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT backend_model, COUNT(*) as count 
             FROM ccproxy_stats 
             WHERE request_at >= DATE('now', '-' || ?1 || ' days')
             GROUP BY backend_model
             ORDER BY count DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([days], |row| {
            Ok(serde_json::json!({
                "type": row.get::<_, String>(0)?,
                "value": row.get::<_, u32>(1)?,
            }))
        }).map_err(|e| StoreError::Query(e.to_string()))?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
        }
        Ok(stats)
    }

    /// Aggregates error code distribution for a specific day range.
    pub fn get_ccproxy_error_distribution_stats(&self, days: i32) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT CAST(status_code AS TEXT) as code, COUNT(*) as count 
             FROM ccproxy_stats 
             WHERE request_at >= DATE('now', '-' || ?1 || ' days') AND status_code != 200
             GROUP BY code
             ORDER BY count DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([days], |row| {
            Ok(serde_json::json!({
                "type": row.get::<_, String>(0)?,
                "value": row.get::<_, u32>(1)?,
            }))
        }).map_err(|e| StoreError::Query(e.to_string()))?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
        }
        Ok(stats)
    }

    /// Retrieves detailed statistics per provider, model, protocol and mode for a specific date.
    pub fn get_ccproxy_provider_stats_by_date(&self, date: &str) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        
        let mut stmt = conn.prepare(
            "SELECT 
                COALESCE(provider, '-') as provider,
                COALESCE(client_model, '-') as client_model,
                COALESCE(backend_model, '-') as backend_model,
                COALESCE(protocol, '-') as protocol,
                tool_compat_mode,
                COUNT(*) as request_count,
                COALESCE(SUM(input_tokens), 0) as total_input_tokens,
                COALESCE(SUM(output_tokens), 0) as total_output_tokens,
                COALESCE(SUM(cache_tokens), 0) as total_cache_tokens,
                COUNT(*) FILTER (WHERE status_code != 200) as error_count
             FROM ccproxy_stats
             WHERE DATE(request_at) = ?1
             GROUP BY provider, client_model, backend_model, protocol, tool_compat_mode
             ORDER BY request_count DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([date], |row| {
            Ok(serde_json::json!({
                "provider": row.get::<_, String>(0)?,
                "clientModel": row.get::<_, String>(1)?,
                "backendModel": row.get::<_, String>(2)?,
                "protocol": row.get::<_, String>(3)?,
                "toolCompatMode": row.get::<_, i32>(4).unwrap_or(0),
                "requestCount": row.get::<_, u32>(5).unwrap_or(0),
                "totalInputTokens": row.get::<_, i64>(6).unwrap_or(0),
                "totalOutputTokens": row.get::<_, i64>(7).unwrap_or(0),
                "totalCacheTokens": row.get::<_, i64>(8).unwrap_or(0),
                "errorCount": row.get::<_, u32>(9).unwrap_or(0),
            }))
        }).map_err(|e| StoreError::Query(e.to_string()))?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
        }
        Ok(stats)
    }

    /// Retrieves error code distribution for a specific date.
    pub fn get_ccproxy_error_stats_by_date(&self, date: &str) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        
        let mut stmt = conn.prepare(
            "SELECT 
                status_code,
                error_message,
                COUNT(*) as error_count
             FROM ccproxy_stats
             WHERE DATE(request_at) = ?1 AND status_code != 200
             GROUP BY status_code, error_message
             ORDER BY error_count DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([date], |row| {
            Ok(serde_json::json!({
                "statusCode": row.get::<_, i32>(0)?,
                "errorMessage": row.get::<_, Option<String>>(1)?,
                "errorCount": row.get::<_, u32>(2)?,
            }))
        }).map_err(|e| StoreError::Query(e.to_string()))?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
        }
        Ok(stats)
    }
}
