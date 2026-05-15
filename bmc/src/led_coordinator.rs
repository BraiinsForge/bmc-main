// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED output arbiter: picks the topmost filled layer across system,
//! preview, and widget producers and emits a single `LedCommand` to the
//! SPI worker, gated by an enable flag.

use tokio::sync::mpsc;

use bmc_led::data::{LedCommand, LedEffect, LedScene};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    System = 0,
    Preview = 1,
    Widgets = 2,
}

#[derive(Debug)]
pub struct LedCoordinator {
    layers: [Option<LedScene>; 3],
    enabled: bool,
    led_tx: mpsc::Sender<LedCommand>,
    applied: Option<LedScene>,
    output_enabled: bool,
}

#[derive(Debug)]
enum CoordinatorCmd {
    Publish {
        layer: Layer,
        scene: Option<LedScene>,
    },
    SetEnabled(bool),
}

#[derive(Clone, Debug)]
pub struct LedCoordinatorHandle {
    tx: mpsc::UnboundedSender<CoordinatorCmd>,
}

impl LedCoordinatorHandle {
    pub fn publish(&self, layer: Layer, scene: Option<LedScene>) {
        let _ = self.tx.send(CoordinatorCmd::Publish { layer, scene });
    }

    pub fn set_enabled(&self, enabled: bool) {
        let _ = self.tx.send(CoordinatorCmd::SetEnabled(enabled));
    }
}

impl LedCoordinator {
    pub(crate) fn new(led_tx: mpsc::Sender<LedCommand>) -> Self {
        Self {
            layers: [None, None, None],
            enabled: true,
            led_tx,
            applied: None,
            output_enabled: true,
        }
    }

    pub(crate) fn publish(&mut self, layer: Layer, scene: Option<LedScene>) {
        self.layers[layer as usize] = scene;
        self.refresh();
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.refresh();
    }

    fn refresh(&mut self) {
        match (self.enabled, self.output_enabled) {
            (false, true) => {
                let _ = self.led_tx.try_send(LedCommand::Disable);
                self.output_enabled = false;
                // `applied` is preserved on purpose: when re-enabled, we
                // re-emit the topmost scene to restart the animation phase
                // visibly.
                return;
            }
            (true, false) => {
                let _ = self.led_tx.try_send(LedCommand::Enable);
                self.output_enabled = true;
                // fall through and re-emit the current pick, forcing a
                // SetEffect even if the dedupe would otherwise suppress it.
                self.applied = None;
            }
            _ => {}
        }
        if !self.enabled {
            return;
        }
        let pick = self.layers.iter().find_map(|l| *l);
        if pick == self.applied {
            return;
        }
        match pick {
            Some(scene) => {
                let _ = self.led_tx.try_send(LedCommand::SetEffect(scene));
            }
            None => {
                // No layer wants the strip. Only emit a clearing SetEffect if
                // we previously had one applied — first-boot ID state must
                // not spuriously emit SetEffect(None).
                if self.applied.is_some() {
                    let _ = self.led_tx.try_send(LedCommand::SetEffect(LedScene {
                        effect: LedEffect::None,
                        period: None,
                        duration: None,
                    }));
                }
            }
        }
        self.applied = pick;
    }
}

#[must_use]
pub fn spawn_led_coordinator(led_tx: mpsc::Sender<LedCommand>) -> LedCoordinatorHandle {
    let (tx, mut rx) = mpsc::unbounded_channel::<CoordinatorCmd>();
    tokio::spawn(async move {
        let mut coord = LedCoordinator::new(led_tx);
        while let Some(cmd) = rx.recv().await {
            match cmd {
                CoordinatorCmd::Publish { layer, scene } => coord.publish(layer, scene),
                CoordinatorCmd::SetEnabled(enabled) => coord.set_enabled(enabled),
            }
        }
    });
    LedCoordinatorHandle { tx }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_led::data::Rgb;

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
        coord.publish(Layer::Widgets, Some(scene(RED)));
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn preview_wins_over_widgets() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::Widgets, Some(scene(RED)));
        coord.publish(Layer::Preview, Some(scene(WHITE)));
        assert_eq!(
            drain(&mut rx),
            vec![
                LedCommand::SetEffect(scene(RED)),
                LedCommand::SetEffect(scene(WHITE)),
            ]
        );
        assert_eq!(coord.layers[Layer::Widgets as usize], Some(scene(RED)));
    }

    #[test]
    fn system_wins_over_preview_and_widgets() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::Widgets, Some(scene(RED)));
        coord.publish(Layer::Preview, Some(scene(WHITE)));
        coord.publish(Layer::System, Some(scene(GREEN)));
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
        coord.publish(Layer::Widgets, Some(scene(RED)));
        coord.publish(Layer::Preview, Some(scene(WHITE)));
        let _ = drain(&mut rx);
        coord.publish(Layer::Preview, None);
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn disable_emits_disable_and_does_not_emit_set_effect() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::Widgets, Some(scene(RED)));
        let _ = drain(&mut rx);
        coord.set_enabled(false);
        let emitted = drain(&mut rx);
        assert_eq!(emitted, vec![LedCommand::Disable]);
    }

    #[test]
    fn enable_after_disable_re_emits_top_scene() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::Widgets, Some(scene(RED)));
        let _ = drain(&mut rx);
        coord.set_enabled(false);
        coord.set_enabled(true);
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
        coord.publish(Layer::Widgets, Some(scene(RED)));
        coord.publish(Layer::Widgets, Some(scene(RED)));
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn dedupe_same_scene_across_layers() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::Widgets, Some(scene(RED)));
        coord.publish(Layer::Preview, Some(scene(RED)));
        assert_eq!(drain(&mut rx), vec![LedCommand::SetEffect(scene(RED))]);
    }

    #[test]
    fn dedupe_persists_across_enable_cycle() {
        let (mut coord, mut rx) = harness();
        coord.publish(Layer::Widgets, Some(scene(RED)));
        coord.set_enabled(false);
        coord.set_enabled(true);
        let _ = drain(&mut rx);
        coord.publish(Layer::Widgets, Some(scene(RED)));
        assert!(drain(&mut rx).is_empty());
    }
}
