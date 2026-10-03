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

use super::*;
use crate::compositor::{UpgradeKind, UpgradePhase, UpgradeRunStatus};

fn generation(value: usize) -> UpgradeGeneration {
    UpgradeGeneration::new(value)
}

fn running(value: usize, kind: UpgradeKind, phase: Option<UpgradePhase>) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: generation(value),
        id: None,
        state: UpgradeRunStatus::Running {
            kind,
            phase,
            progress: None,
        },
    }
}

fn downloading(value: usize) -> UpgradeRunSnapshot {
    running(
        value,
        UpgradeKind::Firmware,
        Some(UpgradePhase::FirmwareDownloading),
    )
}

fn flashing(value: usize) -> UpgradeRunSnapshot {
    running(
        value,
        UpgradeKind::Firmware,
        Some(UpgradePhase::FirmwareApplying),
    )
}

fn failed(value: usize, kind: UpgradeKind) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: generation(value),
        id: None,
        state: UpgradeRunStatus::Failed {
            kind,
            reason: String::new(),
        },
    }
}

fn succeeded(value: usize, kind: UpgradeKind) -> UpgradeRunSnapshot {
    UpgradeRunSnapshot {
        generation: generation(value),
        id: None,
        state: UpgradeRunStatus::Succeeded { kind },
    }
}

fn paused(value: usize, stage: Stage) -> Pause {
    Pause::Paused {
        generation: generation(value),
        stage,
        lapses_at: None,
    }
}

#[test]
fn a_firmware_run_pauses_widgets_from_its_first_snapshot() {
    let now = Instant::now();
    assert_eq!(
        step(
            Pause::Idle,
            Some(&running(0, UpgradeKind::Firmware, None)),
            now
        ),
        paused(0, Stage::Preparing),
        "the image lands on tmpfs, so widgets must go before the download phase is even reported"
    );
}

#[test]
fn a_combined_run_pauses_widgets_like_a_firmware_run() {
    let now = Instant::now();
    assert_eq!(
        step(
            Pause::Idle,
            Some(&running(
                0,
                UpgradeKind::FirmwareAndPackages,
                Some(UpgradePhase::FirmwareDownloading)
            )),
            now
        ),
        paused(0, Stage::Preparing),
        "a combined run downloads a firmware image too"
    );
}

#[test]
fn a_run_first_seen_while_flashing_starts_in_the_applying_stage() {
    let now = Instant::now();
    assert_eq!(
        step(Pause::Idle, Some(&flashing(0)), now),
        paused(0, Stage::Applying)
    );
}

#[test]
fn package_runs_terminals_and_silence_leave_running_widgets_alone() {
    let now = Instant::now();
    for snapshot in [
        None,
        Some(running(
            0,
            UpgradeKind::Packages,
            Some(UpgradePhase::PackageRealizing),
        )),
        Some(succeeded(0, UpgradeKind::Packages)),
        Some(failed(0, UpgradeKind::Firmware)),
        // The post-reboot overlay and Boser's replayed terminal arrive here.
        Some(succeeded(0, UpgradeKind::Firmware)),
    ] {
        assert_eq!(
            step(Pause::Idle, snapshot.as_ref(), now),
            Pause::Idle,
            "{snapshot:?}"
        );
    }
}

#[test]
fn a_new_firmware_generation_adopts_the_pause_in_its_own_stage() {
    let now = Instant::now();
    assert_eq!(
        step(paused(0, Stage::Applying), Some(&downloading(1)), now),
        paused(1, Stage::Preparing),
        "Boser's legacy flash is a new execution after the download; the old stage must not leak into it"
    );
}

#[test]
fn the_same_run_stays_applying_once_flashing_was_reported() {
    let now = Instant::now();
    assert_eq!(
        step(paused(0, Stage::Applying), Some(&downloading(0)), now),
        paused(0, Stage::Applying)
    );
}

#[test]
fn a_package_run_releases_a_pause() {
    let now = Instant::now();
    assert_eq!(
        step(
            paused(0, Stage::Preparing),
            Some(&running(1, UpgradeKind::Packages, None)),
            now
        ),
        Pause::Idle
    );
}

#[test]
fn a_firmware_success_keeps_widgets_stopped_for_the_reboot() {
    let now = Instant::now();
    for kind in [UpgradeKind::Firmware, UpgradeKind::FirmwareAndPackages] {
        assert_eq!(
            step(paused(0, Stage::Applying), Some(&succeeded(0, kind)), now),
            paused(0, Stage::Applying),
            "{kind:?}: the reboot starts widgets fresh"
        );
    }
}

#[test]
fn a_package_success_releases_a_pause() {
    let now = Instant::now();
    assert_eq!(
        step(
            paused(0, Stage::Preparing),
            Some(&succeeded(1, UpgradeKind::Packages)),
            now
        ),
        Pause::Idle
    );
}

#[test]
fn any_failure_releases_a_pause_even_while_flashing() {
    let now = Instant::now();
    for (value, kind) in [
        (0, UpgradeKind::Firmware),
        (1, UpgradeKind::FirmwareAndPackages),
        (2, UpgradeKind::Packages),
    ] {
        assert_eq!(
            step(paused(0, Stage::Applying), Some(&failed(value, kind)), now),
            Pause::Idle,
            "Boser was alive to report {kind:?} gen {value} failing, so the board is not flashing"
        );
    }
}

#[test]
fn a_vanished_run_lapses_after_the_replacement_grace() {
    let now = Instant::now();
    assert_eq!(
        step(paused(0, Stage::Preparing), None, now),
        Pause::Paused {
            generation: generation(0),
            stage: Stage::Preparing,
            lapses_at: Some(now + REPLACEMENT_GRACE),
        }
    );
}

#[test]
fn a_vanished_flash_never_lapses() {
    let now = Instant::now();
    assert_eq!(
        step(paused(0, Stage::Applying), None, now),
        paused(0, Stage::Applying),
        "Boser going quiet mid-flash is the flash itself"
    );
}

#[test]
fn repeated_silence_keeps_the_first_deadline() {
    let start = Instant::now();
    let lapsing = step(paused(0, Stage::Preparing), None, start);
    assert_eq!(step(lapsing, None, start + REPLACEMENT_GRACE / 2), lapsing);
}

#[test]
fn a_replacement_run_cancels_a_pending_lapse() {
    let now = Instant::now();
    let lapsing = step(paused(0, Stage::Preparing), None, now);
    assert_eq!(
        step(lapsing, Some(&flashing(1)), now),
        paused(1, Stage::Applying)
    );
}

#[test]
fn a_pause_expires_only_at_its_deadline() {
    let start = Instant::now();
    let lapsing = step(paused(0, Stage::Preparing), None, start);

    assert_eq!(
        expire(
            lapsing,
            start + REPLACEMENT_GRACE - Duration::from_millis(1)
        ),
        lapsing
    );
    assert_eq!(expire(lapsing, start + REPLACEMENT_GRACE), Pause::Idle);
    assert_eq!(
        expire(paused(0, Stage::Preparing), start + REPLACEMENT_GRACE * 2),
        paused(0, Stage::Preparing)
    );
    assert_eq!(expire(Pause::Idle, start), Pause::Idle);
}

use super::test_support::{Call, ScriptedLifecycle, StopBehaviour, settle};
use crate::system_upgrade::RunStatusService;
use std::sync::Arc;

struct Listener {
    display: RunStatusService,
    acknowledged: Acknowledgement,
    widgets: Arc<ScriptedLifecycle>,
}

fn listen(stop: StopBehaviour) -> Listener {
    let display = RunStatusService::new();
    let widgets = ScriptedLifecycle::new(stop);
    let acknowledged = spawn(
        display
            .take_events()
            .expect("BUG: a new run status service still holds its events"),
        Arc::clone(&widgets) as Arc<dyn WidgetLifecycle>,
    );
    Listener {
        display,
        acknowledged,
        widgets,
    }
}

impl Listener {
    fn acknowledged(&self) -> Option<UpgradeGeneration> {
        *self.acknowledged.borrow()
    }
}

#[tokio::test(start_paused = true)]
async fn a_pause_is_acknowledged_only_once_widgets_are_stopped() {
    let listener = listen(StopBehaviour::Held);

    listener.display.publish(downloading(0));
    settle().await;
    assert_eq!(listener.widgets.calls(), [Call::Stop]);
    assert_eq!(
        listener.acknowledged(),
        None,
        "a self-managed download must not start while widgets are still stopping"
    );

    listener.widgets.release_stop();
    settle().await;
    assert_eq!(listener.acknowledged(), Some(generation(0)));
}

#[tokio::test(start_paused = true)]
async fn a_replacement_run_inherits_the_pause_without_another_stop() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.clear();
    listener.display.publish(downloading(1));
    settle().await;
    listener.display.publish(flashing(2));
    settle().await;

    assert_eq!(listener.widgets.calls(), [Call::Stop]);
    assert_eq!(listener.acknowledged(), Some(generation(2)));
}

#[tokio::test(start_paused = true)]
async fn flashing_then_silence_during_a_held_stop_keeps_widgets_stopped() {
    let listener = listen(StopBehaviour::Held);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.publish(flashing(0));
    listener.display.clear();
    settle().await;
    listener.widgets.release_stop();
    settle().await;
    tokio::time::advance(Duration::from_hours(24)).await;
    settle().await;

    assert_eq!(
        listener.widgets.calls(),
        [Call::Stop],
        "silence after FirmwareApplying is the flash; restarting widgets now races sysupgrade"
    );
}

#[tokio::test(start_paused = true)]
async fn a_failure_during_a_held_stop_restarts_widgets_once_the_stop_completes() {
    let listener = listen(StopBehaviour::Held);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.publish(failed(0, UpgradeKind::Firmware));
    settle().await;
    assert_eq!(listener.widgets.calls(), [Call::Stop]);

    listener.widgets.release_stop();
    settle().await;
    assert_eq!(listener.widgets.calls(), [Call::Stop, Call::Restart]);
    assert_eq!(listener.acknowledged(), None);
}

#[tokio::test(start_paused = true)]
async fn a_run_that_vanishes_restarts_widgets_after_the_replacement_grace() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.clear();
    settle().await;
    tokio::time::advance(REPLACEMENT_GRACE.saturating_sub(Duration::from_secs(1))).await;
    settle().await;
    assert_eq!(listener.widgets.calls(), [Call::Stop]);

    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    assert_eq!(listener.widgets.calls(), [Call::Stop, Call::Restart]);
}

#[tokio::test(start_paused = true)]
async fn a_flash_and_silence_delivered_together_do_not_restart_widgets() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.publish(flashing(0));
    listener.display.clear();
    settle().await;
    tokio::time::advance(REPLACEMENT_GRACE * 2).await;
    settle().await;

    assert_eq!(listener.widgets.calls(), [Call::Stop]);
}

#[tokio::test(start_paused = true)]
async fn a_replacement_run_ready_together_with_the_lapse_keeps_widgets_stopped() {
    let listener = listen(StopBehaviour::Immediate);
    listener.display.publish(downloading(0));
    settle().await;
    listener.display.clear();
    settle().await;
    // `advance` moves the clock before it yields, so the driver first wakes
    // with the replacement queued and the deadline already passed.
    listener.display.publish(flashing(1));
    tokio::time::advance(REPLACEMENT_GRACE).await;
    settle().await;

    assert_eq!(
        listener.widgets.calls(),
        [Call::Stop],
        "a queued snapshot must be stepped before the lapse is judged"
    );
    assert_eq!(listener.acknowledged(), Some(generation(1)));
}

#[tokio::test(start_paused = true)]
async fn a_run_that_fails_before_the_listener_wakes_leaves_widgets_alone() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(downloading(0));
    listener.display.publish(failed(0, UpgradeKind::Firmware));
    settle().await;

    assert_eq!(listener.widgets.calls(), []);
}

#[tokio::test(start_paused = true)]
async fn a_retry_after_a_handled_failure_restarts_and_stops_widgets_again() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.publish(failed(0, UpgradeKind::Firmware));
    settle().await;
    listener.display.publish(downloading(1));
    settle().await;

    assert_eq!(
        listener.widgets.calls(),
        [Call::Stop, Call::Restart, Call::Stop]
    );
    assert_eq!(listener.acknowledged(), Some(generation(1)));
}

#[tokio::test(start_paused = true)]
async fn a_retry_queued_with_its_failure_keeps_widgets_stopped() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(downloading(0));
    settle().await;
    listener.display.publish(failed(0, UpgradeKind::Firmware));
    listener.display.publish(downloading(1));
    settle().await;

    assert_eq!(
        listener.widgets.calls(),
        [Call::Stop],
        "respawning widgets only to stop them again ahead of a download defeats the pause"
    );
    assert_eq!(listener.acknowledged(), Some(generation(1)));
}

#[tokio::test(start_paused = true)]
async fn package_runs_and_the_post_reboot_overlay_never_touch_widgets() {
    let listener = listen(StopBehaviour::Immediate);

    listener.display.publish(running(
        0,
        UpgradeKind::Packages,
        Some(UpgradePhase::PackageRealizing),
    ));
    settle().await;
    listener
        .display
        .publish(succeeded(0, UpgradeKind::Packages));
    settle().await;
    listener
        .display
        .publish(succeeded(1, UpgradeKind::Firmware));
    settle().await;

    assert_eq!(listener.widgets.calls(), []);
}

#[tokio::test(start_paused = true)]
async fn a_crash_during_a_pause_never_restarts_widgets() {
    let mut listener = listen(StopBehaviour::Panics);

    listener.display.publish(downloading(0));
    settle().await;
    assert!(
        listener.acknowledged.changed().await.is_err(),
        "a waiting self-managed run must see the listener die instead of hanging"
    );
    tokio::time::advance(Duration::from_hours(24)).await;
    settle().await;
    assert_eq!(
        listener.widgets.calls(),
        [Call::Stop],
        "a dead listener cannot tell whether a flash is under way"
    );
}

#[tokio::test(start_paused = true)]
async fn a_new_run_during_a_held_restart_is_acknowledged_only_after_its_stop() {
    let display = RunStatusService::new();
    let widgets = ScriptedLifecycle::holding_restarts(StopBehaviour::Immediate);
    let acknowledged = spawn(
        display
            .take_events()
            .expect("BUG: a new run status service still holds its events"),
        Arc::clone(&widgets) as Arc<dyn WidgetLifecycle>,
    );
    display.publish(downloading(0));
    settle().await;
    display.publish(failed(0, UpgradeKind::Firmware));
    settle().await;

    display.publish(downloading(1));
    settle().await;
    assert_eq!(widgets.calls(), [Call::Stop, Call::Restart]);
    assert_eq!(
        *acknowledged.borrow(),
        None,
        "widgets are respawning; the download must wait"
    );

    widgets.release_restart();
    settle().await;
    assert_eq!(widgets.calls(), [Call::Stop, Call::Restart, Call::Stop]);
    assert_eq!(*acknowledged.borrow(), Some(generation(1)));
}
