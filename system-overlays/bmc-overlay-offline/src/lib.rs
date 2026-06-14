// Copyright (C) 2026  Braiins Systems s.r.o.

//! Bottom-right "OFFLINE" indicator. Mapped only while the device has no
//! routable IPv4; unmaps when connectivity returns and remaps if it drops.

use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_system_overlay::{LayerConfig, SystemOverlay, TickOutcome, primary_ipv4};

/// Indicator size in logical pixels.
const SIZE: (u32, u32) = (200, 56);
/// Legacy display text; keep capital case.
const LABEL: &str = "OFFLINE";
/// Red see-through background.
const BACKGROUND_RGBA: (u8, u8, u8, u8) = (180, 0, 0, 160);
/// Connectivity re-check cadence.
const POLL: Duration = Duration::from_secs(2);

/// Injected connectivity source for testing.
trait Env {
    fn online(&self) -> bool;
}

struct OsEnv;
impl Env for OsEnv {
    fn online(&self) -> bool {
        primary_ipv4().is_some()
    }
}

/// Pure: given current online state and last-rendered visibility, decide the
/// next `(visible, wants_render)`.
fn decide(online: bool, was_visible: bool) -> (bool, bool) {
    let visible = !online;
    let wants_render = visible && !was_visible;
    (visible, wants_render)
}

pub struct OfflineOverlay {
    visible: bool,
    online: bool,
    last_probe: Option<Instant>,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for OfflineOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineOverlay")
            .field("visible", &self.visible)
            .field("online", &self.online)
            .finish_non_exhaustive()
    }
}

impl Default for OfflineOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            online: true,
            last_probe: None,
            env: Box::new(OsEnv),
        }
    }
}

impl OfflineOverlay {
    fn probe_if_due(&mut self, now: Instant) {
        if self
            .last_probe
            .is_some_and(|last| now.duration_since(last) < POLL)
        {
            return;
        }

        self.last_probe = Some(now);
        self.online = self.env.online();
    }
}

impl SystemOverlay for OfflineOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig::bottom_right("bmc-overlay-offline", SIZE)
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        self.probe_if_due(now);
        let (visible, wants_render) = decide(self.online, self.visible);
        self.visible = visible;
        TickOutcome {
            visible,
            wants_render,
            next_wake: Some(now + POLL),
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        #[expect(
            clippy::cast_precision_loss,
            reason = "indicator dimensions fit comfortably in f32 mantissa"
        )]
        let (w, h) = (size.0 as f32, size.1 as f32);
        let (bg_r, bg_g, bg_b, bg_a) = BACKGROUND_RGBA;
        r.fill_rounded_rect(
            0.0,
            0.0,
            w,
            h,
            12.0,
            Color::from_rgba(bg_r, bg_g, bg_b, bg_a),
        );
        let text = LABEL;
        let font = 28.0;
        let tw = r.measure_text(text, font);
        r.draw_text(
            text,
            (w - tw) / 2.0,
            h / 2.0 + font / 3.0,
            font,
            Color::from_rgba(255, 255, 255, 255),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct CountingEnv {
        calls: Rc<Cell<usize>>,
        online: bool,
    }

    impl Env for CountingEnv {
        fn online(&self) -> bool {
            self.calls.set(self.calls.get() + 1);
            self.online
        }
    }

    #[test]
    fn offline_maps_and_renders_on_transition() {
        assert_eq!(decide(false, false), (true, true));
    }

    #[test]
    fn offline_stays_mapped_without_extra_render() {
        assert_eq!(decide(false, true), (true, false));
    }

    #[test]
    fn online_unmaps() {
        assert_eq!(decide(true, true), (false, false));
    }

    #[test]
    fn constants_match_legacy_offline_indicator() {
        let (red, green, blue, alpha) = BACKGROUND_RGBA;

        assert_eq!(LABEL, "OFFLINE");
        assert!(red > green);
        assert!(red > blue);
        assert!(alpha > 0);
        assert!(alpha < u8::MAX);
    }

    #[test]
    fn tick_reuses_cached_probe_between_poll_intervals() {
        let start = Instant::now();
        let calls = Rc::new(Cell::new(0));
        let mut overlay = OfflineOverlay {
            visible: false,
            online: true,
            last_probe: None,
            env: Box::new(CountingEnv {
                calls: Rc::clone(&calls),
                online: false,
            }),
        };

        let _ = overlay.tick(start);
        let _ = overlay.tick(start + Duration::from_millis(500));

        assert_eq!(calls.get(), 1);
    }
}
