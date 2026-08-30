use crate::db::{MainStore, StoreError};
use crate::workflow::react::usage::{ModelUsageBreakdown, UsageTotals, WorkflowUsageSummary};
use rusqlite::{params, OptionalExtension};

impl MainStore {
    pub fn summarize_workflow_task_usage(
        &self,
        session_id: &str,
        task_run_id: &str,
        root_session_id: &str,
        root_task_run_id: &str,
        terminal_status: &str,
        duration_ms: Option<i64>,
    ) -> Result<WorkflowUsageSummary, StoreError> {
        self.db_runtime()?.drain_blocking()?;
        let session_id = session_id.to_string();
        let task_run_id = task_run_id.to_string();
        let root_session_id = root_session_id.to_string();
        let root_task_run_id = root_task_run_id.to_string();
        let terminal_status = terminal_status.to_string();

        self.db_runtime()?.read_blocking(move |conn| {
            let read = |session_id: &str, task_run_id: &str| -> Result<Vec<ModelUsageBreakdown>, StoreError> {
                let mut statement = conn.prepare(
                    "SELECT provider_id, backend_model, SUM(input_tokens), SUM(output_tokens), SUM(cache_tokens), SUM(cache_write_tokens), SUM(reasoning_tokens), SUM(audio_input_tokens), SUM(audio_output_tokens), SUM(estimated_cost), CASE WHEN SUM(CASE WHEN pricing_status IS NULL OR pricing_status != 'priced' THEN 1 ELSE 0 END) > 0
                          THEN 'legacy' ELSE 'priced' END
                     FROM ccproxy_stats
                     WHERE workflow_session_id = ?1 AND workflow_task_run_id = ?2
                     GROUP BY provider_id, backend_model",
                )?;
                let rows = statement.query_map(params![session_id, task_run_id], |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<f64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                })?;
                rows.map(|row| {
                    let (
                        provider_id,
                        backend_model,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        audio_input_tokens,
                        audio_output_tokens,
                        persisted_cost,
                        persisted_status,
                    ) = row?;
                    let pricing_status = if persisted_status.as_deref() == Some("priced") {
                        "priced".to_string()
                    } else {
                        persisted_status.unwrap_or_else(|| "legacy".to_string())
                    };
                    Ok(ModelUsageBreakdown {
                        provider_id,
                        backend_model,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        audio_input_tokens,
                        audio_output_tokens,
                        pricing_status,
                        input_per_million: None,
                        output_per_million: None,
                        cache_per_million: None,
                        reasoning_per_million: None,
                        multiplier: None,
                        estimated_cost: persisted_cost,
                    })
                }).collect::<Result<Vec<_>, rusqlite::Error>>().map_err(StoreError::from)
            };

            let self_breakdowns = read(&session_id, &task_run_id)?;
            let self_has_attribution: bool = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM ccproxy_stats
                    WHERE workflow_session_id = ?1 AND workflow_task_run_id = ?2
                )",
                params![session_id, task_run_id],
                |row| row.get(0),
            )?;
            let is_root_task = session_id == root_session_id;
            let combined_breakdowns = if is_root_task {
                let mut combined_statement = conn.prepare(
                    "SELECT provider_id, backend_model, SUM(input_tokens), SUM(output_tokens), SUM(cache_tokens), SUM(cache_write_tokens), SUM(reasoning_tokens), SUM(audio_input_tokens), SUM(audio_output_tokens), SUM(estimated_cost), CASE WHEN SUM(CASE WHEN pricing_status IS NULL OR pricing_status != 'priced' THEN 1 ELSE 0 END) > 0
                          THEN 'legacy' ELSE 'priced' END
                     FROM ccproxy_stats
                     WHERE root_session_id = ?1 AND root_task_run_id = ?2
                     GROUP BY provider_id, backend_model",
                )?;
                let combined_rows = combined_statement.query_map(params![root_session_id, root_task_run_id], |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<f64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                })?;
                combined_rows.map(|row| {
                    let (
                        provider_id,
                        backend_model,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        audio_input_tokens,
                        audio_output_tokens,
                        persisted_cost,
                        persisted_status,
                    ) = row?;
                    let pricing_status = if persisted_status.as_deref() == Some("priced") {
                        "priced".to_string()
                    } else {
                        persisted_status.unwrap_or_else(|| "legacy".to_string())
                    };
                    Ok(ModelUsageBreakdown {
                        provider_id,
                        backend_model,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                        cache_write_tokens,
                        reasoning_tokens,
                        audio_input_tokens,
                        audio_output_tokens,
                        pricing_status,
                        input_per_million: None,
                        output_per_million: None,
                        cache_per_million: None,
                        reasoning_per_million: None,
                        multiplier: None,
                        estimated_cost: persisted_cost,
                    })
                }).collect::<Result<Vec<_>, rusqlite::Error>>().map_err(StoreError::from)?
            } else {
                self_breakdowns.clone()
            };
            let self_usage = UsageTotals::from_breakdowns(&self_breakdowns);
            let combined_breakdowns = if combined_breakdowns.is_empty() {
                self_breakdowns.clone()
            } else {
                combined_breakdowns
            };
            // A child belongs to this stage only after its own terminal summary has been
            // durably finalized with the same root task run. This retains zero-token children
            // without leaking children from an earlier hot-resumed root task. A child summary
            // can only make the combined total complete when its own summary is complete and
            // its task run has an attributed raw stat, including an explicit zero-token stat.
            let (has_sub_agents, child_usage_is_partial) = if is_root_task {
                let mut child_statement = conn.prepare(
                    "SELECT usage.summary_json,
                            EXISTS(
                                SELECT 1 FROM ccproxy_stats stats
                                WHERE stats.workflow_session_id = usage.session_id
                                  AND stats.workflow_task_run_id = usage.task_run_id
                            )
                     FROM workflow_task_usage usage
                     WHERE usage.root_session_id = ?1 AND usage.root_task_run_id = ?2
                       AND (usage.session_id != ?1 OR usage.task_run_id != ?2)",
                )?;
                let child_rows = child_statement.query_map(
                    params![root_session_id, root_task_run_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
                )?;
                let mut has_sub_agents = false;
                let mut child_usage_is_partial = false;
                for child_row in child_rows {
                    let (summary_json, has_attribution) = child_row?;
                    has_sub_agents = true;
                    let summary_is_partial = serde_json::from_str::<WorkflowUsageSummary>(
                        &summary_json,
                    )
                    .map(|summary| summary.is_partial)
                    .unwrap_or(true);
                    child_usage_is_partial |= summary_is_partial || !has_attribution;
                }
                (has_sub_agents, child_usage_is_partial)
            } else {
                (false, false)
            };
            let with_sub_agents = UsageTotals::from_breakdowns(&combined_breakdowns);
            Ok(WorkflowUsageSummary {
                version: 1,
                terminal_status,
                duration_ms,
                has_sub_agents,
                is_partial: !self_has_attribution
                    || child_usage_is_partial
                    || self_usage.unpriced_tokens > 0
                    || with_sub_agents.unpriced_tokens > 0,
                self_usage,
                with_sub_agents,
                model_breakdowns: combined_breakdowns,
            })
        })
    }

    pub fn upsert_workflow_task_usage(
        &self,
        session_id: &str,
        task_run_id: &str,
        root_session_id: &str,
        root_task_run_id: &str,
        terminal_status: &str,
        started_at: Option<&str>,
        ended_at: Option<&str>,
        duration_ms: Option<i64>,
        summary: &WorkflowUsageSummary,
    ) -> Result<(), StoreError> {
        let summary_json = serde_json::to_string(summary).map_err(|error| {
            StoreError::Query(format!("serialize workflow usage summary: {error}"))
        })?;
        let runtime = self.db_runtime()?;
        let session_id = session_id.to_string();
        let task_run_id = task_run_id.to_string();
        let root_session_id = root_session_id.to_string();
        let root_task_run_id = root_task_run_id.to_string();
        let terminal_status = terminal_status.to_string();
        let started_at = started_at.map(str::to_string);
        let ended_at = ended_at.map(str::to_string);
        runtime.write_blocking(move |conn| {
            conn.execute(
                "INSERT INTO workflow_task_usage (
                    session_id, task_run_id, root_session_id, root_task_run_id, terminal_status,
                    started_at, ended_at, duration_ms, summary_json, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, CURRENT_TIMESTAMP)
                 ON CONFLICT(session_id, task_run_id) DO UPDATE SET
                    root_session_id = excluded.root_session_id,
                    root_task_run_id = excluded.root_task_run_id,
                    terminal_status = excluded.terminal_status,
                    started_at = excluded.started_at,
                    ended_at = excluded.ended_at,
                    duration_ms = excluded.duration_ms,
                    summary_json = excluded.summary_json,
                    updated_at = CURRENT_TIMESTAMP",
                params![
                    session_id,
                    task_run_id,
                    root_session_id,
                    root_task_run_id,
                    terminal_status,
                    started_at,
                    ended_at,
                    duration_ms,
                    summary_json,
                ],
            )?;
            Ok(())
        })
    }

    pub fn load_workflow_task_usage(
        &self,
        session_id: &str,
        task_run_id: &str,
    ) -> Result<Option<WorkflowUsageSummary>, StoreError> {
        let runtime = self.db_runtime()?;
        let session_id = session_id.to_string();
        let task_run_id = task_run_id.to_string();
        runtime.read_blocking(move |conn| {
            let summary_json: Option<String> = conn
                .query_row(
                    "SELECT summary_json FROM workflow_task_usage
                     WHERE session_id = ?1 AND task_run_id = ?2",
                    params![session_id, task_run_id],
                    |row| row.get(0),
                )
                .optional()?;
            summary_json
                .map(|summary_json| {
                    serde_json::from_str(&summary_json).map_err(|error| {
                        StoreError::Query(format!("deserialize workflow usage summary: {error}"))
                    })
                })
                .transpose()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn summary() -> WorkflowUsageSummary {
        WorkflowUsageSummary {
            version: 1,
            terminal_status: "completed".to_string(),
            duration_ms: Some(0),
            self_usage: UsageTotals::default(),
            with_sub_agents: UsageTotals::default(),
            has_sub_agents: false,
            is_partial: false,
            model_breakdowns: Vec::new(),
        }
    }

    fn insert_attributed_stat(
        store: &MainStore,
        session_id: &str,
        task_run_id: &str,
        root_session_id: &str,
        root_task_run_id: &str,
        input_tokens: i64,
        output_tokens: i64,
        cache_tokens: i64,
    ) {
        let runtime = store.db_runtime().expect("test runtime should exist");
        let session_id = session_id.to_string();
        let task_run_id = task_run_id.to_string();
        let root_session_id = root_session_id.to_string();
        let root_task_run_id = root_task_run_id.to_string();
        runtime
            .write_blocking(move |conn| {
                conn.execute(
                    "INSERT INTO ccproxy_stats (
                        workflow_session_id, workflow_task_run_id, workflow_segment_id,
                        root_session_id, root_task_run_id, request_kind,
                        client_model, backend_model, provider_id, provider, protocol,
                        tool_compat_mode, status_code, input_tokens, output_tokens, cache_tokens
                     ) VALUES (?1, ?2, 1, ?3, ?4, 'react',
                               'alias', 'unpriced-model', NULL, 'provider', 'openai',
                               0, 200, ?5, ?6, ?7)",
                    params![
                        session_id,
                        task_run_id,
                        root_session_id,
                        root_task_run_id,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                    ],
                )?;
                Ok(())
            })
            .expect("failed to insert attributed stat");
    }

    #[test]
    fn missing_attribution_is_partial_for_every_supported_terminal_status() {
        let directory = tempdir().expect("failed to create temp dir");
        let store =
            MainStore::new(directory.path().join("missing.db")).expect("failed to create store");

        for status in ["completed", "failed", "cancelled", "interrupted"] {
            let task_run_id = format!("session-{status}:task:1");
            let usage = store
                .summarize_workflow_task_usage(
                    &format!("session-{status}"),
                    &task_run_id,
                    "root-session",
                    "root-session:task:1",
                    status,
                    Some(0),
                )
                .expect("failed to summarize unattributed terminal task");
            assert!(
                usage.is_partial,
                "{status} must not look like exact zero cost"
            );
            assert_eq!(usage.terminal_status, status);
            assert_eq!(usage.self_usage.total_tokens, 0);
        }
    }

    #[test]
    fn attributed_zero_tokens_are_complete_but_unpriced_positive_tokens_are_partial() {
        let directory = tempdir().expect("failed to create temp dir");
        let store =
            MainStore::new(directory.path().join("zero.db")).expect("failed to create store");
        insert_attributed_stat(
            &store,
            "zero-session",
            "zero-session:task:1",
            "zero-session",
            "zero-session:task:1",
            0,
            0,
            0,
        );

        let zero = store
            .summarize_workflow_task_usage(
                "zero-session",
                "zero-session:task:1",
                "zero-session",
                "zero-session:task:1",
                "completed",
                Some(0),
            )
            .expect("failed to summarize attributed zero-token task");
        assert!(
            !zero.is_partial,
            "explicit zero-token attribution is complete"
        );
        assert_eq!(zero.self_usage.estimated_cost, Some(0.0));

        insert_attributed_stat(
            &store,
            "unpriced-session",
            "unpriced-session:task:1",
            "unpriced-session",
            "unpriced-session:task:1",
            10,
            5,
            2,
        );
        let unpriced = store
            .summarize_workflow_task_usage(
                "unpriced-session",
                "unpriced-session:task:1",
                "unpriced-session",
                "unpriced-session:task:1",
                "failed",
                Some(0),
            )
            .expect("failed to summarize unpriced task");
        assert!(unpriced.is_partial);
        assert_eq!(unpriced.self_usage.total_tokens, 15);
        assert_eq!(unpriced.self_usage.estimated_cost, None);
        assert_eq!(unpriced.self_usage.unpriced_tokens, 15);
    }

    #[test]
    fn root_summary_is_partial_when_a_terminal_child_has_no_attributed_stats() {
        let directory = tempdir().expect("failed to create temp dir");
        let store = MainStore::new(directory.path().join("partial-child.db"))
            .expect("failed to create test store");
        insert_attributed_stat(
            &store,
            "root-session",
            "root-session:task:1",
            "root-session",
            "root-session:task:1",
            0,
            0,
            0,
        );
        let mut partial_child_summary = summary();
        partial_child_summary.is_partial = true;
        store
            .upsert_workflow_task_usage(
                "partial-child",
                "partial-child:task:1",
                "root-session",
                "root-session:task:1",
                "failed",
                None,
                None,
                Some(0),
                &partial_child_summary,
            )
            .expect("failed to persist partial child summary");

        let root = store
            .summarize_workflow_task_usage(
                "root-session",
                "root-session:task:1",
                "root-session",
                "root-session:task:1",
                "completed",
                Some(0),
            )
            .expect("failed to summarize root with partial child");

        assert!(root.has_sub_agents);
        assert_eq!(root.self_usage.total_tokens, 0);
        assert_eq!(
            root.with_sub_agents.total_tokens,
            root.self_usage.total_tokens
        );
        assert!(
            root.is_partial,
            "combined usage must not present an unattributed child as an exact total"
        );
    }

    #[test]
    fn child_presence_is_scoped_to_the_current_root_task_run() {
        let directory = tempdir().expect("failed to create temp dir");
        let store =
            MainStore::new(directory.path().join("usage.db")).expect("failed to create test store");
        store
            .upsert_workflow_task_usage(
                "child-session",
                "child-session:task:1",
                "root-session",
                "root-session:task:1",
                "completed",
                None,
                None,
                Some(0),
                &summary(),
            )
            .expect("failed to persist prior child summary");

        insert_attributed_stat(
            &store,
            "root-session",
            "root-session:task:2",
            "root-session",
            "root-session:task:2",
            0,
            0,
            0,
        );
        let current = store
            .summarize_workflow_task_usage(
                "root-session",
                "root-session:task:2",
                "root-session",
                "root-session:task:2",
                "completed",
                Some(0),
            )
            .expect("failed to summarize current stage");
        assert!(!current.has_sub_agents);
        assert!(!current.is_partial, "valid zero-token tasks are complete");

        let recovered = store
            .summarize_workflow_task_usage(
                "recovered-session",
                "recovered-session:task:1",
                "root-session",
                "root-session:task:2",
                "interrupted",
                Some(0),
            )
            .expect("failed to summarize unattributed recovered task");
        assert!(
            recovered.is_partial,
            "unattributed recovery must not look free"
        );
        assert_eq!(recovered.self_usage.total_tokens, 0);
        assert_eq!(recovered.self_usage.estimated_cost, Some(0.0));

        insert_attributed_stat(
            &store,
            "zero-token-child",
            "zero-token-child:task:1",
            "root-session",
            "root-session:task:2",
            0,
            0,
            0,
        );
        store
            .upsert_workflow_task_usage(
                "zero-token-child",
                "zero-token-child:task:1",
                "root-session",
                "root-session:task:2",
                "completed",
                None,
                None,
                Some(0),
                &summary(),
            )
            .expect("failed to persist zero-token child summary");

        let current = store
            .summarize_workflow_task_usage(
                "root-session",
                "root-session:task:2",
                "root-session",
                "root-session:task:2",
                "completed",
                Some(0),
            )
            .expect("failed to summarize current stage with child");
        assert!(current.has_sub_agents);
        assert_eq!(current.with_sub_agents.total_tokens, 0);
        assert!(
            !current.is_partial,
            "explicitly attributed zero-token children keep the combined summary complete"
        );
    }
}
