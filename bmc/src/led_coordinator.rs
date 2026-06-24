// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED output arbiter: picks the topmost filled layer across system,
//! preview, and widget producers and emits a single `LedCommand` to the
//! SPI worker, gated by an enable flag.

use tokio::sync::{mpsc, watch};

use bmc_led::data::{LedCommand, LedEffect, LedScene};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// System indicators (wifi, alarm, upgrade) — always wins.
    System = 0,
    /// gRPC preview-scene LED.
    Preview = 1,
    /// Widget-driven effects scoped to the currently active scene.
    LocalScene = 2,
    /// Widget-driven ambient effects with no scene affinity.
    GlobalAmbient = 3,
}

impl Layer {
    /// Layers in priority order, highest first. The single source of truth
    /// for how many layers exist — every per-layer array sizes off
    /// `ALL.len()`, so adding a variant here resizes them in lockstep.
    const ALL: [Layer; 4] = [
        Layer::System,
        Layer::Preview,
        Layer::LocalScene,
        Layer::GlobalAmbient,
    ];
}

// Guard the `layer as usize` indexing: `ALL` must list every variant in
// discriminant order so an index derived from any layer stays in bounds.
// Adding a variant without extending `ALL` fails here instead of panicking
// at runtime.
const _: () = {
    let mut i = 0;
    while i < Layer::ALL.len() {
        assert!(Layer::ALL[i] as usize == i);
        i += 1;
    }
};

#[derive(Debug)]
pub struct LedCoordinator {
    layers: [Option<LedScene>; Layer::ALL.len()],
    enabled: bool,
    led_tx: mpsc::Sender<LedCommand>,
    /// Last `(Layer, LedScene)` we sent on the wire. Keyed by layer too
    /// so the layer change between, say, `Widgets→Solid(red)` and
    /// `System→Solid(red)` re-emits a fresh `SetEffect`: animation
    /// phase is owned by the SPI worker and only restarts on a new
    /// command, so a layer-transition that happens to carry the same
    /// scene must still cross the wire.
    applied: Option<(Layer, LedScene)>,
    output_enabled: bool,
}

#[derive(Clone, Copy, Debug)]
struct DesiredState {
    layers: [Option<LedScene>; Layer::ALL.len()],
    enabled: bool,
}

impl Default for DesiredState {
    fn default() -> Self {
        Self {
            layers: [None; Layer::ALL.len()],
            enabled: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LedCoordinatorHandle {
    tx: watch::Sender<DesiredState>,
}

impl LedCoordinatorHandle {
    pub fn publish(&self, layer: Layer, scene: Option<LedScene>) {
        self.tx.send_modify(|desired| {
            desired.layers[layer as usize] = scene;
        });
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.tx.send_modify(|desired| {
            desired.enabled = enabled;
        });
    }
}

impl LedCoordinator {
    pub(crate) fn new(led_tx: mpsc::Sender<LedCommand>) -> Self {
        Self {
            layers: [None; Layer::ALL.len()],
            enabled: true,
            led_tx,
            applied: None,
            output_enabled: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn publish(&mut self, layer: Layer, scene: Option<LedScene>) {
        self.layers[layer as usize] = scene;
    }

    #[cfg(test)]
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn apply_desired(&mut self, desired: DesiredState) {
        self.layers = desired.layers;
        self.enabled = desired.enabled;
    }

    fn pick(&self) -> Option<(Layer, LedScene)> {
        Layer::ALL
            .into_iter()
            .find_map(|layer| self.layers[layer as usize].map(|scene| (layer, scene)))
    }

    /// Next command to send, paired with the pick it was derived from so
    /// `on_sent` records the originating layer without re-running `pick()`
    /// against state that may have moved on. `None` pick rides with the
    /// Enable/Disable and clear commands, which don't key on a layer.
    fn next_command(&self) -> Option<(LedCommand, Option<(Layer, LedScene)>)> {
        match (self.enabled, self.output_enabled) {
            (false, true) => return Some((LedCommand::Disable, None)),
            (true, false) => return Some((LedCommand::Enable, None)),
            _ => {}
        }
        if !self.enabled {
            return None;
        }
        let pick = self.pick();
        if pick == self.applied {
            return None;
        }
        match pick {
            Some((_, scene)) => Some((LedCommand::SetEffect(scene), pick)),
            None if self.applied.is_some() => Some((
                LedCommand::SetEffect(LedScene {
                    effect: LedEffect::None,
                    period: None,
                    duration: None,
                }),
                None,
            )),
            None => None,
        }
    }

    fn on_sent(&mut self, command: LedCommand, pick: Option<(Layer, LedScene)>) {
        match command {
            LedCommand::Disable => {
                self.output_enabled = false;
            }
            LedCommand::Enable => {
                self.output_enabled = true;
                // Force re-emission of the topmost scene when re-enabled to
                // restart animation phase visibly.
                self.applied = None;
            }
            LedCommand::SetEffect(scene) => {
                self.applied = if matches!(scene.effect, LedEffect::None) {
                    None
                } else {
                    pick
                };
            }
            LedCommand::SetBrightness(_) => {}
        }
    }
}

#[must_use]
pub fn spawn_led_coordinator(led_tx: mpsc::Sender<LedCommand>) -> LedCoordinatorHandle {
    let (tx, mut rx) = watch::channel(DesiredState::default());
    tokio::spawn(async move {
        let mut coord = LedCoordinator::new(led_tx);
        loop {
            coord.apply_desired(*rx.borrow_and_update());

            while let Some((command, pick)) = coord.next_command() {
                // Recompute from the latest desired state before sending so
                // stale queued intents are coalesced (latest-wins semantics).
                if rx.has_changed().unwrap_or(false) {
                    coord.apply_desired(*rx.borrow_and_update());
                    continue;
                }
                if coord.led_tx.send(command).await.is_err() {
                    return;
                }
                coord.on_sent(command, pick);
                coord.apply_desired(*rx.borrow_and_update());
            }

            if rx.changed().await.is_err() {
                break;
            }
        }
    });
    LedCoordinatorHandle { tx }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_led::data::Rgb;
    use tokio::time::{Duration, timeout};

    fn scene(rgb: Rgb) -> LedScene {
        LedScene {
            effect: LedEffect::Solid(rgb),
            period: None,
            duration: None,
        }
    }

    fn drain(rx: &mut mpsc::Receiver<LedCommand>) -> Vec<LedCommand> {
        let mut out = Vec::new();
        while let Ok(c) = rx.try_recv() {
            out.push(c);
        }
        out
    }

    fn harness() -> (LedCoordinator, mpsc::Receiver<LedCommand>) {
        let (tx, rx) = mpsc::channel(16);
        (LedCoordinator::new(tx), rx)
    }

    fn settle(coord: &mut LedCoordinator) {
        while let Some((command, pick)) = coord.next_command() {
            coord
                .led_tx
                .try_send(command)
                .expect("BUG: test queue should have capacity");
            coord.on_sent(command, pick);
        }
    }

    const RED: Rgb = Rgb::new(255, 0, 0);
    const WHITE: Rgb = Rgb::new(255, 255, 255);
    const GREEN: Rgb = Rgb::new(0, 255, 0);

    #[test]
    fn empty_coordinator_emits_nothing() {
        let (_coord, mut rx) = harness();
        assert!(drain(&mut rx).is_empty());
    }

    #[test]
    fn publish_widgets_only_applies_that_scene() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn preview_wins_over_widgets() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        coord.publish(Layer::Preview, Some(scene(WHITE)));
        settle(&mut coord);
        assert_eq!(
            drain(&mut rx),
            vec![
                LedCommand::SetEffect(scene(RED)),
                LedCommand::SetEffect(scene(WHITE)),
            ]
        );
        assert_eq!(coord.layers[Layer::LocalScene as usize], Some(scene(RED)));
    }

    #[test]
    fn system_wins_over_preview_and_widgets() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        coord.publish(Layer::Preview, Some(scene(WHITE)));
        settle(&mut coord);
        coord.publish(Layer::System, Some(scene(GREEN)));
        settle(&mut coord);
        let emitted = drain(&mut rx);
        assert_eq!(
            emitted.last(),
            Some(&LedCommand::SetEffect(scene(GREEN))),
            "system layer must drive the strip when filled"
        );
    }

    #[test]
    fn clearing_preview_falls_through_to_widgets() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        coord.publish(Layer::Preview, Some(scene(WHITE)));
        settle(&mut coord);
        let _ = drain(&mut rx);
        coord.publish(Layer::Preview, None);
        settle(&mut coord);
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn disable_emits_disable_and_does_not_emit_set_effect() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        let _ = drain(&mut rx);
        coord.set_enabled(false);
        settle(&mut coord);
        let emitted = drain(&mut rx);
        assert_eq!(emitted, vec![LedCommand::Disable]);
    }

    #[test]
    fn enable_after_disable_re_emits_top_scene() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        let _ = drain(&mut rx);
        coord.set_enabled(false);
        settle(&mut coord);
        coord.set_enabled(true);
        settle(&mut coord);
        assert_eq!(
            drain(&mut rx),
            vec![
                LedCommand::Disable,
                LedCommand::Enable,
                LedCommand::SetEffect(scene(RED)),
            ]
        );
    }

    #[test]
    fn dedupe_same_scene_on_same_layer() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn layer_change_with_same_scene_re_emits() {
        // The previous bug deduped purely on scene equality and would
        // swallow the layer transition, leaving the SPI worker at the
        // animation phase of the old layer. The new key is `(layer,
        // scene)` so a layer change always produces a fresh SetEffect.
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        coord.publish(Layer::Preview, Some(scene(RED)));
        settle(&mut coord);
        assert_eq!(
            drain(&mut rx),
            vec![
                LedCommand::SetEffect(scene(RED)),
                LedCommand::SetEffect(scene(RED)),
            ]
        );
    }

    #[test]
    fn dedupe_reestablished_after_enable_cycle() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        coord.set_enabled(false);
        settle(&mut coord);
        coord.set_enabled(true);
        settle(&mut coord);
        let _ = drain(&mut rx);
        coord.publish(Layer::LocalScene, Some(scene(RED)));
        settle(&mut coord);
        assert!(drain(&mut rx).is_empty());
    }

    #[tokio::test]
    async fn queued_command_is_eventually_delivered_after_backpressure_relief() {
        let (led_tx, mut led_rx) = mpsc::channel(1);
        led_tx
            .send(LedCommand::SetEffect(scene(GREEN)))
            .await
            .expect("BUG: initial queue fill should succeed");
        let handle = spawn_led_coordinator(led_tx);

        handle.publish(Layer::LocalScene, Some(scene(RED)));

        let drained = timeout(Duration::from_millis(100), led_rx.recv())
            .await
            .expect("BUG: pre-filled command should still be readable");
        assert_eq!(drained, Some(LedCommand::SetEffect(scene(GREEN))));

        let delivered = timeout(Duration::from_millis(500), led_rx.recv())
            .await
            .expect("BUG: coordinator should retry once queue has room");
        assert_eq!(delivered, Some(LedCommand::SetEffect(scene(RED))));
    }

    #[tokio::test]
    async fn latest_desired_scene_wins_while_output_queue_is_blocked() {
        let (led_tx, mut led_rx) = mpsc::channel(1);
        led_tx
            .send(LedCommand::SetEffect(scene(GREEN)))
            .await
            .expect("BUG: initial queue fill should succeed");
        let handle = spawn_led_coordinator(led_tx);

        handle.publish(Layer::LocalScene, Some(scene(RED)));
        handle.publish(Layer::LocalScene, Some(scene(WHITE)));

        let drained = timeout(Duration::from_millis(100), led_rx.recv())
            .await
            .expect("BUG: pre-filled command should still be readable");
        assert_eq!(drained, Some(LedCommand::SetEffect(scene(GREEN))));

        let first = timeout(Duration::from_millis(500), led_rx.recv())
            .await
            .expect("BUG: coordinator should deliver a coalesced scene");
        assert_eq!(first, Some(LedCommand::SetEffect(scene(WHITE))));

        let no_second = timeout(Duration::from_millis(100), led_rx.recv()).await;
        assert!(no_second.is_err(), "no stale command should follow");
    }
}
