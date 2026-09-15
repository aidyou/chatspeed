use crate::ai::model_catalog::pricing::{calculate_cost, UsageBreakdown};
use crate::ccproxy::{
    adapter::unified::{SseStatus, StreamLogRecorder},
    utils::token_estimator::token_usage_is_missing_or_zero,
};
use crate::db::{CcproxyStat, MainStore, PricingConfig};
use serde_json::to_string;
use std::sync::{Arc, Mutex, RwLock};

pub fn finalize_pricing(
    input_tokens: i64,
    output_tokens: i64,
    cache_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
    audio_input_tokens: i64,
    audio_output_tokens: i64,
    pricing: Option<&PricingConfig>,
) -> (Option<f64>, Option<String>, Option<String>) {
    pricing
        .map(|pricing| {
            let cost = calculate_cost(
                UsageBreakdown {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens: cache_tokens,
                    cache_write_tokens,
                    reasoning_tokens,
                    audio_input_tokens,
                    audio_output_tokens,
                },
                pricing,
            );
            (
                Some(cost.total),
                Some("priced".to_string()),
                to_string(pricing).ok(),
            )
        })
        .unwrap_or((None, Some("unpriced".to_string()), None))
}

pub fn record_error_stat(store: &MainStore, stat: CcproxyStat) {
    let _ = store.record_ccproxy_stat(stat);
}

/// This handles both normal completion and premature termination (e.g., client disconnect).
pub struct StreamStatGuard {
    pub log_recorder: Arc<Mutex<StreamLogRecorder>>,
    pub sse_status: Arc<RwLock<SseStatus>>,
    pub main_store: Arc<MainStore>,
    pub client_model: String,
    pub backend_model: String,
    pub provider_id: i64,
    pub provider: String,
    pub protocol: String,
    pub tool_compat_mode: bool,
    pub workflow_session_id: Option<String>,
    pub workflow_task_run_id: Option<String>,
    pub workflow_segment_id: Option<i32>,
    pub root_session_id: Option<String>,
    pub root_task_run_id: Option<String>,
    pub request_kind: Option<String>,
    pub pricing: Option<PricingConfig>,
    /// Opted-in experiment admission settlement handle. When present, the
    /// terminal boundary commits the actual usage (or marks the effect
    /// unknown on stream failure) against the durable budget ledger.
    pub admission: Option<crate::ccproxy::admission::AdmissionSettlement>,
}

impl StreamStatGuard {
    pub fn with_workflow_attribution(mut self, headers: &http::HeaderMap) -> Self {
        let read = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };

        self.workflow_session_id = read("x-cs-workflow-session-id");
        self.workflow_task_run_id = read("x-cs-workflow-task-run-id");
        self.workflow_segment_id =
            read("x-cs-workflow-segment-id").and_then(|value| value.parse::<i32>().ok());
        self.root_session_id = read("x-cs-root-session-id");
        self.root_task_run_id = read("x-cs-root-task-run-id");
        self.request_kind = read("x-cs-request-kind");
        self
    }
}

impl Drop for StreamStatGuard {
    fn drop(&mut self) {
        let (
            input,
            output,
            cache,
            cache_creation,
            reasoning,
            audio_input,
            audio_output,
            has_output,
            stream_failed,
            stream_status_code,
            error_message,
        ) = {
            if let Ok(recorder) = self.log_recorder.lock() {
                (
                    recorder.input_tokens,
                    recorder.output_tokens,
                    recorder.cache_tokens,
                    recorder.cache_creation_tokens,
                    recorder.reasoning_tokens,
                    recorder.audio_input_tokens,
                    recorder.audio_output_tokens,
                    recorder.has_content || recorder.has_thinking || recorder.has_tool_calls,
                    recorder.stream_failed,
                    recorder.stream_status_code,
                    recorder.error_message.clone(),
                )
            } else {
                (
                    None, None, None, None, None, None, None, false, true, 502, None,
                )
            }
        };

        if stream_failed {
            if let Some(settlement) = &self.admission {
                // The effect may have happened but the outcome is unknown:
                // freeze the budget and record one infra failure (INV-5).
                settlement.mark_unknown_blocking("stream_failed", "stream_failure");
            }
            let _ = self.main_store.record_ccproxy_stat(CcproxyStat {
                id: None,
                workflow_session_id: self.workflow_session_id.clone(),
                workflow_task_run_id: self.workflow_task_run_id.clone(),
                workflow_segment_id: self.workflow_segment_id,
                root_session_id: self.root_session_id.clone(),
                root_task_run_id: self.root_task_run_id.clone(),
                request_kind: self.request_kind.clone(),
                client_model: self.client_model.clone(),
                backend_model: self.backend_model.clone(),
                provider_id: Some(self.provider_id),
                provider: self.provider.clone(),
                protocol: self.protocol.clone(),
                tool_compat_mode: if self.tool_compat_mode { 1 } else { 0 },
                status_code: if stream_status_code == 0 {
                    502
                } else {
                    i32::from(stream_status_code)
                },
                error_message: Some(
                    error_message.unwrap_or_else(|| "Backend stream failed".to_string()),
                ),
                input_tokens: 0,
                output_tokens: 0,
                cache_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                audio_input_tokens: 0,
                audio_output_tokens: 0,
                estimated_cost: None,
                pricing_status: Some("unpriced".to_string()),
                pricing_snapshot: None,
                request_at: None,
            });
            return;
        }

        if !has_output {
            if let Some(settlement) = &self.admission {
                // The request was sent, so input tokens were consumed even
                // though no completion content was produced. Settle
                // conservatively at the reserved input bound instead of
                // committing a zero vector (INV-5 / AC-6).
                settlement.commit_conservative_input_blocking();
            }
            return;
        }

        let (est_input, est_output) = if let Ok(status) = self.sse_status.read() {
            (
                status.estimated_input_tokens,
                status.estimated_output_tokens,
            )
        } else {
            (0.0, 0.0)
        };

        let should_estimate =
            token_usage_is_missing_or_zero(&[input, output, cache, cache_creation]);
        let final_input = if should_estimate {
            est_input.ceil() as u64
        } else {
            input.unwrap_or(0)
        };
        let final_output = if should_estimate {
            est_output.ceil() as u64
        } else {
            output.unwrap_or(0)
        };
        let final_cache = cache.unwrap_or(0);
        let final_cache_write = cache_creation.unwrap_or(0);
        let final_reasoning = reasoning.unwrap_or(0);
        let (estimated_cost, pricing_status, pricing_snapshot) = finalize_pricing(
            final_input as i64,
            final_output as i64,
            final_cache as i64,
            final_cache_write as i64,
            final_reasoning as i64,
            audio_input.unwrap_or(0) as i64,
            audio_output.unwrap_or(0) as i64,
            self.pricing.as_ref(),
        );

        log::debug!(
            "StreamStatGuard dropped. Recording stat: provider='{}', model='{}', tokens={}/{}/{}",
            &self.provider,
            &self.backend_model,
            final_input,
            final_output,
            final_cache
        );

        if let Some(settlement) = &self.admission {
            settlement.commit_usage_blocking(crate::budget::types::BudgetVector {
                input_tokens: final_input,
                output_tokens: final_output,
                cache_read_tokens: final_cache,
                cache_write_tokens: final_cache_write,
                ..crate::budget::types::BudgetVector::ZERO
            });
        }

        let _ = self.main_store.record_ccproxy_stat(CcproxyStat {
            id: None,
            workflow_session_id: self.workflow_session_id.clone(),
            workflow_task_run_id: self.workflow_task_run_id.clone(),
            workflow_segment_id: self.workflow_segment_id,
            root_session_id: self.root_session_id.clone(),
            root_task_run_id: self.root_task_run_id.clone(),
            request_kind: self.request_kind.clone(),
            client_model: self.client_model.clone(),
            backend_model: self.backend_model.clone(),
            provider_id: Some(self.provider_id),
            provider: self.provider.clone(),
            protocol: self.protocol.clone(),
            tool_compat_mode: if self.tool_compat_mode { 1 } else { 0 },
            status_code: 200,
            error_message: None,
            input_tokens: final_input as i64,
            output_tokens: final_output as i64,
            cache_tokens: final_cache as i64,
            cache_write_tokens: final_cache_write as i64,
            reasoning_tokens: final_reasoning as i64,
            audio_input_tokens: audio_input.unwrap_or(0) as i64,
            audio_output_tokens: audio_output.unwrap_or(0) as i64,
            estimated_cost,
            pricing_status,
            pricing_snapshot,
            request_at: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn ccproxy_usage_attribution_stream_guard_persists_all_fields() {
        let directory = tempdir().unwrap();
        let store = Arc::new(MainStore::new(directory.path().join("stream-stat.db")).unwrap());
        let mut headers = http::HeaderMap::new();
        for (name, value) in [
            ("x-cs-workflow-session-id", "workflow-session"),
            ("x-cs-workflow-task-run-id", "workflow-session:task:1"),
            ("x-cs-workflow-segment-id", "3"),
            ("x-cs-root-session-id", "root-session"),
            ("x-cs-root-task-run-id", "root-session:task:1"),
            ("x-cs-request-kind", "react"),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        let log_recorder = Arc::new(Mutex::new(StreamLogRecorder {
            has_content: true,
            ..StreamLogRecorder::new("stream".to_string(), "backend".to_string())
        }));
        let guard = StreamStatGuard {
            log_recorder,
            sse_status: Arc::new(RwLock::new(SseStatus::new(
                "message".to_string(),
                "alias".to_string(),
                false,
                12.0,
            ))),
            main_store: store.clone(),
            client_model: "alias".to_string(),
            backend_model: "backend".to_string(),
            provider_id: 1,
            provider: "provider".to_string(),
            protocol: "openai".to_string(),
            tool_compat_mode: false,
            workflow_session_id: None,
            workflow_task_run_id: None,
            workflow_segment_id: None,
            root_session_id: None,
            root_task_run_id: None,
            request_kind: None,
            pricing: None,
            admission: None,
        }
        .with_workflow_attribution(&headers);
        drop(guard);
        let runtime = store.db_runtime().unwrap();
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .unwrap()
            .unwrap();
        let row = runtime
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT workflow_session_id, workflow_task_run_id, workflow_segment_id,
                            root_session_id, root_task_run_id, request_kind,
                            input_tokens, output_tokens, cache_tokens
                     FROM ccproxy_stats",
                    [],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i32>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                        ))
                    },
                )?)
            })
            .await
            .unwrap();
        assert_eq!(
            row,
            (
                "workflow-session".to_string(),
                "workflow-session:task:1".to_string(),
                3,
                "root-session".to_string(),
                "root-session:task:1".to_string(),
                "react".to_string(),
                12,
                0,
                0,
            )
        );
    }

    #[tokio::test]
    async fn stream_guard_preserves_explicit_zero_input_when_output_exists() {
        let directory = tempdir().unwrap();
        let store =
            Arc::new(MainStore::new(directory.path().join("stream-zero-input.db")).unwrap());
        let log_recorder = Arc::new(Mutex::new(StreamLogRecorder {
            input_tokens: Some(0),
            output_tokens: Some(7),
            has_content: true,
            ..StreamLogRecorder::new("stream".to_string(), "backend".to_string())
        }));
        let guard = StreamStatGuard {
            log_recorder,
            sse_status: Arc::new(RwLock::new(SseStatus::new(
                "message".to_string(),
                "alias".to_string(),
                false,
                120.0,
            ))),
            main_store: store.clone(),
            client_model: "alias".to_string(),
            backend_model: "backend".to_string(),
            provider_id: 1,
            provider: "provider".to_string(),
            protocol: "openai".to_string(),
            tool_compat_mode: false,
            workflow_session_id: None,
            workflow_task_run_id: None,
            workflow_segment_id: None,
            root_session_id: None,
            root_task_run_id: None,
            request_kind: None,
            pricing: None,
            admission: None,
        };
        drop(guard);
        let runtime = store.db_runtime().unwrap();
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .unwrap()
            .unwrap();
        let row = runtime
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT input_tokens, output_tokens FROM ccproxy_stats",
                    [],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(row, (0, 7));
    }

    #[tokio::test]
    async fn stream_guard_skips_empty_responses() {
        let directory = tempdir().unwrap();
        let store = Arc::new(MainStore::new(directory.path().join("empty-stream.db")).unwrap());
        let guard = StreamStatGuard {
            log_recorder: Arc::new(Mutex::new(StreamLogRecorder::new(
                "stream".to_string(),
                "backend".to_string(),
            ))),
            sse_status: Arc::new(RwLock::new(SseStatus::new(
                "message".to_string(),
                "alias".to_string(),
                false,
                120.0,
            ))),
            main_store: store.clone(),
            client_model: "alias".to_string(),
            backend_model: "backend".to_string(),
            provider_id: 1,
            provider: "provider".to_string(),
            protocol: "openai".to_string(),
            tool_compat_mode: false,
            workflow_session_id: None,
            workflow_task_run_id: None,
            workflow_segment_id: None,
            root_session_id: None,
            root_task_run_id: None,
            request_kind: None,
            pricing: None,
            admission: None,
        };
        drop(guard);
        let runtime = store.db_runtime().unwrap();
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .unwrap()
            .unwrap();
        let count = runtime
            .read(|conn| {
                Ok(
                    conn.query_row("SELECT COUNT(*) FROM ccproxy_stats", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn stream_guard_settles_conservative_input_when_no_output() {
        use crate::budget::types::{BudgetEnvelope, CapLimit, EffectKind, MoneyMode, ResourceCaps};
        use crate::ccproxy::admission::{admit_before_send, LlmGateInput};
        use std::collections::BTreeSet;

        let directory = tempdir().unwrap();
        let store = Arc::new(MainStore::new(directory.path().join("no-output.db")).unwrap());
        // Create a money-budget scope chain keyed to the request scope.
        let envelope = BudgetEnvelope {
            caps: ResourceCaps {
                input_tokens: CapLimit::HardCap(1_000_000),
                output_tokens: CapLimit::HardCap(1_000_000),
                cache_read_tokens: CapLimit::NotApplicable,
                cache_write_tokens: CapLimit::NotApplicable,
                wall_time_ms: CapLimit::HardCap(600_000),
                tool_calls: CapLimit::HardCap(100),
                processes: CapLimit::HardCap(10),
                disk_bytes: CapLimit::NotApplicable,
                network_bytes: CapLimit::NotApplicable,
                concurrency: CapLimit::HardCap(4),
                money: CapLimit::HardCap(100_000_000),
            },
            required_dimensions: BTreeSet::new(),
            money_mode: MoneyMode::Money {
                currency_code: "cny".into(),
                cap_money_micros: 100_000_000,
                pricing: crate::budget::types::PricingSnapshot {
                    currency_code: "cny".into(),
                    provider_id: "provider".into(),
                    model_id: "backend-model".into(),
                    input_micros_per_million: 1_000_000,
                    output_micros_per_million: 2_000_000,
                    cache_read_micros_per_million: 0,
                    cache_write_micros_per_million: 0,
                    reasoning_micros_per_million: None,
                    multiplier_micros: 1_000_000,
                    source_hash: "hash".into(),
                },
            },
            max_attempts: 1,
            infra_failure_threshold: 5,
            reservation_lease_ms: 600_000,
        };
        for (kind, id, parent) in [
            (crate::budget::types::ScopeKind::Campaign, "camp-1", None),
            (
                crate::budget::types::ScopeKind::Candidate,
                "cand-1",
                Some("camp-1"),
            ),
            (
                crate::budget::types::ScopeKind::Trial,
                "trial-1",
                Some("cand-1"),
            ),
            (
                crate::budget::types::ScopeKind::Request,
                "req-1",
                Some("trial-1"),
            ),
        ] {
            store
                .create_budget_scope(crate::db::budget::NewBudgetScope {
                    scope_id: id.into(),
                    scope_kind: kind,
                    parent_scope_id: parent.map(|value| value.to_string()),
                    envelope: envelope.clone(),
                    now_ms: 1_700_000_000_000,
                })
                .unwrap();
        }

        let mut headers = http::HeaderMap::new();
        headers.insert("x-cs-internal-request", "true".parse().unwrap());
        headers.insert(
            "authorization",
            format!(
                "Bearer {}",
                crate::constants::INTERNAL_CCPROXY_API_KEY.read()
            )
            .parse()
            .unwrap(),
        );
        headers.insert(
            crate::ccproxy::admission::ADMISSION_HEADER,
            serde_json::json!({
                "scope_chain": {
                    "request_id": "req-1",
                    "trial_id": "trial-1",
                    "candidate_id": "cand-1",
                    "campaign_id": "camp-1"
                },
                "effect_id": "eff-1",
                "idempotency_key": "idem-1",
                "attempt": 1
            })
            .to_string()
            .parse()
            .unwrap(),
        );
        let lease = admit_before_send(
            &store,
            &headers,
            LlmGateInput {
                provider_id: "provider".into(),
                model_id: "backend-model".into(),
                estimated_input_tokens: 100,
                effect_kind: EffectKind::LlmCompletion,
                max_output_tokens: Some(50),
            },
        )
        .await
        .unwrap()
        .expect("admission should succeed");
        let settlement = lease.settlement();

        // A stream that terminates with no output: the guard must settle
        // conservatively at the reserved input bound, never at zero.
        let guard = StreamStatGuard {
            log_recorder: Arc::new(Mutex::new(StreamLogRecorder::new(
                "stream".to_string(),
                "backend".to_string(),
            ))),
            sse_status: Arc::new(RwLock::new(SseStatus::new(
                "message".to_string(),
                "alias".to_string(),
                false,
                120.0,
            ))),
            main_store: store.clone(),
            client_model: "alias".to_string(),
            backend_model: "backend".to_string(),
            provider_id: 1,
            provider: "provider".to_string(),
            protocol: "openai".to_string(),
            tool_compat_mode: false,
            workflow_session_id: None,
            workflow_task_run_id: None,
            workflow_segment_id: None,
            root_session_id: None,
            root_task_run_id: None,
            request_kind: None,
            pricing: None,
            admission: Some(settlement),
        };
        drop(guard);

        let status = store.get_budget_scope_status("req-1").unwrap().unwrap();
        assert_eq!(
            status.committed.input_tokens, 100,
            "input must settle at the reserved bound, never zero"
        );
        assert_eq!(
            status.committed.money_micros, 100,
            "money must accumulate (100 tokens at 1 unit/M), never zero"
        );
        assert_eq!(status.committed.output_tokens, 0);
        assert_eq!(status.reserved.input_tokens, 0);
    }
}
