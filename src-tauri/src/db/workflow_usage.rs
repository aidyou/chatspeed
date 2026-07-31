use crate::db::{MainStore, StoreError};
use crate::workflow::react::usage::{
    calculate_model_cost, ModelUsageBreakdown, PricingSnapshot, UsageTotals, WorkflowUsageSummary,
};
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
        require_attribution: bool,
    ) -> Result<WorkflowUsageSummary, StoreError> {
        self.db_runtime()?.drain_blocking()?;
        let pricing = self
            .config
            .get_ai_models()?
            .into_iter()
            .flat_map(|model| {
                let provider_id = model.id;
                model.models.into_iter().filter_map(move |config| {
                    config.pricing.map(|pricing| {
                        (
                            (provider_id, config.id),
                            PricingSnapshot {
                                input_per_million: pricing.input_per_million,
                                output_per_million: pricing.output_per_million,
                                cache_per_million: pricing.cache_per_million,
                                multiplier: pricing.multiplier,
                            },
                        )
                    })
                })
            })
            .collect::<std::collections::HashMap<_, _>>();
        let session_id = session_id.to_string();
        let task_run_id = task_run_id.to_string();
        let root_session_id = root_session_id.to_string();
        let root_task_run_id = root_task_run_id.to_string();
        let terminal_status = terminal_status.to_string();
        let require_attribution = require_attribution;

        self.db_runtime()?.read_blocking(move |conn| {
            let read = |session_id: &str, task_run_id: &str| -> Result<Vec<ModelUsageBreakdown>, StoreError> {
                let mut statement = conn.prepare(
                    "SELECT provider_id, backend_model, SUM(input_tokens), SUM(output_tokens), SUM(cache_tokens)
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
                    ))
                })?;
                rows.map(|row| {
                    let (provider_id, backend_model, input_tokens, output_tokens, cache_tokens) = row?;
                    let snapshot = pricing.get(&(provider_id, backend_model.clone())).copied();
                    let estimated_cost = snapshot.map(|snapshot| {
                        calculate_model_cost(input_tokens, output_tokens, cache_tokens, snapshot)
                    });
                    Ok(ModelUsageBreakdown {
                        provider_id,
                        backend_model,
                        input_tokens,
                        output_tokens,
                        cache_tokens,
                        pricing_status: if snapshot.is_some() { "priced" } else { "missing" }.to_string(),
                        input_per_million: snapshot.map(|item| item.input_per_million),
                        output_per_million: snapshot.map(|item| item.output_per_million),
                        cache_per_million: snapshot.map(|item| item.cache_per_million),
                        multiplier: snapshot.map(|item| item.multiplier),
                        estimated_cost,
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
                    "SELECT provider_id, backend_model, SUM(input_tokens), SUM(output_tokens), SUM(cache_tokens)
                     FROM ccproxy_stats
                     WHERE root_session_id = ?1 AND root_task_run_id = ?2
                     GROUP BY provider_id, backend_model",
                )?;
                let combined_rows = combined_statement.query_map(params![root_session_id, root_task_run_id], |row| {
                    Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
                })?;
                combined_rows.map(|row| {
                    let (provider_id, backend_model, input_tokens, output_tokens, cache_tokens) = row?;
                    let snapshot = pricing.get(&(provider_id, backend_model.clone())).copied();
                    Ok(ModelUsageBreakdown {
                        provider_id, backend_model, input_tokens, output_tokens, cache_tokens,
                        pricing_status: if snapshot.is_some() { "priced" } else { "missing" }.to_string(),
                        input_per_million: snapshot.map(|item| item.input_per_million),
                        output_per_million: snapshot.map(|item| item.output_per_million),
                        cache_per_million: snapshot.map(|item| item.cache_per_million),
                        multiplier: snapshot.map(|item| item.multiplier),
                        estimated_cost: snapshot.map(|item| calculate_model_cost(input_tokens, output_tokens, cache_tokens, item)),
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
            // without leaking children from an earlier hot-resumed root task.
            let has_sub_agents: bool = is_root_task && conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM workflow_task_usage
                    WHERE root_session_id = ?1 AND root_task_run_id = ?2
                      AND (session_id != ?1 OR task_run_id != ?2)
                )",
                params![root_session_id, root_task_run_id],
                |row| row.get(0),
            )?;
            let with_sub_agents = UsageTotals::from_breakdowns(&combined_breakdowns);
            Ok(WorkflowUsageSummary {
                version: 1,
                terminal_status,
                duration_ms,
                has_sub_agents,
                is_partial: (require_attribution && !self_has_attribution)
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

    #[allow(dead_code)]
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

        let current = store
            .summarize_workflow_task_usage(
                "root-session",
                "root-session:task:2",
                "root-session",
                "root-session:task:2",
                "completed",
                Some(0),
                false,
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
                true,
            )
            .expect("failed to summarize unattributed recovered task");
        assert!(
            recovered.is_partial,
            "unattributed recovery must not look free"
        );
        assert_eq!(recovered.self_usage.total_tokens, 0);
        assert_eq!(recovered.self_usage.estimated_cost, Some(0.0));

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
                false,
            )
            .expect("failed to summarize current stage with child");
        assert!(current.has_sub_agents);
        assert_eq!(current.with_sub_agents.total_tokens, 0);
    }
}
