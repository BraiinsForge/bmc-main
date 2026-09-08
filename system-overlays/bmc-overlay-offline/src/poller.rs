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

//! The thread that polls boser, and the handoff the overlay's tick reads.
//!
//! Same shape as the framework's connectivity prober. The thread folds each
//! poll into the retry budget where it lands, so no failure can be skipped,
//! and publishes only the status to show, versioned on change.

use std::any::Any;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::bos::{BosClient, Poll, PollFailure};
use crate::mining::{FAILURES_BEFORE_LOW, Status, StatusTracker, Transition, derive};

/// Pause between polls; the miner-info widget reads the same API at this cadence.
pub const POLL_PERIOD: Duration = Duration::from_secs(5);

/// Room for ureq, rustls and serde frames on one thread, where the
/// connectivity prober gets by on 128 KiB for a getifaddrs walk. Still far
/// under the 2 MiB Rust default, which wastes address space on 32-bit ARM.
const STACK_SIZE: usize = 512 * 1024;

/// Change marker of a published status; handed back as `seen` on the next read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusVersion(NonZeroU64);

/// Fixed points for test doubles faking a run of status changes.
#[cfg(test)]
impl StatusVersion {
    pub const FIRST: Self = Self(NonZeroU64::MIN);

    #[must_use]
    pub fn next(self) -> Self {
        Self(
            self.0
                .checked_add(1)
                .expect("BUG: StatusVersion overflowed u64"),
        )
    }
}

/// The published status and its version. The version is 0 until the first
/// publish and bumps only when the status differs from the one stored,
/// always while the mutex is held, so a (version, status) pair read under
/// the lock is consistent; the lock-free load is only a cheap "anything new?" gate.
#[derive(Default)]
struct Shared {
    version: AtomicU64,
    status: Mutex<Option<Status>>,
    /// Set when the [`Poller`] is dropped; the thread checks it once per period.
    stop: AtomicBool,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("version", &self.version.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Shared {
    fn publish(&self, status: Status) {
        let mut guard = self.lock();
        if *guard != Some(status) {
            *guard = Some(status);
            self.version.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn status_if_changed(&self, seen: Option<StatusVersion>) -> Option<(StatusVersion, Status)> {
        let seen = seen.map_or(0, |version| version.0.get());
        if self.version.load(Ordering::Relaxed) == seen {
            return None;
        }
        let guard = self.lock();
        let version = NonZeroU64::new(self.version.load(Ordering::Relaxed)).map(StatusVersion)?;
        guard.map(|status| (version, status))
    }

    fn lock(&self) -> MutexGuard<'_, Option<Status>> {
        // A panic can only poison a plain value swap or read, so the inner
        // value is always intact; recover it instead of propagating.
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Fold one poll into the budget and publish what there is to show.
fn absorb(tracker: &mut StatusTracker, shared: &Shared, poll: &Poll) {
    if let Err(failure) = poll {
        tracing::debug!(%failure, "mining status poll failed");
    }
    match tracker.observe(poll.as_ref().ok().map(derive)) {
        Transition::Exhausted => {
            if let Err(failure) = poll {
                tracing::warn!(
                    %failure,
                    "no mining status for {FAILURES_BEFORE_LOW} consecutive polls; the pickaxe turns red"
                );
            }
        }
        Transition::Recovered => tracing::info!("mining status polls recovered"),
        Transition::Unchanged => {}
    }
    if let Some(status) = tracker.shown() {
        shared.publish(status);
    }
}

/// What a caught panic said; `panic!` leaves a `&str` or a `String` behind.
fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

/// Poll `client` on a `period` grid until the stop flag is raised. The wait
/// discounts the poll's own duration, so three timed-out requests do not
/// stretch the retry budget.
fn run(mut client: BosClient, shared: &Shared, period: Duration) {
    let mut tracker = StatusTracker::default();
    while !shared.stop.load(Ordering::Relaxed) {
        let started = Instant::now();
        // AssertUnwindSafe: a panic mid-poll leaves at most a stale token in
        // the client. The next poll's 401 clears it and the one after logs in.
        let poll = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.poll()))
            .unwrap_or_else(|payload| {
                let message = panic_message(&*payload);
                tracing::error!(%message, "mining status poll panicked");
                Err(PollFailure::Panicked(message))
            });
        absorb(&mut tracker, shared, &poll);
        std::thread::sleep(period.saturating_sub(started.elapsed()));
    }
}

/// The overlay's handle on the polling thread.
/// Dropping it stops the thread within one poll period, and does not wait:
/// `Drop` runs on the host's loop, and the thread is asleep between polls,
/// so a join would stall the compositor for up to [`POLL_PERIOD`].
#[derive(Debug)]
pub struct Poller(Arc<Shared>);

impl Poller {
    /// Start polling `client` every [`POLL_PERIOD`]. A poller that cannot start
    /// is the miner unreachable for good, so a spawn failure shows red at once.
    #[must_use]
    pub fn spawn(client: BosClient) -> Self {
        let shared = Arc::new(Shared::default());
        let publisher = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("mining-status-poller".to_owned())
            .stack_size(STACK_SIZE)
            .spawn(move || run(client, &publisher, POLL_PERIOD));
        if let Err(err) = spawned {
            tracing::error!("failed to spawn mining status poller thread: {err}");
            shared.publish(Status::Low);
        }
        Self(shared)
    }

    /// The status to show and its version when it changed since `seen`.
    #[must_use]
    pub fn status_if_changed(
        &self,
        seen: Option<StatusVersion>,
    ) -> Option<(StatusVersion, Status)> {
        self.0.status_if_changed(seen)
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bos::PollFailure;
    use crate::mining::{Board, Readings, TunerState};

    fn failed() -> Poll {
        Err(PollFailure::Transport("refused".to_owned()))
    }

    /// A poll that panics, caught the way [`run`] catches it.
    fn panicked(message: &'static str) -> Poll {
        std::panic::catch_unwind(|| -> Poll { panic!("{message}") })
            .unwrap_or_else(|payload| Err(PollFailure::Panicked(panic_message(&*payload))))
    }

    fn healthy() -> Readings {
        Readings {
            tuner: TunerState::Stable,
            boards: vec![Board {
                enabled: true,
                nominal_ghs: Some(500.0),
                last_1m_ghs: Some(500.0),
                last_5m_ghs: Some(500.0),
            }],
        }
    }

    #[test]
    fn nothing_is_published_until_the_budget_runs_out() {
        let shared = Shared::default();
        let mut tracker = StatusTracker::default();
        for _ in 1..FAILURES_BEFORE_LOW {
            absorb(&mut tracker, &shared, &failed());
            assert_eq!(shared.status_if_changed(None), None);
        }
        absorb(&mut tracker, &shared, &failed());
        assert_eq!(
            shared.status_if_changed(None),
            Some((StatusVersion::FIRST, Status::Low))
        );
    }

    #[test]
    fn a_panicking_poll_still_runs_the_budget_down() {
        let shared = Shared::default();
        let mut tracker = StatusTracker::default();
        absorb(&mut tracker, &shared, &Ok(healthy()));
        for _ in 0..FAILURES_BEFORE_LOW {
            absorb(&mut tracker, &shared, &panicked("the resolver thread"));
        }
        assert_eq!(
            shared.status_if_changed(Some(StatusVersion::FIRST)),
            Some((StatusVersion::FIRST.next(), Status::Low))
        );
    }

    #[test]
    fn a_caught_panic_carries_its_message() {
        let literal: Result<(), _> = std::panic::catch_unwind(|| panic!("out of threads"));
        let formatted: Result<(), _> =
            std::panic::catch_unwind(|| panic!("{}", "out of threads".to_owned()));
        for payload in [literal, formatted] {
            let payload = payload.expect_err("the closure panics");
            assert_eq!(panic_message(&*payload), "out of threads");
        }
    }

    #[test]
    fn an_unchanged_status_is_not_republished() {
        let shared = Shared::default();
        let mut tracker = StatusTracker::default();
        absorb(&mut tracker, &shared, &Ok(healthy()));
        let (first, status) = shared
            .status_if_changed(None)
            .expect("the first success publishes");
        assert_eq!(status, Status::Ok);

        absorb(&mut tracker, &shared, &Ok(healthy()));
        absorb(&mut tracker, &shared, &failed());
        assert_eq!(
            shared.status_if_changed(Some(first)),
            None,
            "a repeat and a failure inside the budget change nothing shown"
        );
    }

    #[test]
    fn dropping_the_poller_ends_the_thread() {
        let shared = Arc::new(Shared::default());
        let thread_shared = Arc::clone(&shared);
        let unanswered = crate::bos::unanswered_api_url();
        let thread = std::thread::spawn(move || {
            run(
                BosClient::new(unanswered),
                &thread_shared,
                Duration::from_millis(10),
            );
        });

        drop(Poller(Arc::clone(&shared)));

        thread
            .join()
            .expect("the loop returns once the stop flag is raised");
        assert!(shared.stop.load(Ordering::Relaxed));
    }

    #[test]
    fn a_recovery_publishes_the_next_version() {
        let shared = Shared::default();
        let mut tracker = StatusTracker::default();
        for _ in 0..FAILURES_BEFORE_LOW {
            absorb(&mut tracker, &shared, &failed());
        }
        absorb(&mut tracker, &shared, &Ok(healthy()));
        assert_eq!(
            shared.status_if_changed(Some(StatusVersion::FIRST)),
            Some((StatusVersion::FIRST.next(), Status::Ok))
        );
    }
}
