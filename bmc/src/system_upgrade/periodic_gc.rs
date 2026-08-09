// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bmc_scheduler::Cron;
use bmc_scheduler::JobScheduler;
use bmc_scheduler::scheduler::{AsyncTask, JobConfig, Schedule, Task};
use bmc_upgrade::packages::{PackageBackend, PackageGcOutcome, PackageGcRequest};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::stagger::{HourParity, MAINTENANCE_MIN_DELAY, MaintenanceStagger};

/// Scheduler source name, so the job can be listed and cancelled like any
/// other.
pub(crate) const PERIODIC_GC_SOURCE: &str = "periodic-gc";

/// The periodic collection job.
#[derive(Debug)]
pub(crate) struct PeriodicGc {
    /// Process start, measured monotonically. A clock correction moves the
    /// wall-clock grid the pattern was derived against but cannot move this,
    /// which is the point: it keeps collection out of the boot window when
    /// the grid shifts under us.
    started: Instant,
    run_gate: Arc<Mutex<()>>,
    package_backend: Arc<dyn PackageBackend>,
    /// Read at every occurrence, so the developer opt-out in it takes effect
    /// without a restart. The opt-out is this job's decision alone: forced
    /// collection before an upgrade and `bmc-nix-cli gc` never consult it.
    gc_config_path: PathBuf,
    /// Set when a run removed generations without completing a sweep. Those
    /// generations are gone, so no later cleanup counts them again and only
    /// an unconditional sweep can reclaim what they rooted. In memory only:
    /// a restart in between leaks until the next forced collection.
    escalate_sweep: AtomicBool,
}

impl PeriodicGc {
    /// `started` is captured by the caller at process start, not here:
    /// registration happens well into the startup sequence, and the floor is
    /// about the boot window, not about when this job happened to be built.
    pub(crate) fn new(
        started: Instant,
        run_gate: Arc<Mutex<()>>,
        package_backend: Arc<dyn PackageBackend>,
        gc_config_path: PathBuf,
    ) -> Self {
        Self {
            started,
            run_gate,
            package_backend,
            gc_config_path,
            escalate_sweep: AtomicBool::new(false),
        }
    }

    /// Register the job on the stagger's even-hour grid.
    pub(crate) async fn schedule(
        self: &Arc<Self>,
        scheduler: &JobScheduler,
        stagger: MaintenanceStagger,
    ) -> anyhow::Result<()> {
        let pattern = stagger.pattern(HourParity::Even);
        let cron = <Cron as std::str::FromStr>::from_str(&pattern)?;

        info!(pattern, "Scheduling periodic garbage collection");

        let gc = Arc::clone(self);
        let task: AsyncTask = Box::new(move || {
            let gc = Arc::clone(&gc);
            Box::pin(async move { gc.run().await })
        });

        // Not persisted to the crontab: the schedule is re-derived from each
        // boot's time and nothing about it should outlive the process.
        scheduler
            .schedule(
                Schedule::Cron(cron),
                Task::Async(task),
                JobConfig::new(PERIODIC_GC_SOURCE),
            )
            .await?;

        Ok(())
    }

    /// One occurrence.
    pub(crate) async fn run(&self) {
        if self.started.elapsed() < MAINTENANCE_MIN_DELAY {
            debug!("Skipping periodic garbage collection inside the startup window");
            return;
        }

        // A configuration that fails to load is not a reason to skip: the
        // backend reads the same file for its retention policy and surfaces
        // the failure below.
        if let Ok(config) = bmc_nix::gc::load_gc_config(&self.gc_config_path)
            && matches!(config.periodic, bmc_nix::types::PeriodicGcMode::Disabled)
        {
            debug!("Periodic garbage collection is disabled by configuration");
            return;
        }

        let Ok(_gate) = self.run_gate.try_lock() else {
            debug!("Skipping periodic garbage collection while an upgrade operation is active");
            return;
        };

        let sweep = if self.escalate_sweep.load(Ordering::Relaxed) {
            bmc_nix::gc::Sweep::Always
        } else {
            bmc_nix::gc::Sweep::WhenGenerationsRemoved
        };
        let request = PackageGcRequest {
            on_busy: bmc_nix::gc::OnBusy::Skip,
            sweep,
        };

        match self.package_backend.gc(request).await {
            Ok(PackageGcOutcome::Collected) => {
                self.escalate_sweep.store(false, Ordering::Relaxed);
                info!("Periodic garbage collection finished");
            }
            Ok(PackageGcOutcome::SweptDespiteCleanupFailure) => {
                // The completed sweep reclaimed everything this run unrooted;
                // the entries that resisted removal wait for the next run.
                self.escalate_sweep.store(false, Ordering::Relaxed);
                warn!("Periodic garbage collection swept the store, but generation cleanup failed");
            }
            Ok(PackageGcOutcome::NothingToCollect) => {
                debug!("Nothing was unrooted; skipped the store sweep");
            }
            Ok(PackageGcOutcome::Busy) => {
                debug!("Skipping periodic garbage collection while the package profile is busy");
            }
            Err(err) => {
                let removed = err.unswept_removals();
                if removed > 0 {
                    self.escalate_sweep.store(true, Ordering::Relaxed);
                }
                warn!(error = %err, removed, "Periodic garbage collection failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::AtomicUsize;

    use bmc_upgrade::packages::PackageGcError;

    /// Backend recording every request and answering from a scripted queue.
    #[derive(Debug, Default)]
    struct ScriptedBackend {
        requests: StdMutex<Vec<PackageGcRequest>>,
        results: StdMutex<Vec<Result<PackageGcOutcome, PackageGcError>>>,
        calls: AtomicUsize,
    }

    impl ScriptedBackend {
        fn with(results: Vec<Result<PackageGcOutcome, PackageGcError>>) -> Arc<Self> {
            Arc::new(Self {
                results: StdMutex::new(results),
                ..Self::default()
            })
        }

        fn requests(&self) -> Vec<PackageGcRequest> {
            self.requests
                .lock()
                .expect("BUG: poisoned request log")
                .clone()
        }

        fn sweeps(&self) -> Vec<bmc_nix::gc::Sweep> {
            self.requests()
                .iter()
                .map(|request| request.sweep)
                .collect()
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl PackageBackend for ScriptedBackend {
        async fn gc(&self, request: PackageGcRequest) -> Result<PackageGcOutcome, PackageGcError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .expect("BUG: poisoned request log")
                .push(request);
            let mut results = self.results.lock().expect("BUG: poisoned result queue");
            if results.is_empty() {
                Ok(PackageGcOutcome::Collected)
            } else {
                results.remove(0)
            }
        }

        async fn probe(
            &self,
            _estimate: bmc_upgrade::packages::EstimateMode,
            _install: &[String],
        ) -> bmc_upgrade::packages::PackageProbe {
            bmc_upgrade::packages::PackageProbe::UpToDate
        }

        async fn apply(
            &self,
            _merged: bmc_nix::types::MergedIndex,
            _install: Vec<String>,
            _progress: Arc<dyn bmc_nix::upgrade::UpgradeProgress>,
        ) -> Result<(), bmc_upgrade::packages::ApplyError> {
            unreachable!("BUG: the collection job never applies an upgrade")
        }

        fn store_free_bytes(&self) -> std::io::Result<u64> {
            Ok(u64::MAX)
        }

        async fn list_installable_widgets(
            &self,
        ) -> Result<
            Vec<bmc_upgrade::packages::InstallableWidget>,
            bmc_upgrade::packages::PackageProbeError,
        > {
            Ok(Vec::new())
        }
    }

    /// A `PeriodicGc` whose startup floor has already elapsed.
    ///
    /// The floor is crossed by advancing tokio's clock, not by subtracting from
    /// `Instant::now()` — an `Instant` cannot be moved back past process start,
    /// which on a freshly booted machine is less than the floor ago. Every test
    /// using this helper must therefore be `#[tokio::test(start_paused = true)]`.
    async fn ready(
        backend: Arc<dyn PackageBackend>,
    ) -> (PeriodicGc, Arc<Mutex<()>>, tempfile::TempDir) {
        let config_dir = tempfile::tempdir().expect("BUG: temp dir");
        let run_gate = Arc::new(Mutex::new(()));
        let gc = PeriodicGc::new(
            Instant::now(),
            Arc::clone(&run_gate),
            backend,
            config_dir.path().join("gc.json"),
        );
        tokio::time::advance(MAINTENANCE_MIN_DELAY).await;
        (gc, run_gate, config_dir)
    }

    #[tokio::test]
    async fn a_run_inside_the_startup_window_does_not_reach_the_backend() {
        let backend = ScriptedBackend::with(Vec::new());
        let config_dir = tempfile::tempdir().expect("BUG: temp dir");
        let gc = PeriodicGc::new(
            Instant::now(),
            Arc::new(Mutex::new(())),
            Arc::clone(&backend) as Arc<dyn PackageBackend>,
            config_dir.path().join("gc.json"),
        );

        gc.run().await;

        assert_eq!(
            backend.calls(),
            0,
            "the monotonic floor holds even when a clock correction moved the grid"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_ready_run_asks_for_a_conditional_sweep() {
        let backend = ScriptedBackend::with(Vec::new());
        let (gc, _run_gate, _config_dir) =
            ready(Arc::clone(&backend) as Arc<dyn PackageBackend>).await;

        gc.run().await;

        let requests = backend.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].on_busy, bmc_nix::gc::OnBusy::Skip);
        assert_eq!(
            requests[0].sweep,
            bmc_nix::gc::Sweep::WhenGenerationsRemoved
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_busy_run_gate_skips_the_occurrence() {
        let backend = ScriptedBackend::with(Vec::new());
        let (gc, run_gate, _config_dir) =
            ready(Arc::clone(&backend) as Arc<dyn PackageBackend>).await;
        let held = Arc::clone(&run_gate).lock_owned().await;

        gc.run().await;

        assert_eq!(
            backend.calls(),
            0,
            "collection must never run alongside an upgrade"
        );
        drop(held);
    }

    #[tokio::test(start_paused = true)]
    async fn removals_left_unswept_escalate_the_next_occurrence() {
        let backend = ScriptedBackend::with(vec![
            Err(PackageGcError::UnsweptRemovals {
                removed: 2,
                message: "sweep failed".to_owned(),
            }),
            Ok(PackageGcOutcome::Collected),
            Ok(PackageGcOutcome::Collected),
        ]);
        let (gc, _run_gate, _config_dir) =
            ready(Arc::clone(&backend) as Arc<dyn PackageBackend>).await;

        gc.run().await; // fails after removing two entries
        gc.run().await; // must sweep unconditionally to reclaim them
        gc.run().await; // escalation cleared by the successful sweep

        assert_eq!(
            backend.sweeps(),
            vec![
                bmc_nix::gc::Sweep::WhenGenerationsRemoved,
                bmc_nix::gc::Sweep::Always,
                bmc_nix::gc::Sweep::WhenGenerationsRemoved,
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_operational_failure_without_removals_does_not_escalate() {
        let backend = ScriptedBackend::with(vec![Err(PackageGcError::Operational(
            "failed to lock the profile".to_owned(),
        ))]);
        let (gc, _run_gate, _config_dir) =
            ready(Arc::clone(&backend) as Arc<dyn PackageBackend>).await;

        gc.run().await;
        gc.run().await;

        assert_eq!(
            backend.sweeps(),
            vec![
                bmc_nix::gc::Sweep::WhenGenerationsRemoved,
                bmc_nix::gc::Sweep::WhenGenerationsRemoved,
            ],
            "nothing was removed, so nothing is owed a sweep"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_disabled_configuration_skips_the_backend_but_leaves_the_job_running() {
        let backend = ScriptedBackend::with(vec![Ok(PackageGcOutcome::Collected)]);
        let (gc, _run_gate, config_dir) =
            ready(Arc::clone(&backend) as Arc<dyn PackageBackend>).await;
        let config_path = config_dir.path().join("gc.json");
        std::fs::write(&config_path, r#"{"periodic":"disabled"}"#).expect("BUG: write gc config");

        gc.run().await;
        assert_eq!(
            backend.calls(),
            0,
            "a disabled occurrence must not reach the backend"
        );

        std::fs::write(&config_path, r#"{"periodic":"enabled"}"#).expect("BUG: write gc config");
        gc.run().await;
        assert_eq!(
            backend.calls(),
            1,
            "re-enabling the toggle must take effect without a restart"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_unloadable_configuration_still_reaches_the_backend() {
        let backend = ScriptedBackend::with(vec![Ok(PackageGcOutcome::Collected)]);
        let (gc, _run_gate, config_dir) =
            ready(Arc::clone(&backend) as Arc<dyn PackageBackend>).await;
        std::fs::write(config_dir.path().join("gc.json"), "not json")
            .expect("BUG: write gc config");

        gc.run().await;

        assert_eq!(
            backend.calls(),
            1,
            "a corrupt config must fail loudly in the backend, not silence collection"
        );
    }
}
