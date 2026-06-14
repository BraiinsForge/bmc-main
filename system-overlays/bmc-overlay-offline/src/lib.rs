// Copyright (C) 2026  Braiins Systems s.r.o.

//! Bottom-right "OFFLINE" indicator. Mapped only while the device has no
//! routable IPv4; unmaps when connectivity returns and remaps if it drops.

use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_render::tree::{FontFamily, FontWeight, TextAlign, TextStyle, VerticalAlign};
use bmc_system_overlay::{LayerConfig, SystemOverlay, TickOutcome, primary_ipv4};

/// Surface size in logical pixels. The visible indicator is a content-tight box
/// drawn at the surface's bottom-right corner; the remainder stays transparent.
const SIZE: (u32, u32) = (160, 48);
/// Legacy display text; keep capital case.
const LABEL: &str = "OFFLINE";
/// Label font size in logical pixels.
const FONT_PX: u32 = 16;
/// Horizontal padding around the label (8px outer + 16px inner item padding).
const PAD_X: f32 = 24.0;
/// Vertical padding around the label.
const PAD_Y: f32 = 8.0;
/// Opaque black indicator background.
const BACKGROUND_RGBA: (u8, u8, u8, u8) = (0, 0, 0, 255);
/// Red label text (palette red-50).
const TEXT_RGBA: (u8, u8, u8, u8) = (249, 83, 85, 255);
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
        let (w, h, font) = (size.0 as f32, size.1 as f32, FONT_PX as f32);

        let (t_r, t_g, t_b, t_a) = TEXT_RGBA;
        let style = TextStyle {
            size: FONT_PX,
            color: Color::from_rgba(t_r, t_g, t_b, t_a),
            weight: FontWeight::SEMIBOLD,
            align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            family: FontFamily::Sans,
            ..TextStyle::default()
        };

        let box_w = r.measure_text(LABEL, font) + PAD_X * 2.0;
        let box_h = font * style.line_height + PAD_Y * 2.0;
        let box_x = w - box_w;
        let box_y = h - box_h;

        let (bg_r, bg_g, bg_b, bg_a) = BACKGROUND_RGBA;
        r.fill_rect(
            box_x,
            box_y,
            box_w,
            box_h,
            Color::from_rgba(bg_r, bg_g, bg_b, bg_a),
        );
        r.draw_canvas_text(LABEL, box_x + box_w / 2.0, box_y + box_h / 2.0, &style);
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
        assert_eq!(LABEL, "OFFLINE");
        assert_eq!(FONT_PX, 16);

        // Legacy: opaque black box with red text.
        assert_eq!(BACKGROUND_RGBA, (0, 0, 0, 255));
        let (red, green, blue, alpha) = TEXT_RGBA;
        assert!(red > green);
        assert!(red > blue);
        assert_eq!(alpha, u8::MAX);
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
