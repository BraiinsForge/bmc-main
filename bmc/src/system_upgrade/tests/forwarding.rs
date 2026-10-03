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
use crate::system_upgrade::widget_pause::{
    self,
    test_support::{Call, ScriptedLifecycle, StopBehaviour, settle},
};

struct Forwarding {
    run_gate: Arc<Mutex<()>>,
    display: RunStatusService,
    state: StateService,
    input: Option<tokio::sync::mpsc::UnboundedSender<UpgradeRunState>>,
    output: UpgradeRunStream,
}

fn forward(display: RunStatusService, kind: UpgradeKind) -> Forwarding {
    let run_gate = Arc::new(Mutex::new(()));
    let gate = Arc::clone(&run_gate)
        .try_lock_owned()
        .expect("BUG: fresh gate is lockable");
    let state = StateService::new();
    let (input, input_rx) = tokio::sync::mpsc::unbounded_channel();
    let generation = display.next_generation();
    let output = forward_upgrade_events(
        state.clone(),
        display.clone(),
        gate,
        generation,
        None,
        kind,
        UpgradeRunStream { rx: input_rx },
    );
    Forwarding {
        run_gate,
        display,
        state,
        input: Some(input),
        output,
    }
}

impl Forwarding {
    fn send(&self, state: UpgradeRunState) {
        self.input
            .as_ref()
            .expect("BUG: the run has not ended yet")
            .send(state)
            .expect("BUG: the forwarder reads until the run ends");
    }

    /// Ends the run the way a finished or unwound run task does:
    /// by dropping its sender.
    fn end_run(&mut self) {
        self.input = None;
    }

    async fn drain(&mut self) {
        while self.output.next().await.is_some() {}
    }

    fn shown(&self) -> Option<UpgradeRunStatus> {
        self.display
            .subscribe()
            .borrow()
            .clone()
            .map(|snapshot| snapshot.state)
    }
}

#[tokio::test]
async fn a_failed_run_keeps_the_gate_until_its_run_ends() {
    let mut forwarding = forward(RunStatusService::new(), UpgradeKind::Firmware);
    forwarding.send(UpgradeRunState::Phase(UpgradePhase::FirmwareDownloading));
    forwarding.send(UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed));
    assert_eq!(
        forwarding.output.next().await,
        Some(UpgradeRunState::Phase(UpgradePhase::FirmwareDownloading))
    );
    assert_eq!(
        forwarding.output.next().await,
        Some(UpgradeRunState::Failed(SystemUpgradeError::UpgradeFailed))
    );

    assert_eq!(
        forwarding.shown(),
        Some(UpgradeRunStatus::Failed {
            kind: UpgradeKind::Firmware,
            reason: SystemUpgradeError::UpgradeFailed.to_string(),
        })
    );
    assert!(
        forwarding.run_gate.try_lock().is_err(),
        "a run claimed while this one may still publish would have its Running overwritten"
    );

    forwarding.end_run();
    forwarding.drain().await;
    let _next_run = tokio::time::timeout(Duration::from_secs(1), forwarding.run_gate.lock())
        .await
        .expect("an ended run must let the next upgrade start");
}

#[tokio::test]
async fn a_finished_packages_run_frees_the_gate() {
    let mut forwarding = forward(RunStatusService::new(), UpgradeKind::Packages);
    forwarding.send(UpgradeRunState::Finished);
    forwarding.end_run();
    forwarding.drain().await;

    let _next_run = tokio::time::timeout(Duration::from_secs(1), forwarding.run_gate.lock())
        .await
        .expect("a finished run must let the next upgrade start");
}

#[tokio::test(start_paused = true)]
async fn a_run_ending_after_the_handoff_keeps_the_gate_and_its_flashing_display() {
    let mut forwarding = forward(RunStatusService::new(), UpgradeKind::Firmware);
    forwarding.send(UpgradeRunState::Phase(UpgradePhase::FirmwareApplying));
    forwarding.end_run();
    forwarding.drain().await;
    tokio::time::advance(Duration::from_hours(24)).await;
    settle().await;

    assert!(
        forwarding.run_gate.try_lock().is_err(),
        "nothing may start between the sysupgrade handoff and the reboot"
    );
    assert_eq!(
        forwarding.shown(),
        Some(UpgradeRunStatus::Running {
            kind: UpgradeKind::Firmware,
            phase: Some(UpgradePhase::FirmwareApplying),
            progress: None,
        })
    );
}

#[tokio::test]
async fn a_run_ending_without_an_outcome_clears_its_display_and_frees_the_gate() {
    let mut forwarding = forward(RunStatusService::new(), UpgradeKind::Firmware);
    forwarding.send(UpgradeRunState::Phase(UpgradePhase::FirmwareDownloading));
    forwarding.end_run();
    forwarding.drain().await;

    let _gate = tokio::time::timeout(Duration::from_secs(1), forwarding.run_gate.lock())
        .await
        .expect("an unwound run must not hold the gate");
    assert_eq!(
        forwarding.shown(),
        None,
        "no overlay may stay stuck on Running"
    );
    assert_eq!(*forwarding.state.subscribe().borrow(), None);
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_firmware_run_restarts_widgets_after_the_replacement_grace() {
    let display = RunStatusService::new();
    let widgets = ScriptedLifecycle::new(StopBehaviour::Immediate);
    let _acknowledged = widget_pause::spawn(
        display
            .take_events()
            .expect("BUG: a new run status service still holds its events"),
        Arc::clone(&widgets) as Arc<dyn WidgetLifecycle>,
    );
    let mut forwarding = forward(display, UpgradeKind::Firmware);
    forwarding.send(UpgradeRunState::Phase(UpgradePhase::FirmwareDownloading));
    settle().await;
    forwarding.end_run();
    forwarding.drain().await;
    settle().await;
    assert_eq!(widgets.calls(), [Call::Stop]);

    tokio::time::advance(widget_pause::REPLACEMENT_GRACE).await;
    settle().await;
    assert_eq!(widgets.calls(), [Call::Stop, Call::Restart]);
}
