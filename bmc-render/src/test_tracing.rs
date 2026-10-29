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

//! Test-only tracing subscribers, for asserting that a diagnostic fired.

use std::cell::Cell;
use std::sync::OnceLock;

thread_local! {
    /// The WARN records this thread has emitted since the subscriber was installed.
    static WARNS: Cell<usize> = const { Cell::new(0) };
}

/// Counts WARN records into the emitting thread's tally,
/// without a `tracing-subscriber` dev dependency.
struct ThreadWarnTally;

impl tracing::Subscriber for ThreadWarnTally {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }

    fn event(&self, _: &tracing::Event<'_>) {
        WARNS.with(|warns| warns.set(warns.get() + 1));
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// The calling thread's WARN records since the counter was made or last reset.
#[derive(Debug)]
pub struct WarnCounter {
    baseline: Cell<usize>,
}

impl WarnCounter {
    fn new() -> Self {
        Self {
            baseline: Cell::new(WARNS.with(Cell::get)),
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        WARNS.with(Cell::get) - self.baseline.get()
    }

    pub fn reset(&self) {
        self.baseline.set(WARNS.with(Cell::get));
    }
}

/// Run `scenario`, returning its result and how many WARN records it emitted on this thread.
///
/// The counting subscriber is global and installed once.
/// tracing caches a callsite's interest process-wide on its first hit,
/// so a test running beside this one with no subscriber
/// would otherwise cache `never` for a warning this one counts.
///
/// # Panics
///
/// Panics when another global subscriber was installed first:
/// a test binary that counts warnings must install no global subscriber of its own.
pub fn counting_warns<T>(scenario: impl FnOnce(&WarnCounter) -> T) -> (T, usize) {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        tracing::subscriber::set_global_default(ThreadWarnTally)
            .expect("BUG: counting_warns installs the only global subscriber");
    });
    let warns = WarnCounter::new();
    let result = scenario(&warns);
    (result, warns.count())
}
