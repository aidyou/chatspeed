use crate::db::{error::StoreError, types::CcproxyStat, MainStore};
use chrono::{Duration, Local, NaiveDate, TimeZone, Utc};
use rusqlite::params;

fn stats_range_for_days(days: i32) -> Result<Option<(String, String)>, StoreError> {
    if days == -1 {
        return Ok(None);
    }
    if days < -1 {
        return Err(StoreError::Query(format!(
            "Invalid proxy statistics day range: {days}"
        )));
    }

    let today = Local::now().date_naive();
    stats_range_for_local_dates(
        today - Duration::days(i64::from(days)),
        today + Duration::days(1),
    )
    .map(Some)
}

fn stats_range_for_local_date(date: &str) -> Result<(String, String), StoreError> {
    let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|error| {
        StoreError::Query(format!("Invalid proxy statistics date '{date}': {error}"))
    })?;
    stats_range_for_local_dates(date, date + Duration::days(1))
}

fn stats_range_for_local_dates(
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<(String, String), StoreError> {
    let to_utc_boundary = |date: NaiveDate| {
        Local
            .from_local_datetime(&date.and_hms_opt(0, 0, 0).ok_or_else(|| {
                StoreError::Query(format!("Invalid proxy statistics date boundary: {date}"))
            })?)
            .earliest()
            .map(|boundary| {
                boundary
                    .with_timezone(&Utc)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .ok_or_else(|| {
                StoreError::Query(format!(
                    "Invalid local proxy statistics date boundary: {date}"
                ))
            })
    };

    Ok((to_utc_boundary(start_date)?, to_utc_boundary(end_date)?))
}

fn stats_range_for_date_range(
    start_date: &str,
    end_date: &str,
) -> Result<(String, String), StoreError> {
    let start_date = NaiveDate::parse_from_str(start_date, "%Y-%m-%d").map_err(|error| {
        StoreError::Query(format!(
            "Invalid proxy statistics start date '{start_date}': {error}"
        ))
    })?;
    let end_date = NaiveDate::parse_from_str(end_date, "%Y-%m-%d").map_err(|error| {
        StoreError::Query(format!(
            "Invalid proxy statistics end date '{end_date}': {error}"
        ))
    })?;
    if end_date < start_date {
        return Err(StoreError::Query(
            "Proxy statistics end date precedes start date".to_string(),
        ));
    }
    stats_range_for_local_dates(start_date, end_date + Duration::days(1))
}

fn range_where_clause(range: Option<&(String, String)>) -> &'static str {
    if range.is_some() {
        " WHERE request_at >= ?1 AND request_at < ?2"
    } else {
        ""
    }
}

fn range_params(range: Option<(String, String)>) -> Vec<String> {
    range
        .into_iter()
        .flat_map(|(start_at, end_at)| [start_at, end_at])
        .collect()
}

fn query_grouped_stats(
    conn: &rusqlite::Connection,
    range: Option<(String, String)>,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let sql = format!(
        "SELECT
            DATE(request_at, 'localtime') AS date,
            client_model,
            provider_id,
            COALESCE(provider, '-') AS provider,
            COALESCE(backend_model, '-') AS backend_model,
            COALESCE(protocol, '-') AS protocol,
            tool_compat_mode,
            COUNT(*) AS request_count,
            COALESCE(SUM(input_tokens), 0) AS total_input_tokens,
            COALESCE(SUM(output_tokens), 0) AS total_output_tokens,
            COALESCE(SUM(cache_tokens), 0) AS total_cache_tokens,
            COALESCE(SUM(cache_write_tokens), 0) AS total_cache_write_tokens,
            COALESCE(SUM(reasoning_tokens), 0) AS total_reasoning_tokens,
            COALESCE(SUM(audio_input_tokens), 0) AS total_audio_input_tokens,
            COALESCE(SUM(audio_output_tokens), 0) AS total_audio_output_tokens,
            COALESCE(SUM(estimated_cost), 0) AS total_estimated_cost
         FROM ccproxy_stats{}
         GROUP BY date, client_model, provider_id, provider, backend_model, protocol, tool_compat_mode
         ORDER BY date DESC",
        range_where_clause(range.as_ref())
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| StoreError::Query(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(range_params(range)), |row| {
            Ok(serde_json::json!({
                "date": row.get::<_, String>(0)?,
                "clientModel": row.get::<_, String>(1)?,
                "providerId": row.get::<_, Option<i64>>(2)?,
                "provider": row.get::<_, String>(3)?,
                "backendModel": row.get::<_, String>(4)?,
                "protocol": row.get::<_, String>(5)?,
                "toolCompatMode": row.get::<_, i32>(6).unwrap_or(0),
                "requestCount": row.get::<_, u32>(7).unwrap_or(0),
                "totalInputTokens": row.get::<_, i64>(8).unwrap_or(0),
                "totalOutputTokens": row.get::<_, i64>(9).unwrap_or(0),
                "totalCacheTokens": row.get::<_, i64>(10).unwrap_or(0),
                "totalCacheWriteTokens": row.get::<_, i64>(11).unwrap_or(0),
                "totalReasoningTokens": row.get::<_, i64>(12).unwrap_or(0),
                "totalAudioInputTokens": row.get::<_, i64>(13).unwrap_or(0),
                "totalAudioOutputTokens": row.get::<_, i64>(14).unwrap_or(0),
                "totalEstimatedCost": row.get::<_, f64>(15).unwrap_or(0.0),
            }))
        })
        .map_err(|e| StoreError::Query(e.to_string()))?;

    let mut stats = Vec::new();
    for row in rows {
        stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
    }
    Ok(stats)
}

fn query_daily_stats(
    conn: &rusqlite::Connection,
    range: Option<(String, String)>,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let sql = format!(
        "WITH filtered_stats AS (
            SELECT * FROM ccproxy_stats{}
         ), daily_stats AS (
            SELECT
                DATE(request_at, 'localtime') AS date,
                COALESCE(SUM(input_tokens), 0) AS total_input_tokens,
                COALESCE(SUM(output_tokens), 0) AS total_output_tokens,
                COALESCE(SUM(cache_tokens), 0) AS total_cache_tokens,
                COALESCE(SUM(cache_write_tokens), 0) AS total_cache_write_tokens,
                COALESCE(SUM(reasoning_tokens), 0) AS total_reasoning_tokens,
                COALESCE(SUM(audio_input_tokens), 0) AS total_audio_input_tokens,
                COALESCE(SUM(audio_output_tokens), 0) AS total_audio_output_tokens,
                COALESCE(SUM(estimated_cost), 0) AS total_estimated_cost,
                COUNT(DISTINCT provider) AS provider_count,
                COUNT(*) FILTER (WHERE status_code != 200) AS error_count,
                COUNT(*) AS total_request_count
            FROM filtered_stats
            GROUP BY date
         ), ranked_models AS (
            SELECT
                DATE(request_at, 'localtime') AS date,
                client_model,
                ROW_NUMBER() OVER (
                    PARTITION BY DATE(request_at, 'localtime')
                    ORDER BY COUNT(*) DESC, client_model ASC
                ) AS rank
            FROM filtered_stats
            GROUP BY date, client_model
         )
         SELECT
            daily_stats.date,
            daily_stats.total_input_tokens,
            daily_stats.total_output_tokens,
            daily_stats.total_cache_tokens,
            daily_stats.total_cache_write_tokens,
            daily_stats.total_reasoning_tokens,
            daily_stats.total_audio_input_tokens,
            daily_stats.total_audio_output_tokens,
            daily_stats.total_estimated_cost,
            daily_stats.provider_count,
            daily_stats.error_count,
            COALESCE(ranked_models.client_model, '-'),
            daily_stats.total_request_count
         FROM daily_stats
         LEFT JOIN ranked_models
            ON ranked_models.date = daily_stats.date AND ranked_models.rank = 1
         ORDER BY daily_stats.date DESC",
        range_where_clause(range.as_ref())
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| StoreError::Query(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(range_params(range)), |row| {
            Ok(serde_json::json!({
                "date": row.get::<_, String>(0)?,
                "totalInputTokens": row.get::<_, i64>(1).unwrap_or(0),
                "totalOutputTokens": row.get::<_, i64>(2).unwrap_or(0),
                "totalCacheTokens": row.get::<_, i64>(3).unwrap_or(0),
                "totalCacheWriteTokens": row.get::<_, i64>(4).unwrap_or(0),
                "totalReasoningTokens": row.get::<_, i64>(5).unwrap_or(0),
                "totalAudioInputTokens": row.get::<_, i64>(6).unwrap_or(0),
                "totalAudioOutputTokens": row.get::<_, i64>(7).unwrap_or(0),
                "estimatedCost": row.get::<_, f64>(8).unwrap_or(0.0),
                "providerCount": row.get::<_, u32>(9).unwrap_or(0),
                "errorCount": row.get::<_, u32>(10).unwrap_or(0),
                "topProvider": row.get::<_, String>(11).unwrap_or_else(|_| "-".to_string()),
                "totalRequestCount": row.get::<_, u32>(12).unwrap_or(0),
            }))
        })
        .map_err(|e| StoreError::Query(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| StoreError::Query(e.to_string()))
}

fn query_provider_stats_by_date(
    conn: &rusqlite::Connection,
    date: &str,
) -> Result<Vec<serde_json::Value>, StoreError> {
    let (start_at, end_at) = stats_range_for_local_date(date)?;
    let mut stmt = conn
        .prepare(
            "SELECT
            COALESCE(provider, '-') AS provider,
            provider_id,
            COALESCE(client_model, '-') AS client_model,
            COALESCE(backend_model, '-') AS backend_model,
            COALESCE(protocol, '-') AS protocol,
            tool_compat_mode,
            COUNT(*) AS request_count,
            COALESCE(SUM(input_tokens), 0) AS total_input_tokens,
            COALESCE(SUM(output_tokens), 0) AS total_output_tokens,
            COALESCE(SUM(cache_tokens), 0) AS total_cache_tokens,
            COUNT(*) FILTER (WHERE status_code != 200) AS error_count
         FROM ccproxy_stats
         WHERE request_at >= ?1 AND request_at < ?2
         GROUP BY provider_id, provider, client_model, backend_model, protocol, tool_compat_mode
         ORDER BY request_count DESC",
        )
        .map_err(|e| StoreError::Query(e.to_string()))?;
    let rows = stmt
        .query_map(params![start_at, end_at], |row| {
            Ok(serde_json::json!({
                "provider": row.get::<_, String>(0)?,
                "providerId": row.get::<_, Option<i64>>(1)?,
                "clientModel": row.get::<_, String>(2)?,
                "backendModel": row.get::<_, String>(3)?,
                "protocol": row.get::<_, String>(4)?,
                "toolCompatMode": row.get::<_, i32>(5).unwrap_or(0),
                "requestCount": row.get::<_, u32>(6).unwrap_or(0),
                "totalInputTokens": row.get::<_, i64>(7).unwrap_or(0),
                "totalOutputTokens": row.get::<_, i64>(8).unwrap_or(0),
                "totalCacheTokens": row.get::<_, i64>(9).unwrap_or(0),
                "errorCount": row.get::<_, u32>(10).unwrap_or(0),
            }))
        })
        .map_err(|e| StoreError::Query(e.to_string()))?;
    let mut stats = Vec::new();
    for row in rows {
        stats.push(row.map_err(|e| StoreError::Query(e.to_string()))?);
    }
    Ok(stats)
}

impl MainStore {
    /// Records a new proxy statistic entry in the database.
    ///
    /// # Field Mapping
    /// - `client_model`: Should be the user-configured proxy alias (e.g., "code-small").
    ///   For internal direct requests using X-CS-Provider-Id/X-CS-Model-Id headers,
    ///   this will be the model_id since no alias lookup is performed.
    /// - `backend_model`: The actual model ID sent to the provider's API
    ///   (e.g., "Qwen/Qwen3-Next-80B-A3B-Instruct").
    ///
    /// # Consistency Note
    /// All handlers should use `proxy_model.client_alias` for `client_model` and
    /// `proxy_model.model` for `backend_model` to ensure consistent statistics.
    /// See `CcproxyStat` struct documentation for details.
    pub fn record_ccproxy_stat(&self, stat: CcproxyStat) -> Result<(), StoreError> {
        let result = self.db_runtime()?.enqueue_ccproxy_stat(stat);
        if let Err(error) = &result {
            log::error!("Failed to enqueue CCProxy statistic: {error}");
        }
        result
    }

    pub(crate) async fn get_ccproxy_daily_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .read(move |conn| query_daily_stats(conn, range))
            .await
    }

    pub(crate) async fn delete_ccproxy_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<(), StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .write(move |conn| {
                match range {
                    None => conn.execute("DELETE FROM ccproxy_stats", params![]),
                    Some((start_at, _)) => conn.execute(
                        "DELETE FROM ccproxy_stats WHERE request_at < ?1",
                        params![start_at],
                    ),
                }
                .map_err(|e| StoreError::Query(e.to_string()))?;
                Ok(())
            })
            .await
    }

    pub(crate) async fn get_ccproxy_model_usage_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .read(move |conn| {
            let sql = format!(
                "SELECT backend_model, COUNT(*) AS count FROM ccproxy_stats{} GROUP BY backend_model ORDER BY count DESC",
                range_where_clause(range.as_ref())
            );
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(range_params(range)), |row| {
                Ok(serde_json::json!({
                    "type": row.get::<_, String>(0)?,
                    "value": row.get::<_, u32>(1)?,
                }))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub(crate) async fn get_ccproxy_model_token_usage_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .read(move |conn| {
            let sql = format!(
                "SELECT backend_model, SUM(input_tokens + output_tokens) AS total_tokens FROM ccproxy_stats{} GROUP BY backend_model ORDER BY total_tokens DESC",
                range_where_clause(range.as_ref())
            );
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(range_params(range)), |row| {
                Ok(serde_json::json!({
                    "type": row.get::<_, String>(0)?,
                    "value": row.get::<_, i64>(1).unwrap_or(0),
                }))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub(crate) async fn get_ccproxy_provider_token_usage_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .read(move |conn| {
            let sql = format!(
                "SELECT COALESCE(provider, '-') AS provider, SUM(input_tokens + output_tokens) AS total_tokens FROM ccproxy_stats{} GROUP BY provider ORDER BY total_tokens DESC",
                range_where_clause(range.as_ref())
            );
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(range_params(range)), |row| {
                Ok(serde_json::json!({
                    "type": row.get::<_, String>(0)?,
                    "value": row.get::<_, i64>(1).unwrap_or(0),
                }))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub(crate) async fn get_ccproxy_error_distribution_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .read(move |conn| {
            let sql = format!(
                "SELECT CAST(status_code AS TEXT) AS code, COUNT(*) AS count FROM ccproxy_stats{}{} GROUP BY code ORDER BY count DESC",
                range_where_clause(range.as_ref()),
                if range.is_some() { " AND status_code != 200" } else { " WHERE status_code != 200" }
            );
            let mut statement = conn.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(range_params(range)), |row| {
                Ok(serde_json::json!({
                    "type": row.get::<_, String>(0)?,
                    "value": row.get::<_, u32>(1)?,
                }))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }

    pub(crate) async fn get_ccproxy_provider_stats_by_date_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        date: String,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        runtime
            .read(move |connection| query_provider_stats_by_date(connection, &date))
            .await
    }

    pub(crate) async fn get_ccproxy_grouped_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        days: i32,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_days(days)?;
        runtime
            .read(move |connection| query_grouped_stats(connection, range))
            .await
    }

    pub(crate) async fn get_ccproxy_grouped_stats_by_date_range_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let range = stats_range_for_date_range(start_date, end_date)?;
        runtime
            .read(move |connection| query_grouped_stats(connection, Some(range)))
            .await
    }

    /// Executes the narrow today-cost aggregation on a dedicated reader worker.
    pub(crate) async fn get_ccproxy_today_cost_stats_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        let today = Local::now().date_naive();
        let range = stats_range_for_local_dates(today, today + Duration::days(1))?;
        runtime
            .read(move |connection| query_grouped_stats(connection, Some(range)))
            .await
    }

    pub(crate) async fn get_ccproxy_error_stats_by_date_with_runtime(
        runtime: std::sync::Arc<crate::db::runtime::DbRuntime>,
        date: String,
        client_model: Option<String>,
        backend_model: Option<String>,
    ) -> Result<Vec<serde_json::Value>, StoreError> {
        runtime
            .read(move |conn| {
                let (start_at, end_at) = stats_range_for_local_date(&date)?;
                let mut sql = "SELECT status_code, error_message, COUNT(*) AS error_count
                 FROM ccproxy_stats
                 WHERE request_at >= ?1 AND request_at < ?2 AND status_code != 200"
                    .to_string();
                let mut values = vec![start_at, end_at];
                let mut parameter_index = 3;
                if let Some(client_model) = client_model {
                    sql.push_str(&format!(" AND client_model = ?{parameter_index}"));
                    values.push(client_model);
                    parameter_index += 1;
                }
                if let Some(backend_model) = backend_model {
                    sql.push_str(&format!(" AND backend_model = ?{parameter_index}"));
                    values.push(backend_model);
                }
                sql.push_str(" GROUP BY status_code, error_message ORDER BY error_count DESC");
                let mut statement = conn.prepare(&sql)?;
                let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
                    Ok(serde_json::json!({
                        "statusCode": row.get::<_, i32>(0)?,
                        "errorMessage": row.get::<_, Option<String>>(1)?,
                        "errorCount": row.get::<_, u32>(2)?,
                    }))
                })?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::{
        stats_range_for_date_range, stats_range_for_days, stats_range_for_local_date,
        stats_range_for_local_dates,
    };
    use chrono::{Duration, Local, TimeZone, Utc};
    use rusqlite::Connection;
    use std::time::Instant;

    #[test]
    fn rejects_invalid_statistics_ranges() {
        assert!(stats_range_for_days(-2).is_err());
        assert!(stats_range_for_local_date("not-a-date").is_err());
        assert!(stats_range_for_date_range("2026-07-27", "2026-07-21").is_err());
    }

    #[test]
    fn accepts_chronological_statistics_date_range() {
        let (start_at, end_at) = stats_range_for_date_range("2026-07-21", "2026-07-27").unwrap();
        assert!(start_at < end_at);
    }

    #[test]
    fn local_date_range_is_a_non_empty_utc_half_open_interval() {
        let (start_at, end_at) = stats_range_for_local_date("2026-03-08").unwrap();
        assert!(start_at < end_at);
        assert_eq!(start_at.len(), "2026-03-08 00:00:00".len());
        assert_eq!(end_at.len(), "2026-03-09 00:00:00".len());
    }

    #[test]
    fn range_predicate_uses_the_request_timestamp_index() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ccproxy_stats (request_at TEXT NOT NULL);
                 CREATE INDEX idx_ccproxy_stats_request_at
                    ON ccproxy_stats(request_at DESC);",
            )
            .unwrap();
        let plan = connection
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT COUNT(*) FROM ccproxy_stats
                 WHERE request_at >= ?1 AND request_at < ?2",
                ["2026-07-19 00:00:00", "2026-07-27 00:00:00"],
                |row| row.get::<_, String>(3),
            )
            .unwrap();
        assert!(plan.contains("idx_ccproxy_stats_request_at"));
    }

    #[test]
    #[ignore = "records reproducible 100k/500k/1m query-plan and latency evidence"]
    fn statistics_range_benchmark_reports_indexed_query_evidence() {
        const SAMPLE_SIZES: [usize; 3] = [100_000, 500_000, 1_000_000];
        let end_date = Local::now().date_naive();
        let start_date = end_date - Duration::days(6);
        let (start_at, end_at) =
            stats_range_for_local_dates(start_date, end_date + Duration::days(1))
                .expect("benchmark date range should be valid");

        for sample_size in SAMPLE_SIZES {
            let connection = Connection::open_in_memory().expect("benchmark database should open");
            connection
                .execute_batch(
                    "CREATE TABLE ccproxy_stats (request_at TEXT NOT NULL);
                     CREATE INDEX idx_ccproxy_stats_request_at
                        ON ccproxy_stats(request_at DESC);",
                )
                .expect("benchmark schema should initialize");

            let seed_start = Instant::now();
            let transaction = connection
                .unchecked_transaction()
                .expect("benchmark seed transaction should open");
            {
                let mut statement = transaction
                    .prepare("INSERT INTO ccproxy_stats (request_at) VALUES (?1)")
                    .expect("benchmark insert statement should prepare");
                for index in 0..sample_size {
                    let local_date = end_date - Duration::days((index % 365) as i64);
                    let local_noon = Local
                        .from_local_datetime(
                            &local_date
                                .and_hms_opt(12, 0, 0)
                                .expect("noon should be a valid local time"),
                        )
                        .earliest()
                        .expect("local noon should resolve across DST");
                    statement
                        .execute([local_noon
                            .with_timezone(&Utc)
                            .format("%Y-%m-%d %H:%M:%S")
                            .to_string()])
                        .expect("benchmark row should insert");
                }
            }
            transaction.commit().expect("benchmark seed should commit");

            let plan = connection
                .query_row(
                    "EXPLAIN QUERY PLAN
                     SELECT COUNT(*) FROM ccproxy_stats
                     WHERE request_at >= ?1 AND request_at < ?2",
                    [&start_at, &end_at],
                    |row| row.get::<_, String>(3),
                )
                .expect("benchmark query plan should be available");
            assert!(plan.contains("idx_ccproxy_stats_request_at"));

            let old_start = Instant::now();
            let old_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM ccproxy_stats
                     WHERE DATE(request_at, 'localtime') >= ?1
                       AND DATE(request_at, 'localtime') <= ?2",
                    [start_date.to_string(), end_date.to_string()],
                    |row| row.get(0),
                )
                .expect("legacy benchmark query should succeed");
            let old_elapsed = old_start.elapsed();

            let indexed_start = Instant::now();
            let indexed_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM ccproxy_stats WHERE request_at >= ?1 AND request_at < ?2",
                    [&start_at, &end_at],
                    |row| row.get(0),
                )
                .expect("indexed benchmark query should succeed");
            let indexed_elapsed = indexed_start.elapsed();
            assert_eq!(old_count, indexed_count);

            println!(
                "ccproxy range benchmark: rows={sample_size}, seed_ms={}, old_date_ms={}, indexed_range_ms={}, matched_rows={indexed_count}, plan={plan}",
                seed_start.elapsed().as_millis(),
                old_elapsed.as_millis(),
                indexed_elapsed.as_millis(),
            );
        }
    }
}
