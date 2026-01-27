use rusqlite::params;
use crate::db::{error::StoreError, types::CcproxyStat, MainStore};

impl MainStore {
    /// Records a new proxy statistic entry in the database.
    pub fn record_ccproxy_stat(&self, stat: CcproxyStat) -> Result<i64, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        
        match conn.execute(
            "INSERT INTO ccproxy_stats (model, provider, protocol, tool_compat_mode, status_code, error_message, input_tokens, output_tokens, cache_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                stat.model,
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
                SUM(input_tokens) as total_input_tokens,
                SUM(output_tokens) as total_output_tokens,
                SUM(cache_tokens) as total_cache_tokens,
                COUNT(DISTINCT provider) as provider_count,
                COUNT(*) FILTER (WHERE status_code != 200) as error_count,
                (SELECT provider FROM ccproxy_stats s2 WHERE DATE(s2.request_at) = DATE(s1.request_at) GROUP BY provider ORDER BY COUNT(*) DESC LIMIT 1) as top_provider
             FROM ccproxy_stats s1
             WHERE request_at >= DATE('now', '-' || ?1 || ' days')
             GROUP BY date
             ORDER BY date DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([days], |row| {
            Ok(serde_json::json!({
                "date": row.get::<_, String>(0)?,
                "totalInputTokens": row.get::<_, i64>(1)?,
                "totalOutputTokens": row.get::<_, i64>(2)?,
                "totalCacheTokens": row.get::<_, i64>(3)?,
                "providerCount": row.get::<_, u32>(4)?,
                "errorCount": row.get::<_, u32>(5)?,
                "topProvider": row.get::<_, String>(6)?,
            }))
        }).map_err(|e| StoreError::Query(e.to_string()))?;

        let mut stats = Vec::new();
        for row in rows {
            stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
        }
        Ok(stats)
    }

    /// Retrieves detailed statistics per provider, protocol and mode for a specific date.
    pub fn get_ccproxy_provider_stats_by_date(&self, date: &str) -> Result<Vec<serde_json::Value>, StoreError> {
        let conn = self.conn.lock().map_err(|e| StoreError::LockError(e.to_string()))?;
        
        let mut stmt = conn.prepare(
            "SELECT 
                provider,
                protocol,
                tool_compat_mode,
                COUNT(*) as request_count,
                SUM(input_tokens) as total_input_tokens,
                SUM(output_tokens) as total_output_tokens,
                SUM(cache_tokens) as total_cache_tokens,
                COUNT(*) FILTER (WHERE status_code != 200) as error_count
             FROM ccproxy_stats
             WHERE DATE(request_at) = ?1
             GROUP BY provider, protocol, tool_compat_mode
             ORDER BY request_count DESC"
        ).map_err(|e| StoreError::Query(e.to_string()))?;

        let rows = stmt.query_map([date], |row| {
            Ok(serde_json::json!({
                "provider": row.get::<_, String>(0)?,
                "protocol": row.get::<_, String>(1)?,
                "toolCompatMode": row.get::<_, i32>(2)?,
                "requestCount": row.get::<_, u32>(3)?,
                "totalInputTokens": row.get::<_, i64>(4)?,
                "totalOutputTokens": row.get::<_, i64>(5)?,
                "totalCacheTokens": row.get::<_, i64>(6)?,
                "errorCount": row.get::<_, u32>(7)?,
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
