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

//! Cache and Wayland dispatch for coherent upgrade-progress snapshots.

use std::time::{Duration, Instant};

use ::deck_upgrade_v1::server::deck_upgrade_v1::{self, DeckUpgradeV1, Kind, Phase};
use bmc::compositor::{UpgradeDisplaySnapshot, UpgradeDisplayState, UpgradeKind, UpgradePhase};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::state::CompositorState;

const TERMINAL_LIFETIME: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
enum WireEvent {
    Started(Kind),
    Phase(Phase),
    DownloadProgress {
        downloaded_bytes: u64,
    },
    DownloadProgressWithTotal {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Succeeded {
        remaining_ms: u32,
    },
    Failed {
        remaining_ms: u32,
    },
    SnapshotDone,
}

#[derive(Debug, Clone)]
struct CachedSnapshot {
    snapshot: UpgradeDisplaySnapshot,
    deadline: Option<Instant>,
}

/// Pure snapshot/deadline cache, separated from Wayland resources for tests.
#[derive(Debug, Default)]
struct UpgradeCache {
    current: Option<CachedSnapshot>,
}

impl UpgradeCache {
    fn set(&mut self, snapshot: UpgradeDisplaySnapshot, now: Instant) {
        let deadline = if matches!(
            snapshot.state,
            UpgradeDisplayState::Succeeded { .. } | UpgradeDisplayState::Failed { .. }
        ) {
            self.current
                .as_ref()
                .filter(|current| current.snapshot.generation == snapshot.generation)
                .and_then(|current| current.deadline)
                .or_else(|| {
                    Some(
                        now.checked_add(TERMINAL_LIFETIME)
                            .expect("BUG: upgrade terminal deadline overflows Instant"),
                    )
                })
        } else {
            None
        };
        self.current = Some(CachedSnapshot { snapshot, deadline });
    }

    fn events(&self, now: Instant) -> Option<Vec<WireEvent>> {
        let cached = self.current.as_ref()?;
        let mut events = vec![WireEvent::Started(kind(&cached.snapshot.state))];
        match &cached.snapshot.state {
            UpgradeDisplayState::Running {
                phase, progress, ..
            } => {
                if let Some(phase) = phase {
                    events.push(WireEvent::Phase(phase_to_wire(*phase)));
                }
                if let Some(progress) = progress {
                    events.push(match progress.total_bytes {
                        Some(total_bytes) => WireEvent::DownloadProgressWithTotal {
                            downloaded_bytes: progress.downloaded_bytes,
                            total_bytes,
                        },
                        None => WireEvent::DownloadProgress {
                            downloaded_bytes: progress.downloaded_bytes,
                        },
                    });
                }
            }
            UpgradeDisplayState::Succeeded { .. } => {
                events.push(WireEvent::Succeeded {
                    remaining_ms: remaining_ms(cached.deadline?, now)?,
                });
            }
            UpgradeDisplayState::Failed { .. } => {
                events.push(WireEvent::Failed {
                    remaining_ms: remaining_ms(cached.deadline?, now)?,
                });
            }
        }
        events.push(WireEvent::SnapshotDone);
        Some(events)
    }
}

fn remaining_ms(deadline: Instant, now: Instant) -> Option<u32> {
    if now >= deadline {
        return None;
    }
    let duration = deadline
        .checked_duration_since(now)
        .expect("BUG: deadline follows current instant");
    let milliseconds = duration.as_millis().max(1);
    Some(u32::try_from(milliseconds).expect("BUG: terminal lifetime fits u32"))
}

fn kind(state: &UpgradeDisplayState) -> Kind {
    match state {
        UpgradeDisplayState::Running { kind, .. }
        | UpgradeDisplayState::Succeeded { kind }
        | UpgradeDisplayState::Failed { kind } => match kind {
            UpgradeKind::Firmware => Kind::Firmware,
            UpgradeKind::Packages => Kind::Packages,
        },
    }
}

fn phase_to_wire(phase: UpgradePhase) -> Phase {
    match phase {
        UpgradePhase::FirmwareDownloading => Phase::FirmwareDownloading,
        UpgradePhase::FirmwareVerifying => Phase::FirmwareVerifying,
        UpgradePhase::FirmwareApplying => Phase::FirmwareApplying,
        UpgradePhase::PackageRealizing => Phase::PackageRealizing,
        UpgradePhase::PackageVerifying => Phase::PackageVerifying,
        UpgradePhase::PackageBuilding => Phase::PackageBuilding,
        UpgradePhase::PackageActivating => Phase::PackageActivating,
    }
}

#[derive(Debug, Default)]
pub struct UpgradeState {
    cache: UpgradeCache,
    resources: Vec<DeckUpgradeV1>,
}

impl UpgradeState {
    pub fn set(&mut self, snapshot: UpgradeDisplaySnapshot, now: Instant) {
        self.cache.set(snapshot, now);
        if let Some(events) = self.cache.events(now) {
            self.resources.retain(Resource::is_alive);
            for resource in &self.resources {
                emit(resource, &events);
            }
        }
    }

    fn replay(&mut self, resource: &DeckUpgradeV1) {
        if let Some(events) = self.cache.events(Instant::now()) {
            emit(resource, &events);
        }
    }

    fn remove(&mut self, resource: &DeckUpgradeV1) {
        self.resources.retain(|candidate| candidate != resource);
    }

    #[cfg(test)]
    pub fn current_snapshot(&self) -> Option<&UpgradeDisplaySnapshot> {
        self.cache.current.as_ref().map(|cached| &cached.snapshot)
    }
}

fn emit(resource: &DeckUpgradeV1, events: &[WireEvent]) {
    for event in events {
        match event {
            WireEvent::Started(kind) => resource.started(*kind),
            WireEvent::Phase(phase) => resource.phase(*phase),
            WireEvent::DownloadProgress { downloaded_bytes } => {
                resource.send_download_progress(*downloaded_bytes);
            }
            WireEvent::DownloadProgressWithTotal {
                downloaded_bytes,
                total_bytes,
            } => resource.send_download_progress_with_total(*downloaded_bytes, *total_bytes),
            WireEvent::Succeeded { remaining_ms } => resource.succeeded(*remaining_ms),
            WireEvent::Failed { remaining_ms } => resource.failed(*remaining_ms),
            WireEvent::SnapshotDone => resource.snapshot_done(),
        }
    }
}

impl GlobalDispatch<DeckUpgradeV1, ()> for CompositorState {
    fn bind(
        state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<DeckUpgradeV1>,
        (): &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let resource = data_init.init(resource, ());
        state.upgrade.replay(&resource);
        state.upgrade.resources.push(resource);
    }
}

impl Dispatch<DeckUpgradeV1, ()> for CompositorState {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &DeckUpgradeV1,
        request: deck_upgrade_v1::Request,
        (): &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            deck_upgrade_v1::Request::Destroy => state.upgrade.remove(resource),
            other => tracing::warn!("Unknown deck_upgrade_v1 request: {other:?}"),
        }
    }
}

pub fn create_global(display: &DisplayHandle) {
    display.create_global::<CompositorState, DeckUpgradeV1, ()>(1, ());
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc::compositor::{DownloadProgress, UpgradeDisplayState, UpgradeGeneration};

    fn running(generation: usize) -> UpgradeDisplaySnapshot {
        UpgradeDisplaySnapshot {
            generation: UpgradeGeneration::new(generation),
            state: UpgradeDisplayState::Running {
                kind: UpgradeKind::Packages,
                phase: Some(UpgradePhase::PackageRealizing),
                progress: Some(DownloadProgress {
                    downloaded_bytes: 3,
                    total_bytes: Some(5),
                }),
            },
        }
    }

    #[test]
    fn running_snapshot_emits_a_coherent_ordered_sequence() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(running(1), now);
        assert_eq!(
            cache.events(now),
            Some(vec![
                WireEvent::Started(Kind::Packages),
                WireEvent::Phase(Phase::PackageRealizing),
                WireEvent::DownloadProgressWithTotal {
                    downloaded_bytes: 3,
                    total_bytes: 5,
                },
                WireEvent::SnapshotDone,
            ])
        );
    }

    #[test]
    fn initial_running_snapshot_emits_only_started_and_done() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Running {
                    kind: UpgradeKind::Firmware,
                    phase: None,
                    progress: None,
                },
            },
            now,
        );
        assert_eq!(
            cache.events(now),
            Some(vec![
                WireEvent::Started(Kind::Firmware),
                WireEvent::SnapshotDone
            ])
        );
    }

    #[test]
    fn unknown_total_progress_emits_only_downloaded_bytes() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Running {
                    kind: UpgradeKind::Packages,
                    phase: None,
                    progress: Some(DownloadProgress {
                        downloaded_bytes: u64::from(u32::MAX) + 1,
                        total_bytes: None,
                    }),
                },
            },
            now,
        );
        assert_eq!(
            cache.events(now),
            Some(vec![
                WireEvent::Started(Kind::Packages),
                WireEvent::DownloadProgress {
                    downloaded_bytes: u64::from(u32::MAX) + 1,
                },
                WireEvent::SnapshotDone,
            ])
        );
    }

    #[test]
    fn terminal_snapshot_emits_started_terminal_and_done_without_a_prior_running_state() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Succeeded {
                    kind: UpgradeKind::Firmware,
                },
            },
            now,
        );
        assert_eq!(
            cache.events(now),
            Some(vec![
                WireEvent::Started(Kind::Firmware),
                WireEvent::Succeeded {
                    remaining_ms: 10_000
                },
                WireEvent::SnapshotDone,
            ])
        );
    }

    #[test]
    fn repeated_terminal_snapshots_keep_the_original_deadline() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        let terminal = UpgradeDisplaySnapshot {
            generation: UpgradeGeneration::new(1),
            state: UpgradeDisplayState::Failed {
                kind: UpgradeKind::Packages,
            },
        };
        cache.set(terminal.clone(), now);
        cache.set(terminal, now + Duration::from_secs(1));
        assert_eq!(
            cache.events(now + Duration::from_secs(4)),
            Some(vec![
                WireEvent::Started(Kind::Packages),
                WireEvent::Failed {
                    remaining_ms: 6_000
                },
                WireEvent::SnapshotDone,
            ])
        );
    }

    #[test]
    fn terminal_replay_is_suppressed_at_deadline() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Failed {
                    kind: UpgradeKind::Packages,
                },
            },
            now,
        );
        assert_eq!(cache.events(now + TERMINAL_LIFETIME), None);
        assert_eq!(
            cache.events(now + TERMINAL_LIFETIME + Duration::from_nanos(1)),
            None
        );
    }

    #[test]
    fn positive_sub_millisecond_terminal_remainder_rounds_up_to_one_millisecond() {
        let now = Instant::now();
        assert_eq!(remaining_ms(now + Duration::from_nanos(1), now), Some(1));
    }

    #[test]
    fn a_new_generation_replaces_an_expired_terminal_snapshot() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Failed {
                    kind: UpgradeKind::Packages,
                },
            },
            now,
        );
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(2),
                state: UpgradeDisplayState::Failed {
                    kind: UpgradeKind::Packages,
                },
            },
            now + TERMINAL_LIFETIME,
        );
        assert_eq!(
            cache.events(now + TERMINAL_LIFETIME),
            Some(vec![
                WireEvent::Started(Kind::Packages),
                WireEvent::Failed {
                    remaining_ms: 10_000
                },
                WireEvent::SnapshotDone,
            ])
        );
    }

    #[test]
    fn a_new_running_generation_replaces_a_terminal_snapshot_without_terminal_events() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Failed {
                    kind: UpgradeKind::Firmware,
                },
            },
            now,
        );
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(2),
                state: UpgradeDisplayState::Running {
                    kind: UpgradeKind::Packages,
                    phase: None,
                    progress: None,
                },
            },
            now + Duration::from_secs(1),
        );
        assert_eq!(
            cache.events(now + Duration::from_secs(1)),
            Some(vec![
                WireEvent::Started(Kind::Packages),
                WireEvent::SnapshotDone
            ])
        );
    }

    #[test]
    fn coalesced_second_failure_replaces_the_cached_generation() {
        let now = Instant::now();
        let mut cache = UpgradeCache::default();
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(1),
                state: UpgradeDisplayState::Failed {
                    kind: UpgradeKind::Packages,
                },
            },
            now,
        );
        cache.set(
            UpgradeDisplaySnapshot {
                generation: UpgradeGeneration::new(2),
                state: UpgradeDisplayState::Failed {
                    kind: UpgradeKind::Packages,
                },
            },
            now + Duration::from_secs(1),
        );
        assert_eq!(
            cache
                .current
                .as_ref()
                .expect("BUG: second failure remains cached")
                .snapshot
                .generation,
            UpgradeGeneration::new(2)
        );
        assert_eq!(
            cache.events(now + Duration::from_secs(5)),
            Some(vec![
                WireEvent::Started(Kind::Packages),
                WireEvent::Failed {
                    remaining_ms: 6_000
                },
                WireEvent::SnapshotDone,
            ])
        );
    }
}
