use crate::db::{CcproxyStat, StoreError};
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc as std_mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

const DEFAULT_READER_COUNT: usize = 2;
const DEFAULT_QUEUE_CAPACITY: usize = 256;
const TELEMETRY_HIGH_WATER_WARNING: usize = 1_000;
const SLOW_ENQUEUE_WARNING: Duration = Duration::from_millis(100);
const SLOW_JOB_WARNING: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
pub struct DbRuntimeMetrics {
    write_queue_high_water: AtomicUsize,
    read_queue_high_water: AtomicUsize,
    telemetry_pending_high_water: AtomicUsize,
    write_jobs_completed: AtomicU64,
    read_jobs_completed: AtomicU64,
    telemetry_batches_completed: AtomicU64,
    telemetry_records_completed: AtomicU64,
    failed_jobs: AtomicU64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DbRuntimeMetricsSnapshot {
    pub write_queue_high_water: usize,
    pub read_queue_high_water: usize,
    pub telemetry_pending_high_water: usize,
    pub write_jobs_completed: u64,
    pub read_jobs_completed: u64,
    pub telemetry_batches_completed: u64,
    pub telemetry_records_completed: u64,
    pub failed_jobs: u64,
}

impl DbRuntimeMetrics {
    fn observe_high_water(high_water: &AtomicUsize, value: usize) {
        let mut current = high_water.load(Ordering::Relaxed);
        while value > current {
            match high_water.compare_exchange_weak(
                current,
                value,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => current = observed,
            }
        }
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> DbRuntimeMetricsSnapshot {
        DbRuntimeMetricsSnapshot {
            write_queue_high_water: self.write_queue_high_water.load(Ordering::Relaxed),
            read_queue_high_water: self.read_queue_high_water.load(Ordering::Relaxed),
            telemetry_pending_high_water: self.telemetry_pending_high_water.load(Ordering::Relaxed),
            write_jobs_completed: self.write_jobs_completed.load(Ordering::Relaxed),
            read_jobs_completed: self.read_jobs_completed.load(Ordering::Relaxed),
            telemetry_batches_completed: self.telemetry_batches_completed.load(Ordering::Relaxed),
            telemetry_records_completed: self.telemetry_records_completed.load(Ordering::Relaxed),
            failed_jobs: self.failed_jobs.load(Ordering::Relaxed),
        }
    }
}

type DbJob = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

enum WorkerMessage {
    Job(DbJob),
    Shutdown(oneshot::Sender<()>),
}

struct Worker {
    sender: mpsc::Sender<WorkerMessage>,
    join_handle: Mutex<Option<JoinHandle<()>>>,
}

enum TelemetryMessage {
    Stat(CcproxyStat),
    Flush(oneshot::Sender<()>),
    Shutdown(oneshot::Sender<()>),
}

struct TelemetryIngress {
    sender: std_mpsc::Sender<TelemetryMessage>,
    pending: Arc<AtomicUsize>,
    join_handle: Mutex<Option<JoinHandle<()>>>,
}

impl TelemetryIngress {
    fn flush_blocking(&self) -> Result<(), StoreError> {
        let (ack_sender, ack_receiver) = oneshot::channel();
        self.sender
            .send(TelemetryMessage::Flush(ack_sender))
            .map_err(|_| StoreError::RuntimeClosed)?;
        ack_receiver
            .blocking_recv()
            .map_err(|_| StoreError::RuntimeClosed)
    }

    fn enqueue(&self, stat: CcproxyStat) -> Result<(), StoreError> {
        let pending = self.pending.fetch_add(1, Ordering::Relaxed) + 1;
        if pending == TELEMETRY_HIGH_WATER_WARNING {
            log::warn!(
                "CCProxy telemetry backlog reached {} records; persistence is lagging",
                TELEMETRY_HIGH_WATER_WARNING
            );
        }
        if self.sender.send(TelemetryMessage::Stat(stat)).is_err() {
            self.pending.fetch_sub(1, Ordering::Relaxed);
            log::error!("CCProxy telemetry ingress is unavailable; statistic was not queued");
            return Err(StoreError::RuntimeClosed);
        }
        Ok(())
    }

    fn shutdown_blocking(&self) -> Result<(), StoreError> {
        let (ack_sender, ack_receiver) = oneshot::channel();
        self.sender
            .send(TelemetryMessage::Shutdown(ack_sender))
            .map_err(|_| StoreError::RuntimeClosed)?;
        ack_receiver
            .blocking_recv()
            .map_err(|_| StoreError::RuntimeClosed)?;
        if let Some(join_handle) = self.join_handle.lock().take() {
            join_handle
                .join()
                .map_err(|_| StoreError::WorkerFailed("telemetry worker panicked".to_string()))?;
        }
        Ok(())
    }

    #[cfg(test)]
    async fn shutdown(&self) -> Result<(), StoreError> {
        let (ack_sender, ack_receiver) = oneshot::channel();
        self.sender
            .send(TelemetryMessage::Shutdown(ack_sender))
            .map_err(|_| StoreError::RuntimeClosed)?;
        ack_receiver.await.map_err(|_| StoreError::RuntimeClosed)?;
        if let Some(join_handle) = self.join_handle.lock().take() {
            tokio::task::spawn_blocking(move || join_handle.join())
                .await
                .map_err(|error| StoreError::WorkerFailed(error.to_string()))?
                .map_err(|_| StoreError::WorkerFailed("telemetry worker panicked".to_string()))?;
        }
        Ok(())
    }
}

impl Worker {
    fn barrier_blocking(&self) -> Result<(), StoreError> {
        let (result_sender, result_receiver) = oneshot::channel();
        let job: DbJob = Box::new(move |_| {
            let _ = result_sender.send(());
        });
        self.sender
            .blocking_send(WorkerMessage::Job(job))
            .map_err(|_| StoreError::RuntimeClosed)?;
        result_receiver
            .blocking_recv()
            .map_err(|_| StoreError::RuntimeClosed)
    }

    fn shutdown_blocking(&self) -> Result<(), StoreError> {
        let (ack_sender, ack_receiver) = oneshot::channel();
        self.sender
            .blocking_send(WorkerMessage::Shutdown(ack_sender))
            .map_err(|_| StoreError::RuntimeClosed)?;
        ack_receiver
            .blocking_recv()
            .map_err(|_| StoreError::RuntimeClosed)?;
        if let Some(join_handle) = self.join_handle.lock().take() {
            join_handle
                .join()
                .map_err(|_| StoreError::WorkerFailed("worker thread panicked".to_string()))?;
        }
        Ok(())
    }

    #[cfg(test)]
    async fn shutdown(&self) -> Result<(), StoreError> {
        let (ack_sender, ack_receiver) = oneshot::channel();
        self.sender
            .send(WorkerMessage::Shutdown(ack_sender))
            .await
            .map_err(|_| StoreError::RuntimeClosed)?;
        ack_receiver.await.map_err(|_| StoreError::RuntimeClosed)?;

        if let Some(join_handle) = self.join_handle.lock().take() {
            tokio::task::spawn_blocking(move || join_handle.join())
                .await
                .map_err(|error| StoreError::WorkerFailed(error.to_string()))?
                .map_err(|_| StoreError::WorkerFailed("worker thread panicked".to_string()))?;
        }
        Ok(())
    }
}

/// Owns the SQLite connections used by the application database runtime.
///
/// File-backed databases use one writer and two read-only workers. In-memory databases use
/// the writer for reads because independent SQLite in-memory connections do not share state.
pub struct DbRuntime {
    writer: Worker,
    readers: Vec<Worker>,
    telemetry: TelemetryIngress,
    next_reader: AtomicUsize,
    metrics: Arc<DbRuntimeMetrics>,
    accepting_jobs: AtomicBool,
    is_memory: bool,
}

impl DbRuntime {
    pub fn open<P: AsRef<Path>>(db_path: P) -> Result<Self, StoreError> {
        Self::open_with_options(db_path, DEFAULT_READER_COUNT, DEFAULT_QUEUE_CAPACITY)
    }

    fn open_with_options<P: AsRef<Path>>(
        db_path: P,
        reader_count: usize,
        queue_capacity: usize,
    ) -> Result<Self, StoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        let is_memory = db_path == Path::new(":memory:");
        let metrics = Arc::new(DbRuntimeMetrics::default());
        let writer = Self::spawn_worker(
            "db-writer",
            open_writer_connection(&db_path)?,
            queue_capacity,
            Arc::clone(&metrics),
            true,
        )?;

        let mut readers = Vec::new();
        if !is_memory {
            for index in 0..reader_count {
                readers.push(Self::spawn_worker(
                    &format!("db-reader-{index}"),
                    open_reader_connection(&db_path)?,
                    queue_capacity,
                    Arc::clone(&metrics),
                    false,
                )?);
            }
        }

        let telemetry = Self::spawn_telemetry(writer.sender.clone(), Arc::clone(&metrics))?;

        Ok(Self {
            writer,
            readers,
            telemetry,
            next_reader: AtomicUsize::new(0),
            metrics,
            accepting_jobs: AtomicBool::new(true),
            is_memory,
        })
    }

    fn spawn_worker(
        name: &str,
        mut connection: Connection,
        queue_capacity: usize,
        metrics: Arc<DbRuntimeMetrics>,
        is_writer: bool,
    ) -> Result<Worker, StoreError> {
        let (sender, mut receiver) = mpsc::channel(queue_capacity);
        let worker_name = name.to_string();
        let join_handle = std::thread::Builder::new()
            .name(worker_name)
            .spawn(move || {
                while let Some(message) = receiver.blocking_recv() {
                    match message {
                        WorkerMessage::Job(job) => {
                            job(&mut connection);
                            if is_writer {
                                metrics.write_jobs_completed.fetch_add(1, Ordering::Relaxed);
                            } else {
                                metrics.read_jobs_completed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        WorkerMessage::Shutdown(ack_sender) => {
                            log::debug!("Database worker is shutting down");
                            let _ = ack_sender.send(());
                            break;
                        }
                    }
                }
            })
            .map_err(|error| StoreError::WorkerFailed(error.to_string()))?;

        Ok(Worker {
            sender,
            join_handle: Mutex::new(Some(join_handle)),
        })
    }

    fn spawn_telemetry(
        writer_sender: mpsc::Sender<WorkerMessage>,
        metrics: Arc<DbRuntimeMetrics>,
    ) -> Result<TelemetryIngress, StoreError> {
        let (sender, receiver) = std_mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let worker_pending = Arc::clone(&pending);
        let join_handle = std::thread::Builder::new()
            .name("db-telemetry".to_string())
            .spawn(move || {
                let mut batch = Vec::with_capacity(100);
                let mut flush_ack = None;
                let mut shutdown_ack = None;
                loop {
                    match receiver.recv_timeout(Duration::from_millis(25)) {
                        Ok(TelemetryMessage::Stat(stat)) => batch.push(stat),
                        Ok(TelemetryMessage::Flush(ack_sender)) => flush_ack = Some(ack_sender),
                        Ok(TelemetryMessage::Shutdown(ack_sender)) => shutdown_ack = Some(ack_sender),
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                    }

                    while batch.len() < 100 {
                        match receiver.try_recv() {
                            Ok(TelemetryMessage::Stat(stat)) => batch.push(stat),
                            Ok(TelemetryMessage::Flush(ack_sender)) => flush_ack = Some(ack_sender),
                            Ok(TelemetryMessage::Shutdown(ack_sender)) => shutdown_ack = Some(ack_sender),
                            Err(std_mpsc::TryRecvError::Empty) => break,
                            Err(std_mpsc::TryRecvError::Disconnected) => break,
                        }
                    }

                    if !batch.is_empty() {
                        let batch = std::mem::take(&mut batch);
                        let batch_size = batch.len();
                        let (result_sender, result_receiver) = oneshot::channel();
                        let job: DbJob = Box::new(move |connection| {
                            let started_at = Instant::now();
                            let result = (|| -> Result<(), StoreError> {
                                let transaction = connection.transaction()?;
                                for stat in batch {
                                    transaction.execute(
                                        "INSERT INTO ccproxy_stats (client_model, backend_model, provider_id, provider, protocol, tool_compat_mode, status_code, error_message, input_tokens, output_tokens, cache_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                                        rusqlite::params![
                                            stat.client_model,
                                            stat.backend_model,
                                            stat.provider_id,
                                            stat.provider,
                                            stat.protocol,
                                            stat.tool_compat_mode,
                                            stat.status_code,
                                            stat.error_message,
                                            stat.input_tokens,
                                            stat.output_tokens,
                                            stat.cache_tokens,
                                        ],
                                    )?;
                                }
                                transaction.commit()?;
                                Ok(())
                            })();
                            if started_at.elapsed() >= SLOW_JOB_WARNING {
                                log::warn!(
                                    "CCProxy telemetry batch SQL job took {} ms for {} records",
                                    started_at.elapsed().as_millis(),
                                    batch_size
                                );
                            }
                            let _ = result_sender.send(result);
                        });
                        let result = writer_sender
                            .blocking_send(WorkerMessage::Job(job))
                            .map_err(|_| StoreError::RuntimeClosed)
                            .and_then(|_| {
                                result_receiver
                                    .blocking_recv()
                                    .map_err(|_| StoreError::RuntimeClosed)?
                            });
                        worker_pending.fetch_sub(batch_size, Ordering::Relaxed);
                        match result {
                            Ok(()) => {
                                metrics.telemetry_batches_completed.fetch_add(1, Ordering::Relaxed);
                                metrics.telemetry_records_completed.fetch_add(batch_size as u64, Ordering::Relaxed);
                                log::debug!("Persisted CCProxy telemetry batch with {batch_size} records");
                            }
                            Err(error) => {
                                metrics.failed_jobs.fetch_add(1, Ordering::Relaxed);
                                log::error!("Failed to persist CCProxy telemetry batch: {error}");
                            }
                        }
                    }

                    if let Some(ack_sender) = flush_ack.take() {
                        let _ = ack_sender.send(());
                    }
                    if let Some(ack_sender) = shutdown_ack.take() {
                        let _ = ack_sender.send(());
                        break;
                    }
                }
            })
            .map_err(|error| StoreError::WorkerFailed(error.to_string()))?;

        Ok(TelemetryIngress {
            sender,
            pending,
            join_handle: Mutex::new(Some(join_handle)),
        })
    }

    pub async fn write<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        self.execute(&self.writer, operation, true).await
    }

    pub async fn read<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let worker = if self.is_memory {
            &self.writer
        } else {
            let index = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
            &self.readers[index]
        };
        self.execute(worker, operation, self.is_memory).await
    }

    async fn execute<T, F>(
        &self,
        worker: &Worker,
        operation: F,
        is_writer: bool,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        if !self.accepting_jobs.load(Ordering::Acquire) {
            return Err(StoreError::RuntimeMaintenance);
        }
        let (result_sender, result_receiver) = oneshot::channel();
        let metrics = Arc::clone(&self.metrics);
        let job: DbJob = Box::new(move |connection| {
            let started_at = Instant::now();
            let result = operation(connection);
            if started_at.elapsed() >= SLOW_JOB_WARNING {
                let worker_kind = if is_writer { "writer" } else { "reader" };
                log::warn!(
                    "Database {worker_kind} job took {} ms",
                    started_at.elapsed().as_millis()
                );
            }
            if result.is_err() {
                metrics.failed_jobs.fetch_add(1, Ordering::Relaxed);
            }
            let _ = result_sender.send(result);
        });

        let enqueue_started_at = Instant::now();
        worker
            .sender
            .send(WorkerMessage::Job(job))
            .await
            .map_err(|_| StoreError::RuntimeClosed)?;
        if enqueue_started_at.elapsed() >= SLOW_ENQUEUE_WARNING {
            let worker_kind = if is_writer { "writer" } else { "reader" };
            log::warn!(
                "Database {worker_kind} queue enqueue waited {} ms",
                enqueue_started_at.elapsed().as_millis()
            );
        }
        self.observe_queue_depth(worker, is_writer);
        result_receiver
            .await
            .map_err(|_| StoreError::RuntimeClosed)?
    }

    fn execute_blocking<T, F>(
        &self,
        worker: &Worker,
        operation: F,
        is_writer: bool,
        require_running: bool,
    ) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        if require_running && !self.accepting_jobs.load(Ordering::Acquire) {
            return Err(StoreError::RuntimeMaintenance);
        }
        let (result_sender, result_receiver) = std_mpsc::sync_channel(1);
        let metrics = Arc::clone(&self.metrics);
        let job: DbJob = Box::new(move |connection| {
            let result = operation(connection);
            if result.is_err() {
                metrics.failed_jobs.fetch_add(1, Ordering::Relaxed);
            }
            let _ = result_sender.send(result);
        });

        let sender = worker.sender.clone();
        let (dispatch_sender, dispatch_receiver) = std_mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = sender
                .blocking_send(WorkerMessage::Job(job))
                .map_err(|_| StoreError::RuntimeClosed);
            let _ = dispatch_sender.send(result);
        });
        dispatch_receiver
            .recv()
            .map_err(|_| StoreError::RuntimeClosed)??;
        self.observe_queue_depth(worker, is_writer);
        result_receiver
            .recv()
            .map_err(|_| StoreError::RuntimeClosed)?
    }

    fn observe_queue_depth(&self, worker: &Worker, is_writer: bool) {
        let queue_depth = worker.sender.max_capacity() - worker.sender.capacity();
        if is_writer {
            DbRuntimeMetrics::observe_high_water(&self.metrics.write_queue_high_water, queue_depth);
        } else {
            DbRuntimeMetrics::observe_high_water(&self.metrics.read_queue_high_water, queue_depth);
        }
    }

    /// Runs a write job on the dedicated writer and blocks only the caller, never the worker pool.
    pub fn write_blocking<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        self.execute_blocking(&self.writer, operation, true, true)
    }

    /// Runs a read job on a dedicated reader and blocks only the caller, never the worker pool.
    pub fn read_blocking<T, F>(&self, operation: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StoreError> + Send + 'static,
    {
        let worker = if self.is_memory {
            &self.writer
        } else {
            let index = self.next_reader.fetch_add(1, Ordering::Relaxed) % self.readers.len();
            &self.readers[index]
        };
        self.execute_blocking(worker, operation, self.is_memory, true)
    }

    /// Runs an exclusive checkpoint after the caller has entered maintenance mode.
    pub fn checkpoint_for_maintenance(&self) -> Result<(), StoreError> {
        self.execute_blocking(
            &self.writer,
            |connection| {
                connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
                Ok(())
            },
            true,
            false,
        )
    }

    pub fn drain_for_maintenance(&self) -> Result<(), StoreError> {
        self.accepting_jobs.store(false, Ordering::Release);
        std::thread::scope(|scope| {
            scope.spawn(|| self.drain_blocking()).join().map_err(|_| {
                StoreError::WorkerFailed("maintenance drain thread panicked".to_string())
            })?
        })
    }

    pub fn resume_after_maintenance(&self) {
        self.accepting_jobs.store(true, Ordering::Release);
    }

    pub fn shutdown_for_maintenance(&self) -> Result<(), StoreError> {
        std::thread::scope(|scope| {
            scope
                .spawn(|| self.shutdown_blocking())
                .join()
                .map_err(|_| {
                    StoreError::WorkerFailed("maintenance shutdown thread panicked".to_string())
                })?
        })
    }

    pub fn drain_blocking(&self) -> Result<(), StoreError> {
        self.telemetry.flush_blocking()?;
        self.writer.barrier_blocking()?;
        for reader in &self.readers {
            reader.barrier_blocking()?;
        }
        Ok(())
    }

    pub fn shutdown_blocking(&self) -> Result<(), StoreError> {
        self.telemetry.flush_blocking()?;
        self.telemetry.shutdown_blocking()?;
        for reader in &self.readers {
            reader.shutdown_blocking()?;
        }
        self.writer.shutdown_blocking()
    }

    pub fn enqueue_ccproxy_stat(&self, stat: CcproxyStat) -> Result<(), StoreError> {
        if !self.accepting_jobs.load(Ordering::Acquire) {
            return Err(StoreError::RuntimeMaintenance);
        }
        self.telemetry.enqueue(stat)?;
        DbRuntimeMetrics::observe_high_water(
            &self.metrics.telemetry_pending_high_water,
            self.telemetry.pending.load(Ordering::Relaxed),
        );
        Ok(())
    }

    #[cfg(test)]
    pub fn metrics(&self) -> DbRuntimeMetricsSnapshot {
        self.metrics.snapshot()
    }

    #[cfg(test)]
    pub async fn shutdown(&self) -> Result<(), StoreError> {
        self.telemetry.shutdown().await?;
        for reader in &self.readers {
            reader.shutdown().await?;
        }
        self.writer.shutdown().await
    }
}

fn open_writer_connection(db_path: &PathBuf) -> Result<Connection, StoreError> {
    let connection = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    configure_writer_connection(&connection)?;
    Ok(connection)
}

fn open_reader_connection(db_path: &PathBuf) -> Result<Connection, StoreError> {
    let connection = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    connection.execute("PRAGMA foreign_keys=ON", [])?;
    connection.execute("PRAGMA query_only=ON", [])?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(connection)
}

fn configure_writer_connection(connection: &Connection) -> Result<(), StoreError> {
    let _ = connection.query_row("PRAGMA journal_mode=WAL", [], |_| Ok(()));
    connection.execute("PRAGMA synchronous=NORMAL", [])?;
    connection.execute("PRAGMA foreign_keys=ON", [])?;
    connection.busy_timeout(Duration::from_secs(5))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::DbRuntime;
    use crate::db::{CcproxyStat, StoreError};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    #[tokio::test]
    async fn writer_ack_returns_committed_row_and_reader_observes_it() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime = DbRuntime::open(temp_dir.path().join("runtime.db")).unwrap();
        runtime
            .write(|connection| {
                connection.execute("CREATE TABLE records (value TEXT NOT NULL)", [])?;
                connection.execute("INSERT INTO records (value) VALUES ('saved')", [])?;
                Ok(connection.last_insert_rowid())
            })
            .await
            .unwrap();
        let count = runtime
            .read(|connection| {
                Ok(
                    connection.query_row("SELECT COUNT(*) FROM records", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn file_backed_readers_run_on_distinct_workers() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime = Arc::new(DbRuntime::open(temp_dir.path().join("runtime.db")).unwrap());
        runtime
            .write(|connection| {
                connection.execute("CREATE TABLE records (value TEXT NOT NULL)", [])?;
                Ok(())
            })
            .await
            .unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let left_runtime = Arc::clone(&runtime);
        let left_barrier = Arc::clone(&barrier);
        let left = tokio::spawn(async move {
            left_runtime
                .read(move |_| {
                    left_barrier.wait();
                    Ok(std::thread::current()
                        .name()
                        .unwrap_or_default()
                        .to_string())
                })
                .await
        });
        let right_runtime = Arc::clone(&runtime);
        let right_barrier = Arc::clone(&barrier);
        let right = tokio::spawn(async move {
            right_runtime
                .read(move |_| {
                    right_barrier.wait();
                    Ok(std::thread::current()
                        .name()
                        .unwrap_or_default()
                        .to_string())
                })
                .await
        });

        let (left_name, right_name) = tokio::time::timeout(Duration::from_secs(2), async {
            (left.await.unwrap().unwrap(), right.await.unwrap().unwrap())
        })
        .await
        .unwrap();
        assert_ne!(left_name, right_name);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn blocking_facade_dispatches_sql_to_the_writer_from_tokio() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime = DbRuntime::open(temp_dir.path().join("runtime.db")).unwrap();
        let worker_name = runtime
            .write_blocking(|connection| {
                connection.execute("CREATE TABLE records (value TEXT NOT NULL)", [])?;
                Ok(std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string())
            })
            .unwrap();
        assert_eq!(worker_name, "db-writer");
        let count = runtime.read_blocking(|connection| {
            connection.execute("INSERT INTO records (value) VALUES ('saved')", [])?;
            Ok(
                connection.query_row("SELECT COUNT(*) FROM records", [], |row| {
                    row.get::<_, i64>(0)
                })?,
            )
        });
        assert!(count.is_err(), "reader connections must remain query-only");
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn memory_database_reads_share_the_writer_connection() {
        let runtime = DbRuntime::open(":memory:").unwrap();
        runtime
            .write(|connection| {
                connection.execute("CREATE TABLE records (value TEXT NOT NULL)", [])?;
                connection.execute("INSERT INTO records (value) VALUES ('saved')", [])?;
                Ok(())
            })
            .await
            .unwrap();
        let count = runtime
            .read(|connection| {
                Ok(
                    connection.query_row("SELECT COUNT(*) FROM records", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn telemetry_batches_commit_before_shutdown() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime = DbRuntime::open(temp_dir.path().join("runtime.db")).unwrap();
        runtime
            .write(|connection| {
                connection.execute(
                    "CREATE TABLE ccproxy_stats (
                        client_model TEXT NOT NULL,
                        backend_model TEXT NOT NULL,
                        provider_id INTEGER,
                        provider TEXT NOT NULL,
                        protocol TEXT NOT NULL,
                        tool_compat_mode INTEGER NOT NULL,
                        status_code INTEGER NOT NULL,
                        error_message TEXT,
                        input_tokens INTEGER NOT NULL,
                        output_tokens INTEGER NOT NULL,
                        cache_tokens INTEGER NOT NULL
                    )",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        for index in 0..3 {
            runtime
                .enqueue_ccproxy_stat(CcproxyStat {
                    id: None,
                    client_model: "client".to_string(),
                    backend_model: "backend".to_string(),
                    provider_id: Some(1),
                    provider: "provider".to_string(),
                    protocol: "openai".to_string(),
                    tool_compat_mode: 0,
                    status_code: 200,
                    error_message: None,
                    input_tokens: index,
                    output_tokens: index,
                    cache_tokens: 0,
                    request_at: None,
                })
                .unwrap();
        }
        runtime.drain_for_maintenance().unwrap();
        assert!(matches!(
            runtime.read(|_| Ok(())).await,
            Err(StoreError::RuntimeMaintenance)
        ));
        assert!(matches!(
            runtime.enqueue_ccproxy_stat(CcproxyStat {
                id: None,
                client_model: "client".to_string(),
                backend_model: "backend".to_string(),
                provider_id: Some(1),
                provider: "provider".to_string(),
                protocol: "openai".to_string(),
                tool_compat_mode: 0,
                status_code: 200,
                error_message: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_tokens: 0,
                request_at: None,
            }),
            Err(StoreError::RuntimeMaintenance)
        ));
        let metrics = runtime.metrics();
        assert_eq!(metrics.telemetry_records_completed, 3);
        assert!(metrics.telemetry_batches_completed >= 1);
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn maintenance_drain_waits_for_in_flight_reader_jobs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime = Arc::new(DbRuntime::open(temp_dir.path().join("runtime.db")).unwrap());
        let (started_sender, started_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let reader_runtime = Arc::clone(&runtime);
        let reader = tokio::spawn(async move {
            reader_runtime
                .read(move |_| {
                    started_sender.send(()).map_err(|error| {
                        StoreError::WorkerFailed(format!("reader start signal failed: {error}"))
                    })?;
                    release_receiver.recv().map_err(|error| {
                        StoreError::WorkerFailed(format!("reader release signal failed: {error}"))
                    })?;
                    Ok(())
                })
                .await
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("reader job should start");

        let drain_runtime = Arc::clone(&runtime);
        let drain = tokio::task::spawn_blocking(move || drain_runtime.drain_for_maintenance());
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !drain.is_finished(),
            "maintenance must wait for the active reader before checkpointing"
        );

        release_sender
            .send(())
            .expect("reader release receiver should remain available");
        reader.await.unwrap().unwrap();
        drain.await.unwrap().unwrap();
        runtime.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_rejects_later_jobs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime = DbRuntime::open(temp_dir.path().join("runtime.db")).unwrap();
        runtime.shutdown().await.unwrap();
        assert!(runtime.write(|_| Ok(())).await.is_err());
    }
}
