use crate::ccproxy::adapter::unified::{SseStatus, StreamLogRecorder};
use crate::db::{CcproxyStat, MainStore};
use std::sync::{Arc, Mutex, RwLock};

/// A Drop guard to ensure proxy statistics are recorded when a stream ends.
/// This handles both normal completion and premature termination (e.g., client disconnect).
pub struct StreamStatGuard {
    pub log_recorder: Arc<Mutex<StreamLogRecorder>>,
    pub sse_status: Arc<RwLock<SseStatus>>,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
    pub client_model: String,
    pub backend_model: String,
    pub provider: String,
    pub protocol: String,
    pub tool_compat_mode: bool,
}

impl Drop for StreamStatGuard {
    fn drop(&mut self) {
        let (input, output) = {
            if let Ok(recorder) = self.log_recorder.lock() {
                (
                    recorder.input_tokens.unwrap_or(0),
                    recorder.output_tokens.unwrap_or(0),
                )
            } else {
                (0, 0)
            }
        };

        if let Ok(store) = self.main_store.read() {
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
                    "StreamStatGuard dropped. Recording stat: provider='{}', model='{}', tokens={}/{}",
                    &self.provider,
                    &self.backend_model,
                    final_input,
                    final_output
                );

                let _ = store.record_ccproxy_stat(CcproxyStat {
                    id: None,
                    client_model: self.client_model.clone(),
                    backend_model: self.backend_model.clone(),
                    provider: self.provider.clone(),
                    protocol: self.protocol.clone(),
                    tool_compat_mode: if self.tool_compat_mode { 1 } else { 0 },
                    status_code: 200,
                    error_message: None,
                    input_tokens: final_input as i64,
                    output_tokens: final_output as i64,
                    cache_tokens: 0,
                    request_at: None,
                });
            }
        }
    }
}
