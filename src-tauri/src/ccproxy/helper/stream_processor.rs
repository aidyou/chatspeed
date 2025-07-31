use bytes::BytesMut;
use reqwest::Response;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::mpsc;

use crate::ccproxy::types::StreamFormat;

/// A processor for handling Server-Sent Events (SSE) streams
///
/// The processor maintains an internal buffer to accumulate incoming chunks and
/// splits them into complete SSE events delimited by double newlines (`\n\n`).
#[derive(Clone)]
pub struct StreamProcessor {
    stop_flag: Arc<AtomicBool>,
}

impl StreamProcessor {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// stops the stream processor
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }

    /// Processes an HTTP response stream into complete SSE events
    ///
    /// # Arguments
    /// * `response` - The streaming HTTP response containing SSE data
    ///
    /// # Returns
    /// A channel receiver that yields `Result<Bytes, String>` where:
    /// - `Ok(Bytes)` contains a complete SSE event (including delimiters)
    /// - `Err(String)` contains error message if chunk reading fails
    ///
    /// # Behavior
    /// - Maintains an 8KB internal buffer for accumulating partial events
    /// - Splits the stream on `\n\n` boundaries per SSE specification
    /// - Automatically stops when the response ends or receiver is dropped
    ///
    /// # Example
    /// ```no_run
    /// let processor = StreamProcessor::new();
    /// let mut event_receiver = processor.process_stream(response).await;
    ///
    /// while let Some(event) = event_receiver.recv().await {
    ///     match event {
    ///         Ok(data) => process_event(data),
    ///         Err(e) => log_error(e),
    ///     }
    /// }
    /// ```
    pub async fn process_stream(
        &self,
        mut response: Response,
        format: &StreamFormat,
    ) -> mpsc::Receiver<Result<bytes::Bytes, String>> {
        let (tx, rx) = mpsc::channel(32);
        let stop_flag = self.stop_flag.clone();

        let deliv = match format {
            StreamFormat::Gemini => b"\r\n",
            _ => b"\n\n",
        };
        let deliv_len = deliv.len();

        tokio::spawn(async move {
            let mut buffer = BytesMut::with_capacity(8192);

            while !stop_flag.load(Ordering::Relaxed) {
                match response.chunk().await {
                    Ok(Some(chunk)) => {
                        buffer.extend_from_slice(&chunk);

                        loop {
                            if stop_flag.load(Ordering::Relaxed) {
                                return Ok::<(), String>(());
                            }

                            match memchr::memmem::find(&buffer, deliv) {
                                Some(pos) => {
                                    let end = pos + deliv_len;
                                    if end > buffer.len() {
                                        break;
                                    }

                                    let event = buffer.split_to(end).freeze();
                                    if tx.send(Ok(event)).await.is_err() {
                                        return Ok::<(), String>(());
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                    Ok(None) => {
                        // Handle remaining data in buffer
                        if !buffer.is_empty() {
                            log::debug!("buffer remain: {}", String::from_utf8_lossy(&buffer));
                            let remaining = buffer.split().freeze();
                            let _ = tx.send(Ok(remaining)).await;
                        }
                        break;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string())).await;
                        break;
                    }
                }
            }
            Ok(())
        });

        rx
    }
}
