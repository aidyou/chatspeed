//! Numeric-only JSON summaries: no transcript, workflow IDs, paths or credentials.
use serde::Serialize;
use std::{fs::OpenOptions, io::Write, path::Path, time::Instant};

/// Cumulative UI-application timings; these are not GPU presentation timings.
#[derive(Clone, Default, Serialize)]
pub struct Metrics {
    #[serde(skip)]
    pub started: Option<Instant>,
    pub timestamp_ms: u64,
    pub elapsed_ms: u64,
    pub samples: u64,
    pub events: u64,
    pub event_bytes: u64,
    pub queue_samples: u64,
    pub queue_depth_peak: usize,
    pub markdown_samples: u64,
    pub snapshot_samples: u64,
    pub switch_samples: u64,
    pub ui_batches: u64,
    pub queue_us_total: u64,
    pub queue_us_max: u64,
    pub markdown_update_us: u64,
    pub snapshot_decode_us: u64,
    pub switch_ui_applied_us: u64,
    pub switch_ui_applied_us_total: u64,
    pub switch_ui_applied_us_max: u64,
    pub reconnects: u64,
    pub resets: u64,
    pub stale_responses: u64,
    pub cached_messages: usize,
    pub cached_markdown: usize,
    pub rss_bytes: Option<u64>,
    pub cpu_ticks: Option<u64>,
}

impl Metrics {
    pub fn queue(&mut self, queued: Instant) {
        self.queue_samples += 1;
        let us = queued.elapsed().as_micros().min(u64::MAX as u128) as u64;
        self.queue_us_total = self.queue_us_total.saturating_add(us);
        self.queue_us_max = self.queue_us_max.max(us);
    }

    pub fn sample(&mut self) {
        self.samples += 1;
        self.elapsed_ms = self
            .started
            .get_or_insert_with(Instant::now)
            .elapsed()
            .as_millis() as u64;
        self.timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        #[cfg(target_os = "linux")]
        {
            // VmRSS is already in KiB; no guessed page size or fake zero on failure.
            self.rss_bytes = std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("VmRSS:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|v| v * 1024)
                });
            self.cpu_ticks = std::fs::read_to_string("/proc/self/stat")
                .ok()
                .and_then(|s| {
                    let fields: Vec<_> = s.rsplit_once(')')?.1.split_whitespace().collect();
                    Some(
                        fields.get(11)?.parse::<u64>().ok()?
                            + fields.get(12)?.parse::<u64>().ok()?,
                    )
                });
        }
    }

    /// Create the sink before opening a window so path failures cannot stay hidden.
    pub fn prepare(path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        OpenOptions::new().create(true).append(true).open(path)?;
        Ok(())
    }

    pub fn emit(&mut self, path: Option<&Path>) -> std::io::Result<()> {
        self.sample();
        let line = serde_json::to_string(self)?;
        if let Some(path) = path {
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            writeln!(file, "{line}")?;
        } else {
            eprintln!("{line}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_metrics_parent_is_created_and_records_are_appended() {
        let root = std::env::temp_dir().join(format!("observer-metrics-{}", std::process::id()));
        let path = root.join("nested/metrics.jsonl");
        Metrics::prepare(&path).unwrap();
        let mut metrics = Metrics::default();
        metrics.emit(Some(&path)).unwrap();
        metrics.emit(Some(&path)).unwrap();
        let records = std::fs::read_to_string(&path).unwrap();
        assert_eq!(records.lines().count(), 2);
        for line in records.lines() {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(record["timestamp_ms"].as_u64().unwrap() > 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn summary_contains_only_numeric_or_unavailable_values() {
        let value = serde_json::to_value(Metrics::default()).unwrap();
        for value in value.as_object().unwrap().values() {
            assert!(value.is_number() || value.is_null());
        }
        let line = serde_json::to_string(&Metrics::default()).unwrap();
        assert!(!line.contains("token"));
        assert!(!line.contains("content"));
        assert!(!line.contains("message\""));
    }
}
