use futures_util::Stream;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::task::{Context, Poll};
use tokio::time::{interval, Duration};

/// Monitor client connection status and handle disconnection events
#[derive(Clone)]
pub struct ConnectionMonitor {
    is_connected: Arc<AtomicBool>,
    disconnect_callbacks: Arc<tokio::sync::Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
}

impl ConnectionMonitor {
    /// Create a new connection monitor with initial connected state
    pub fn new() -> Self {
        Self {
            is_connected: Arc::new(AtomicBool::new(true)),
            disconnect_callbacks: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    /// Check if the client is still connected
    ///
    /// # Returns
    /// `true` if the client is connected, `false` otherwise
    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Relaxed)
    }

    /// Mark the client as disconnected and execute all registered callbacks
    ///
    /// This method is idempotent - calling it multiple times will only execute
    /// callbacks once.
    pub async fn mark_disconnected(&self) {
        if self.is_connected.swap(false, Ordering::Relaxed) {
            log::info!("Client disconnected, executing cleanup callbacks");

            let callbacks = self.disconnect_callbacks.lock().await;
            for callback in callbacks.iter() {
                callback();
            }
        }
    }

    /// Register a callback to be executed when the client disconnects
    ///
    /// # Arguments
    /// * `callback` - A function to be called when disconnection is detected
    pub async fn on_disconnect<F>(&self, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut callbacks = self.disconnect_callbacks.lock().await;
        callbacks.push(Box::new(callback));
    }
}

/// A wrapper stream that monitors client disconnection events
///
/// This stream wrapper automatically detects when the underlying stream fails
/// or ends, and triggers disconnection callbacks accordingly.
pub struct MonitoredStream<S> {
    inner: S,
    monitor: ConnectionMonitor,
    heartbeat_monitor: Option<HeartbeatMonitor>,
}

impl<S> MonitoredStream<S> {
    /// Create a new monitored stream with heartbeat monitoring
    ///
    /// # Arguments
    /// * `stream` - The underlying stream to monitor
    /// * `monitor` - Connection monitor for state management
    /// * `heartbeat_monitor` - Heartbeat monitor for activity tracking
    pub fn new_with_heartbeat(
        stream: S,
        monitor: ConnectionMonitor,
        heartbeat_monitor: HeartbeatMonitor,
    ) -> Self {
        Self {
            inner: stream,
            monitor,
            heartbeat_monitor: Some(heartbeat_monitor),
        }
    }
}

impl<S, T, E> Stream for MonitoredStream<S>
where
    S: Stream<Item = Result<T, E>> + Unpin,
{
    type Item = Result<T, E>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.monitor.is_connected() {
            return Poll::Ready(None);
        }

        match Pin::new(&mut self.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(item))) => {
                // Update activity timestamp for heartbeat monitoring
                if let Some(ref heartbeat) = self.heartbeat_monitor {
                    heartbeat.update_activity();
                }
                Poll::Ready(Some(Ok(item)))
            }
            Poll::Ready(Some(Err(e))) => {
                // Stream error may indicate client disconnection
                tokio::spawn({
                    let monitor = self.monitor.clone();
                    async move {
                        monitor.mark_disconnected().await;
                    }
                });
                Poll::Ready(Some(Err(e)))
            }
            Poll::Ready(None) => {
                // Stream ended - mark as disconnected
                tokio::spawn({
                    let monitor = self.monitor.clone();
                    async move {
                        monitor.mark_disconnected().await;
                    }
                });
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Heartbeat monitor that periodically checks client connection status
///
/// This monitor tracks client activity and automatically marks connections as
/// disconnected if no activity is detected within the timeout period.
pub struct HeartbeatMonitor {
    monitor: ConnectionMonitor,
    interval_seconds: u64,
    last_activity: Arc<std::sync::Mutex<std::time::Instant>>,
}

impl HeartbeatMonitor {
    /// Create a new heartbeat monitor
    ///
    /// # Arguments
    /// * `monitor` - Connection monitor to update when timeouts occur
    /// * `interval_seconds` - How often to check for activity (in seconds)
    pub fn new(monitor: ConnectionMonitor, interval_seconds: u64) -> Self {
        Self {
            monitor,
            interval_seconds,
            last_activity: Arc::new(std::sync::Mutex::new(std::time::Instant::now())),
        }
    }

    /// Update the last activity timestamp
    ///
    /// This should be called whenever client data is received to reset
    /// the timeout counter.
    pub fn update_activity(&self) {
        if let Ok(mut last) = self.last_activity.lock() {
            *last = std::time::Instant::now();
            log::trace!("Client activity detected, updated heartbeat timestamp");
        }
    }

    /// Start the heartbeat monitoring task
    ///
    /// This spawns a background task that periodically checks for client activity.
    /// If no activity is detected within 3x the interval period, the connection
    /// is marked as disconnected.
    ///
    /// # Returns
    /// A `JoinHandle` for the background monitoring task
    pub fn start_heartbeat(&self) -> tokio::task::JoinHandle<()> {
        let monitor = self.monitor.clone();
        let last_activity = self.last_activity.clone();
        let interval_duration = Duration::from_secs(self.interval_seconds);
        let timeout_duration = Duration::from_secs(self.interval_seconds * 3); // 3x interval as timeout

        tokio::spawn(async move {
            let mut interval = interval(interval_duration);

            while monitor.is_connected() {
                interval.tick().await;

                // Check if last activity exceeds timeout threshold
                let should_disconnect = if let Ok(last) = last_activity.lock() {
                    last.elapsed() > timeout_duration
                } else {
                    false
                };

                if should_disconnect {
                    log::warn!(
                        "Client connection timeout detected (no activity for {}s), marking as disconnected",
                        timeout_duration.as_secs()
                    );
                    monitor.mark_disconnected().await;
                    break;
                } else {
                    log::debug!("Heartbeat check - client still connected");
                }
            }

            log::info!("Heartbeat monitor stopped");
        })
    }
}
