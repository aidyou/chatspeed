//! Wiring the Phase 2I promotion supervisor into the headless runtime.
//!
//! This module is the only place that connects the promotion pieces:
//!
//! - the durable promotion store ([`ExperimentPromotionStore`]),
//! - the server-owned target registry and the execution-profile registry,
//! - the `PromotionCheckpointOwner` (the only Git-ref mutator) and the paired
//!   canary runner, and
//! - the durable schedule store, which is what a promotion's evidence is bound
//!   to.
//!
//! It deliberately adds no second lifecycle: the supervisor is ticked from the
//! same bounded, shutdown-aware loop shape the 2H scheduler uses, and every
//! server-side resource (base repository, domain directories, registries) is
//! resolved from the instance's own configuration (INV-1/INV-2).

use crate::db::experiment_promotion::ExperimentPromotionStore;
use crate::db::experiment_schedule::ExperimentScheduleStore;
use crate::workflow::react::experiment_promotion::scheduler::{
    PromotionSupervisor, PromotionSupervisorConfig, PromotionTickOutcome,
};
use crate::workflow::react::experiment_promotion::types::PromotionError;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// The shortest cadence the promotion supervisor may tick at.
pub const MIN_PROMOTION_POLL_MS: u64 = 250;

/// The default cadence. A promotion attempt performs container work, so it is
/// deliberately slower than the job scheduler's cadence.
pub const DEFAULT_PROMOTION_POLL_MS: u64 = 1_000;

/// The promotion lease length. A tick that performs a canary may take minutes,
/// so the lease is renewed implicitly by the next claim: the supervisor's own
/// tick re-claims only after this expires, which is exactly the recovery window.
pub const PROMOTION_LEASE_MS: u64 = 15 * 60 * 1_000;

/// The effective lease length. `CHATSPEED_PROMOTION_LEASE_MS` may override the
/// default so an operator (or a recovery test) can shorten the window a dead
/// instance holds; every other value falls back to the default.
pub fn promotion_lease_ms() -> u64 {
    std::env::var("CHATSPEED_PROMOTION_LEASE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 250)
        .unwrap_or(PROMOTION_LEASE_MS)
}

/// Builds the supervisor for one experiment domain.
pub fn server_promotion_supervisor(
    domain_root: impl Into<PathBuf>,
    base_repo: Option<PathBuf>,
    store: Arc<crate::db::MainStore>,
    owner_id: impl Into<String>,
) -> PromotionSupervisor {
    PromotionSupervisor::new(
        ExperimentPromotionStore::new(store.clone()),
        ExperimentScheduleStore::new(store),
        PromotionSupervisorConfig {
            domain_root: domain_root.into(),
            base_repo,
            owner_id: owner_id.into(),
            lease_ms: promotion_lease_ms(),
        },
    )
}

/// A spawned promotion supervisor loop.
pub struct PromotionSupervisorHandle {
    pub shutdown: tokio::sync::watch::Sender<bool>,
    pub task: tokio::task::JoinHandle<()>,
}

impl PromotionSupervisorHandle {
    /// Stops the loop. Idempotent: a second stop is a no-op.
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
        self.task.abort();
    }
}

/// Spawns the promotion supervisor loop for one domain.
pub fn spawn_promotion_supervisor(
    domain_root: PathBuf,
    base_repo: Option<PathBuf>,
    store: Arc<crate::db::MainStore>,
    owner_id: impl Into<String>,
    poll_ms: u64,
) -> PromotionSupervisorHandle {
    let supervisor = Arc::new(server_promotion_supervisor(
        domain_root,
        base_repo,
        store,
        owner_id,
    ));
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let tick = Arc::new(move || supervisor.tick(crate::headless::domain::now_ms()));
    let task = tokio::spawn(run_promotion_loop(
        receiver,
        poll_ms.max(MIN_PROMOTION_POLL_MS),
        tick,
    ));
    PromotionSupervisorHandle { shutdown, task }
}

/// Drives one supervisor tick per interval until shutdown.
///
/// The cadence is enforced *after* each tick, so an idle instance stays idle
/// instead of spinning as fast as empty ticks complete, and a shutdown signal
/// interrupts either phase. A tick never leaves a promotion half-processed: the
/// durable intent is what the next tick re-observes.
async fn run_promotion_loop<F>(
    mut receiver: tokio::sync::watch::Receiver<bool>,
    poll_ms: u64,
    tick: Arc<F>,
) where
    F: Fn() -> Result<PromotionTickOutcome, PromotionError> + Send + Sync + 'static,
{
    loop {
        if *receiver.borrow() {
            break;
        }
        let tick_fn = tick.clone();
        let running = tokio::task::spawn_blocking(move || tick_fn());
        tokio::select! {
            result = running => match result {
                Ok(Ok(PromotionTickOutcome::Idle)) => {}
                Ok(Ok(outcome)) => log::info!("[Headless][promotion] tick: {outcome:?}"),
                Ok(Err(error)) => log::warn!(
                    "[Headless][promotion] tick failed ({}): {}",
                    error.code.as_str(),
                    error.message
                ),
                Err(error) => log::warn!("[Headless][promotion] tick task failed: {error}"),
            },
            changed = receiver.changed() => {
                if changed.is_err() || *receiver.borrow() {
                    break;
                }
                continue;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(poll_ms)) => {}
            changed = receiver.changed() => {
                if changed.is_err() || *receiver.borrow() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::react::experiment_promotion::scheduler::PromotionTickOutcome;
    use tempfile::tempdir;

    #[test]
    fn a_domain_without_work_is_supervised_idly() {
        let directory = tempdir().expect("tempdir");
        let store =
            Arc::new(crate::db::MainStore::new(directory.path().join("p.db")).expect("store"));
        let supervisor =
            server_promotion_supervisor(directory.path(), None, store, "promotion-supervisor");
        assert_eq!(
            supervisor.tick(1_700_000_000_000).expect("tick"),
            PromotionTickOutcome::Idle
        );
    }

    #[tokio::test]
    async fn the_loop_spawns_and_stops_without_work() {
        let directory = tempdir().expect("tempdir");
        let store =
            Arc::new(crate::db::MainStore::new(directory.path().join("p.db")).expect("store"));
        let handle = spawn_promotion_supervisor(
            directory.path().to_path_buf(),
            None,
            store,
            "promotion-supervisor",
            MIN_PROMOTION_POLL_MS,
        );
        tokio::time::sleep(Duration::from_millis(120)).await;
        handle.stop();
        handle.stop();
    }
}
