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

use std::time::{Duration, Instant};

use bmc_render::renderer::test_support::ShapingRecorder;
use bmc_system_overlay::gl_check::{
    CAPTURED, FRAME_STEP, GlRun, MISSED, REUSED, UNCACHED, assert_layered_run_matches_full_paint,
};

use super::*;

const DECK: (u32, u32) = (1_280, 480);
const BMM101: (u32, u32) = (480, 320);
const HOLD: Duration = Duration::from_secs(5);
const FIRST_IP: &str = "10.0.0.42";

/// The tray at `ip`, its slider at `brightness`, `held` into a restart hold if any.
fn view(ip: &str, brightness: u8, held: Option<Duration>) -> SettingsTrayView {
    SettingsTrayView {
        status: held.map(|elapsed| ui::Status {
            action: ui::Action::Restart,
            phase: ui::Phase::Holding(ui::Hold::new(elapsed, HOLD)),
            reason: None,
        }),
        ip: Some(ip.to_owned()),
        brightness,
        ..SettingsTrayView::resting()
    }
}

/// Where a resting tray at `size` laid out the control keyed `key`.
#[expect(clippy::cast_precision_loss, reason = "display sizes are small")]
fn bounds_of(size: (u32, u32), key: &str) -> bmc_render::interaction::Rect {
    let mut state = SettingsTrayRenderState::new(Instant::now());
    let _ = render_settings_tray(
        &mut ShapingRecorder::new(size.0 as f32, size.1 as f32),
        OverlayViewport::rectangular(size.0, size.1),
        &mut state,
        &view(FIRST_IP, 50, None),
        &bmc_intl::formatter(bmc_intl::Language::default()),
        OverlayFrame::unlayered(FrameKind::Fresh),
    );
    state
        .tree
        .element_bounds(key)
        .unwrap_or_else(|| panic!("BUG: the tray lays out {key}"))
}

/// A point `fraction` across the BMM101 slider, which maps a touch to a value across its bounds.
fn on_track(fraction: f32) -> (f64, f64) {
    let slider = bounds_of(BMM101, ui::BRIGHTNESS_SLIDER_KEY);
    (
        f64::from(slider.x + slider.w * fraction),
        f64::from(slider.y + slider.h / 2.0),
    )
}

/// The centre of the Deck tray's restart button.
fn on_restart() -> (f64, f64) {
    let button = bounds_of(DECK, ui::RESTART_KEY);
    (
        f64::from(button.x + button.w / 2.0),
        f64::from(button.y + button.h / 2.0),
    )
}

/// The tray's render state and the view it draws, its frames stamped by the run's clock.
struct Tray {
    state: SettingsTrayRenderState,
    size: (u32, u32),
    ip: &'static str,
    brightness: u8,
    held: Option<Duration>,
}

impl Tray {
    fn new(run: &GlRun<'_>, size: (u32, u32)) -> Self {
        Self {
            state: SettingsTrayRenderState::new(run.now()),
            size,
            ip: FIRST_IP,
            brightness: 50,
            held: None,
        }
    }

    fn paint(&mut self, run: &mut GlRun<'_>, kind: FrameKind) {
        let view = view(self.ip, self.brightness, self.held);
        let viewport = OverlayViewport::rectangular(self.size.0, self.size.1);
        let state = &mut self.state;
        run.paint(kind, |renderer, frame| {
            render_settings_tray(
                renderer,
                viewport,
                state,
                &view,
                &bmc_intl::formatter(bmc_intl::Language::default()),
                frame,
            )
            .outcome
        });
    }

    /// Paint `frames` more of a restart hold at `(x, y)`, advancing the hold by hand
    /// in place of the overlay's hold FSM, the finger jittering a pixel up and down
    /// as a resting one does.
    fn hold_for(&mut self, run: &mut GlRun<'_>, frames: usize, (x, y): (f64, f64)) {
        for frame in 0..frames {
            let jitter = if frame % 2 == 0 { 1.0 } else { -1.0 };
            self.touch(TouchEvent::Motion {
                id: 0,
                x,
                y: y + jitter,
            });
            self.held = Some(self.held.unwrap_or_default() + FRAME_STEP);
            self.paint(run, FrameKind::Fresh);
        }
    }

    fn touch(&mut self, event: TouchEvent) {
        self.state.tree.push_touch(event);
    }

    fn pressed(&self, key: &str) -> bool {
        self.state.tree.is_pressed(key)
    }

    /// Lose the layer behind the tray's back, paint through the miss, and hide.
    fn lose_layer_and_hide(&mut self, run: &mut GlRun<'_>) {
        run.lose_layer(&self.state.tree);
        self.paint(run, FrameKind::Fresh);
        self.paint(run, FrameKind::Fresh);
        run.hide(&mut self.state.tree);
    }
}

#[test]
fn a_slider_drag_paints_as_a_full_paint_does() {
    let outcomes = assert_layered_run_matches_full_paint(BMM101, |run| {
        let mut tray = Tray::new(run, BMM101);
        for _ in 0..4 {
            tray.paint(run, FrameKind::Fresh);
        }
        let (x, y) = on_track(0.2);
        tray.touch(TouchEvent::Down { id: 0, x, y });
        tray.paint(run, FrameKind::Fresh);
        assert!(
            tray.pressed(ui::BRIGHTNESS_SLIDER_KEY),
            "the press lands on the slider"
        );
        for (fraction, brightness) in [(0.4, 40), (0.6, 60), (0.8, 80)] {
            let (x, y) = on_track(fraction);
            tray.touch(TouchEvent::Motion { id: 0, x, y });
            tray.brightness = brightness;
            tray.paint(run, FrameKind::Fresh);
            tray.paint(run, FrameKind::AnimationOnly);
        }
        assert!(
            tray.pressed(ui::BRIGHTNESS_SLIDER_KEY),
            "the drag keeps the slider held"
        );
        tray.touch(TouchEvent::Up { id: 0 });
        tray.paint(run, FrameKind::Fresh);
        assert!(
            !tray.pressed(ui::BRIGHTNESS_SLIDER_KEY),
            "the release lets go"
        );
        tray.ip = "10.0.0.43";
        for _ in 0..4 {
            tray.paint(run, FrameKind::Fresh);
        }
        let (x, y) = on_track(0.5);
        tray.touch(TouchEvent::Down { id: 0, x, y });
        tray.paint(run, FrameKind::Fresh);
        tray.paint(run, FrameKind::AnimationOnly);
        tray.state.tree.cancel_touch();
        tray.paint(run, FrameKind::AnimationOnly);
        assert!(
            !tray.pressed(ui::BRIGHTNESS_SLIDER_KEY),
            "the cancellation lets go"
        );
        tray.lose_layer_and_hide(run);
        for _ in 0..9 {
            tray.paint(run, FrameKind::Fresh);
        }
    });
    let Some(outcomes) = outcomes else { return };
    assert_eq!(
        outcomes,
        [
            [CAPTURED].as_slice(),
            &[REUSED; 3],
            // The press holds the slider, the drag only moves it, the release lets go.
            &[CAPTURED],
            &[REUSED; 6],
            &[CAPTURED],
            // The address changes.
            &[CAPTURED, REUSED, REUSED, REUSED],
            // A second press, cancelled on a replayed frame.
            &[CAPTURED, REUSED, CAPTURED],
            // The lost layer misses once, and the tray paints uncached until hidden.
            &[MISSED, UNCACHED],
            &[CAPTURED],
            &[REUSED; 8],
        ]
        .concat()
    );
}

#[test]
fn a_restart_hold_paints_as_a_full_paint_does() {
    let outcomes = assert_layered_run_matches_full_paint(DECK, |run| {
        let (x, y) = on_restart();
        let mut tray = Tray::new(run, DECK);
        for _ in 0..4 {
            tray.paint(run, FrameKind::Fresh);
        }
        tray.touch(TouchEvent::Down { id: 0, x, y });
        tray.paint(run, FrameKind::Fresh);
        assert!(tray.pressed(ui::RESTART_KEY), "the press lands on restart");
        tray.hold_for(run, 6, (x, y));
        tray.hold_for(run, 4, (x + 4.0, y));
        assert!(
            tray.pressed(ui::RESTART_KEY),
            "a move within the button keeps the hold"
        );
        tray.touch(TouchEvent::Up { id: 0 });
        tray.held = None;
        tray.paint(run, FrameKind::Fresh);
        assert!(!tray.pressed(ui::RESTART_KEY), "the release ends the hold");
        tray.touch(TouchEvent::Down { id: 0, x, y });
        tray.paint(run, FrameKind::Fresh);
        tray.hold_for(run, 4, (x, y));
        tray.state.tree.cancel_touch();
        tray.paint(run, FrameKind::AnimationOnly);
        assert!(
            !tray.pressed(ui::RESTART_KEY),
            "the cancellation ends the hold"
        );
        tray.held = None;
        tray.paint(run, FrameKind::Fresh);
    });
    let Some(outcomes) = outcomes else { return };
    assert_eq!(
        outcomes,
        [
            [CAPTURED].as_slice(),
            &[REUSED; 3],
            // The press, then the hold's notice, which its progress only moves.
            &[CAPTURED, CAPTURED],
            &[REUSED; 9],
            // The release clears the notice.
            &[CAPTURED],
            // A second hold, cancelled on a replayed frame, then cleared.
            &[
                CAPTURED, CAPTURED, REUSED, REUSED, REUSED, CAPTURED, CAPTURED
            ],
        ]
        .concat()
    );
}
