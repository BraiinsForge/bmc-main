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

//! Client-side decoder assembling coherent `deck_upgrade_v1` snapshots
//! from the wire event stream, enforcing the sequencing rules
//! the protocol XML specifies.

use std::time::Duration;

use crate::client::deck_upgrade_v1::{Event, Kind, Phase};
use crate::join_u64;

/// Optional byte progress for the active stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// A coherent upgrade state committed by `snapshot_done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpgradeSnapshot {
    pub kind: Kind,
    pub state: UpgradeState,
}

/// Current lifecycle state of an upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeState {
    Running {
        phase: Option<Phase>,
        progress: Option<DownloadProgress>,
    },
    Succeeded {
        remaining: Duration,
    },
    Failed {
        remaining: Duration,
    },
}

#[derive(Debug, Clone, Copy)]
enum UpgradeTerminal {
    Succeeded { remaining: Duration },
    Failed { remaining: Duration },
}

#[derive(Debug, Clone, Copy)]
struct UpgradeCandidate {
    kind: Kind,
    phase: Option<Phase>,
    progress: Option<DownloadProgress>,
    terminal: Option<UpgradeTerminal>,
}

#[derive(Debug, Clone, Copy)]
enum UpgradeCandidateState {
    Valid(UpgradeCandidate),
    Invalid,
}

/// Decodes one complete `deck_upgrade_v1` snapshot at a time.
///
/// Malformed candidates remain invalid until `snapshot_done`, which prevents a
/// later event in the same wire sequence from becoming a partial snapshot.
#[derive(Debug, Default)]
pub struct UpgradeDecoder {
    candidate: Option<UpgradeCandidateState>,
}

impl UpgradeDecoder {
    /// Feed one wire event; returns a snapshot
    /// when `snapshot_done` commits a coherent candidate.
    pub fn decode(&mut self, event: &Event) -> Option<UpgradeSnapshot> {
        match event {
            Event::Started { kind } => {
                self.started(kind.into_result().ok());
                None
            }
            Event::Phase { phase } => {
                self.phase(phase.into_result().ok());
                None
            }
            Event::DownloadProgress {
                downloaded_bytes_hi,
                downloaded_bytes_lo,
            } => {
                self.progress(DownloadProgress {
                    downloaded_bytes: join_u64(*downloaded_bytes_hi, *downloaded_bytes_lo),
                    total_bytes: None,
                });
                None
            }
            Event::DownloadProgressWithTotal {
                downloaded_bytes_hi,
                downloaded_bytes_lo,
                total_bytes_hi,
                total_bytes_lo,
            } => {
                self.progress(DownloadProgress {
                    downloaded_bytes: join_u64(*downloaded_bytes_hi, *downloaded_bytes_lo),
                    total_bytes: Some(join_u64(*total_bytes_hi, *total_bytes_lo)),
                });
                None
            }
            Event::Succeeded { remaining_ms } => {
                self.terminal(UpgradeTerminal::Succeeded {
                    remaining: Duration::from_millis(u64::from(*remaining_ms)),
                });
                None
            }
            Event::Failed { remaining_ms } => {
                self.terminal(UpgradeTerminal::Failed {
                    remaining: Duration::from_millis(u64::from(*remaining_ms)),
                });
                None
            }
            Event::SnapshotDone => self.snapshot_done(),
        }
    }

    fn started(&mut self, kind: Option<Kind>) {
        if self.candidate.is_some() {
            self.invalidate_active();
            return;
        }

        self.candidate = Some(match kind {
            Some(kind) => UpgradeCandidateState::Valid(UpgradeCandidate {
                kind,
                phase: None,
                progress: None,
                terminal: None,
            }),
            None => UpgradeCandidateState::Invalid,
        });
    }

    fn phase(&mut self, phase: Option<Phase>) {
        if self.candidate.is_none() {
            return;
        }
        let Some(phase) = phase else {
            self.invalidate_active();
            return;
        };
        let kind = match self.candidate {
            Some(UpgradeCandidateState::Valid(UpgradeCandidate {
                kind,
                phase: None,
                progress: None,
                terminal: None,
                ..
            })) => kind,
            Some(UpgradeCandidateState::Valid(_) | UpgradeCandidateState::Invalid) | None => {
                self.invalidate_active();
                return;
            }
        };
        if !phase_is_valid_for_kind(kind, phase) {
            self.invalidate_active();
            return;
        }
        if let Some(UpgradeCandidateState::Valid(candidate)) = &mut self.candidate {
            candidate.phase = Some(phase);
        }
    }

    fn progress(&mut self, progress: DownloadProgress) {
        if self.candidate.is_none() {
            return;
        }
        if !matches!(
            self.candidate,
            Some(UpgradeCandidateState::Valid(UpgradeCandidate {
                progress: None,
                terminal: None,
                ..
            }))
        ) {
            self.invalidate_active();
            return;
        }
        if let Some(UpgradeCandidateState::Valid(candidate)) = &mut self.candidate {
            candidate.progress = Some(progress);
        }
    }

    fn terminal(&mut self, terminal: UpgradeTerminal) {
        if self.candidate.is_none() {
            return;
        }
        if !matches!(
            self.candidate,
            Some(UpgradeCandidateState::Valid(UpgradeCandidate {
                phase: None,
                progress: None,
                terminal: None,
                ..
            }))
        ) {
            self.invalidate_active();
            return;
        }
        if let Some(UpgradeCandidateState::Valid(candidate)) = &mut self.candidate {
            candidate.terminal = Some(terminal);
        }
    }

    fn snapshot_done(&mut self) -> Option<UpgradeSnapshot> {
        let UpgradeCandidateState::Valid(candidate) = self.candidate.take()? else {
            return None;
        };
        let state = match candidate.terminal {
            Some(UpgradeTerminal::Succeeded { remaining }) => UpgradeState::Succeeded { remaining },
            Some(UpgradeTerminal::Failed { remaining }) => UpgradeState::Failed { remaining },
            None => UpgradeState::Running {
                phase: candidate.phase,
                progress: candidate.progress,
            },
        };
        Some(UpgradeSnapshot {
            kind: candidate.kind,
            state,
        })
    }

    fn invalidate_active(&mut self) {
        if self.candidate.is_some() {
            self.candidate = Some(UpgradeCandidateState::Invalid);
        }
    }
}

fn phase_is_valid_for_kind(kind: Kind, phase: Phase) -> bool {
    match kind {
        Kind::Firmware => true,
        Kind::Packages => matches!(
            phase,
            Phase::PackageRealizing
                | Phase::PackageVerifying
                | Phase::PackageBuilding
                | Phase::PackageActivating
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::split_u64;
    use wayland_client::WEnum;

    fn started(kind: Kind) -> Event {
        Event::Started {
            kind: WEnum::Value(kind),
        }
    }

    fn phase(phase: Phase) -> Event {
        Event::Phase {
            phase: WEnum::Value(phase),
        }
    }

    fn download_progress(downloaded_bytes: u64) -> Event {
        let (downloaded_bytes_hi, downloaded_bytes_lo) = split_u64(downloaded_bytes);
        Event::DownloadProgress {
            downloaded_bytes_hi,
            downloaded_bytes_lo,
        }
    }

    fn download_progress_with_total(downloaded_bytes: u64, total_bytes: u64) -> Event {
        let (downloaded_bytes_hi, downloaded_bytes_lo) = split_u64(downloaded_bytes);
        let (total_bytes_hi, total_bytes_lo) = split_u64(total_bytes);
        Event::DownloadProgressWithTotal {
            downloaded_bytes_hi,
            downloaded_bytes_lo,
            total_bytes_hi,
            total_bytes_lo,
        }
    }

    fn decode_all(decoder: &mut UpgradeDecoder, events: impl IntoIterator<Item = Event>) {
        for event in events {
            decoder.decode(&event);
        }
    }

    fn running_snapshot(
        kind: Kind,
        phase: Option<Phase>,
        progress: Option<DownloadProgress>,
    ) -> UpgradeSnapshot {
        UpgradeSnapshot {
            kind,
            state: UpgradeState::Running { phase, progress },
        }
    }

    fn assert_invalid_upgrade_sequence(events: impl IntoIterator<Item = Event>) {
        let mut decoder = UpgradeDecoder::default();
        decode_all(&mut decoder, events);
        assert_eq!(
            decoder.decode(&Event::SnapshotDone),
            None,
            "malformed sequence must not publish a partial snapshot"
        );
    }

    #[test]
    fn firmware_upgrade_decoder_accepts_every_known_phase() {
        for wire_phase in [
            Phase::FirmwareDownloading,
            Phase::FirmwareVerifying,
            Phase::FirmwareApplying,
            Phase::PackageRealizing,
            Phase::PackageVerifying,
            Phase::PackageBuilding,
            Phase::PackageActivating,
        ] {
            let mut decoder = UpgradeDecoder::default();
            decode_all(&mut decoder, [started(Kind::Firmware), phase(wire_phase)]);

            assert_eq!(
                decoder.decode(&Event::SnapshotDone),
                Some(running_snapshot(Kind::Firmware, Some(wire_phase), None)),
                "firmware upgrade must accept {wire_phase:?}"
            );
        }
    }

    #[test]
    fn package_upgrade_decoder_accepts_only_package_phases() {
        for wire_phase in [
            Phase::PackageRealizing,
            Phase::PackageVerifying,
            Phase::PackageBuilding,
            Phase::PackageActivating,
        ] {
            let mut decoder = UpgradeDecoder::default();
            decode_all(&mut decoder, [started(Kind::Packages), phase(wire_phase)]);
            assert_eq!(
                decoder.decode(&Event::SnapshotDone),
                Some(running_snapshot(Kind::Packages, Some(wire_phase), None))
            );
        }

        for wire_phase in [
            Phase::FirmwareDownloading,
            Phase::FirmwareVerifying,
            Phase::FirmwareApplying,
        ] {
            assert_invalid_upgrade_sequence([started(Kind::Packages), phase(wire_phase)]);
        }
    }

    #[test]
    fn upgrade_decoder_ignores_events_before_started() {
        let mut decoder = UpgradeDecoder::default();

        decode_all(
            &mut decoder,
            [
                phase(Phase::PackageRealizing),
                download_progress(3),
                download_progress_with_total(3, 5),
                Event::Succeeded { remaining_ms: 10 },
                Event::Failed { remaining_ms: 10 },
            ],
        );
        assert_eq!(decoder.decode(&Event::SnapshotDone), None);

        decode_all(&mut decoder, [started(Kind::Packages)]);
        assert_eq!(
            decoder.decode(&Event::SnapshotDone),
            Some(running_snapshot(Kind::Packages, None, None))
        );
    }

    #[test]
    fn unknown_upgrade_enums_invalidate_the_candidate_until_done() {
        let mut decoder = UpgradeDecoder::default();
        decode_all(
            &mut decoder,
            [
                Event::Started {
                    kind: WEnum::Unknown(99),
                },
                phase(Phase::PackageRealizing),
            ],
        );
        assert_eq!(decoder.decode(&Event::SnapshotDone), None);

        decode_all(
            &mut decoder,
            [
                started(Kind::Firmware),
                Event::Phase {
                    phase: WEnum::Unknown(99),
                },
            ],
        );
        assert_eq!(decoder.decode(&Event::SnapshotDone), None);
    }

    #[test]
    fn upgrade_decoder_preserves_both_progress_forms_and_u64_boundaries() {
        for downloaded_bytes in [0, u64::from(u32::MAX), u64::from(u32::MAX) + 1, u64::MAX] {
            let mut decoder = UpgradeDecoder::default();
            decode_all(
                &mut decoder,
                [started(Kind::Packages), download_progress(downloaded_bytes)],
            );
            assert_eq!(
                decoder.decode(&Event::SnapshotDone),
                Some(running_snapshot(
                    Kind::Packages,
                    None,
                    Some(DownloadProgress {
                        downloaded_bytes,
                        total_bytes: None,
                    }),
                )),
                "unknown-total progress must preserve {downloaded_bytes:#x}"
            );
        }

        let mut decoder = UpgradeDecoder::default();
        decode_all(
            &mut decoder,
            [
                started(Kind::Firmware),
                download_progress_with_total(u64::from(u32::MAX) + 1, u64::MAX),
            ],
        );
        assert_eq!(
            decoder.decode(&Event::SnapshotDone),
            Some(running_snapshot(
                Kind::Firmware,
                None,
                Some(DownloadProgress {
                    downloaded_bytes: u64::from(u32::MAX) + 1,
                    total_bytes: Some(u64::MAX),
                }),
            ))
        );

        let mut decoder = UpgradeDecoder::default();
        decode_all(
            &mut decoder,
            [
                started(Kind::Packages),
                phase(Phase::PackageRealizing),
                download_progress_with_total(3, 5),
            ],
        );
        assert_eq!(
            decoder.decode(&Event::SnapshotDone),
            Some(running_snapshot(
                Kind::Packages,
                Some(Phase::PackageRealizing),
                Some(DownloadProgress {
                    downloaded_bytes: 3,
                    total_bytes: Some(5),
                }),
            ))
        );
    }

    #[test]
    fn terminal_upgrade_snapshots_require_no_phase_or_progress() {
        let mut decoder = UpgradeDecoder::default();
        decode_all(
            &mut decoder,
            [
                started(Kind::Firmware),
                Event::Succeeded {
                    remaining_ms: 1_500,
                },
            ],
        );
        assert_eq!(
            decoder.decode(&Event::SnapshotDone),
            Some(UpgradeSnapshot {
                kind: Kind::Firmware,
                state: UpgradeState::Succeeded {
                    remaining: Duration::from_millis(1_500),
                },
            })
        );

        let mut decoder = UpgradeDecoder::default();
        decode_all(
            &mut decoder,
            [started(Kind::Packages), Event::Failed { remaining_ms: 250 }],
        );
        assert_eq!(
            decoder.decode(&Event::SnapshotDone),
            Some(UpgradeSnapshot {
                kind: Kind::Packages,
                state: UpgradeState::Failed {
                    remaining: Duration::from_millis(250),
                },
            })
        );
    }

    #[test]
    fn duplicate_and_misordered_upgrade_events_invalidate_until_done() {
        assert_invalid_upgrade_sequence([
            started(Kind::Packages),
            phase(Phase::PackageRealizing),
            phase(Phase::PackageVerifying),
        ]);
        assert_invalid_upgrade_sequence([
            started(Kind::Packages),
            download_progress(1),
            download_progress_with_total(1, 2),
        ]);
        assert_invalid_upgrade_sequence([
            started(Kind::Packages),
            download_progress(1),
            phase(Phase::PackageRealizing),
        ]);
        assert_invalid_upgrade_sequence([
            started(Kind::Firmware),
            phase(Phase::FirmwareDownloading),
            Event::Succeeded { remaining_ms: 1 },
        ]);
        assert_invalid_upgrade_sequence([
            started(Kind::Firmware),
            Event::Failed { remaining_ms: 1 },
            download_progress(1),
        ]);
        assert_invalid_upgrade_sequence([
            started(Kind::Firmware),
            Event::Succeeded { remaining_ms: 1 },
            Event::Failed { remaining_ms: 1 },
        ]);
        assert_invalid_upgrade_sequence([started(Kind::Firmware), started(Kind::Packages)]);
    }
}
