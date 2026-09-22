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

/// Counts WARN records without a `tracing-subscriber` dev dependency.
#[derive(Clone, Default)]
pub(crate) struct WarnCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl WarnCounter {
    pub(crate) fn count(&self) -> usize {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub(crate) fn reset(&self) {
        self.0.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}

impl tracing::Subscriber for WarnCounter {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }

    fn event(&self, _: &tracing::Event<'_>) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// Run `scenario` with a [`WarnCounter`] installed,
/// returning its result and how many WARN records it emitted.
pub(crate) fn counting_warns<T>(scenario: impl FnOnce(&WarnCounter) -> T) -> (T, usize) {
    let warns = WarnCounter::default();
    let result = tracing::subscriber::with_default(warns.clone(), || {
        // tracing caches a callsite's interest process-wide on first reach,
        // so a sibling test reaching it without a subscriber would mute it.
        tracing::callsite::rebuild_interest_cache();
        scenario(&warns)
    });
    (result, warns.count())
}
