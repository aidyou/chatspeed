use crate::ccproxy::adapter::unified::{SseStatus, StreamLogRecorder};
use crate::db::{CcproxyStat, MainStore};
use std::sync::{Arc, Mutex, RwLock};

/// A Drop guard to ensure proxy statistics are recorded when a stream ends.
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
        let (input, output, cache) = {
            if let Ok(recorder) = self.log_recorder.lock() {
                (
                    recorder.input_tokens.unwrap_or(0),
                    recorder.output_tokens.unwrap_or(0),
                    recorder.cache_tokens.unwrap_or(0),
                )
            } else {
                (0, 0, 0)
            }
        };

        let store = &self.main_store;
        let (est_input, est_output) = if let Ok(status) = self.sse_status.read() {
            (
                status.estimated_input_tokens,
                status.estimated_output_tokens,
            )
        } else {
            (0.0, 0.0)
        };

        let final_input = if input > 0 {
            input
        } else {
            est_input.ceil() as u64
        };
        let final_output = if output > 0 {
            output
        } else {
            est_output.ceil() as u64
        };

        // Only record if we actually processed some tokens or input
        if final_input > 0 || final_output > 0 {
            #[cfg(debug_assertions)]
            log::debug!(
                "StreamStatGuard dropped. Recording stat: provider='{}', model='{}', tokens={}/{}/{}",
                &self.provider,
                &self.backend_model,
                final_input,
                final_output,
                cache
            );

            let _ = store.record_ccproxy_stat(CcproxyStat {
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
                cache_tokens: cache as i64,
                request_at: None,
            });
        }
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
        let guard = StreamStatGuard {
            log_recorder: Arc::new(Mutex::new(StreamLogRecorder::new(
                "stream".to_string(),
                "backend".to_string(),
            ))),
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
        }
        .with_workflow_attribution(&headers);
        drop(guard);
        let runtime = store.db_runtime().unwrap();
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .unwrap()
            .unwrap();
        let row = runtime.read(|conn| Ok(conn.query_row(
            "SELECT workflow_session_id, workflow_task_run_id, workflow_segment_id, root_session_id, root_task_run_id, request_kind, input_tokens FROM ccproxy_stats", [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i32>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, i64>(6)?)),
        )?)).await.unwrap();
        assert_eq!(
            row,
            (
                "workflow-session".to_string(),
                "workflow-session:task:1".to_string(),
                3,
                "root-session".to_string(),
                "root-session:task:1".to_string(),
                "react".to_string(),
                12
            )
        );
    }
}
