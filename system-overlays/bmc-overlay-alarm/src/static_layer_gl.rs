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

use bmc_render::renderer::test_support::ShapingRecorder;
use bmc_system_overlay::gl_check::{
    CAPTURED, GlRun, REUSED, assert_layered_run_matches_full_paint,
    assert_moving_damage_matches_full_paint,
};

use super::*;

const SIZE: (u32, u32) = (1_280, 480);

fn view() -> AlarmView {
    AlarmView {
        time: "7:30".to_owned(),
        period: "AM".to_owned(),
        label: "Wake up".to_owned(),
        snooze_allowed: true,
    }
}

fn paint(run: &mut GlRun<'_>, state: &mut AlarmRenderState, kind: FrameKind) {
    let view = view();
    run.paint(
        kind,
        state,
        |state| Some(&mut state.tree),
        |state, renderer, frame| {
            render_alarm(
                renderer,
                SIZE,
                state,
                &view,
                &bmc_intl::formatter(bmc_intl::Language::default()),
                frame,
            )
            .outcome
        },
    );
}

/// The centre of the stop button, where a recorded frame laid it out.
fn stop_button() -> (f64, f64) {
    let mut state = AlarmRenderState::new(Instant::now());
    let _ = render_alarm(
        &mut ShapingRecorder::new(1_280.0, 480.0),
        SIZE,
        &mut state,
        &view(),
        &bmc_intl::formatter(bmc_intl::Language::default()),
        OverlayFrame::unlayered(FrameKind::Fresh),
    );
    let button = state
        .tree
        .element_bounds(STOP_ALARM_KEY)
        .expect("BUG: the alarm draws its stop button");
    let (x, y) = button.centre();
    (f64::from(x), f64::from(y))
}

#[test]
fn a_stop_press_paints_as_a_full_paint_does() {
    let (x, y) = stop_button();
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let mut state = AlarmRenderState::new(run.now());
        let ringing = |run: &mut GlRun<'_>, state: &mut AlarmRenderState| {
            for _ in 0..6 {
                paint(run, state, FrameKind::AnimationOnly);
            }
        };
        paint(run, &mut state, FrameKind::Fresh);
        ringing(run, &mut state);
        state.tree.push_touch(TouchEvent::Down { id: 0, x, y });
        paint(run, &mut state, FrameKind::AnimationOnly);
        assert!(
            state.tree.is_pressed(STOP_ALARM_KEY),
            "the touch must land on stop"
        );
        ringing(run, &mut state);
        state.tree.push_touch(TouchEvent::Up { id: 0 });
        paint(run, &mut state, FrameKind::AnimationOnly);
        ringing(run, &mut state);
    });
    let Some(outcomes) = outcomes else { return };
    let ringing = [REUSED; 6];
    let capture = [CAPTURED].as_slice();
    assert_eq!(
        outcomes,
        [capture, &ringing, capture, &ringing, capture, &ringing,].concat(),
        "the press and the release each capture again"
    );
}

/// A square moved on every rebuild gives the alarm's replays damage that spans both buffers.
#[test]
fn moving_damage_on_the_alarm_paints_as_a_full_paint_does() {
    let intl = bmc_intl::formatter(bmc_intl::Language::default());
    let _ = assert_moving_damage_matches_full_paint(
        SIZE,
        |_| (),
        |()| build_alarm_ui_tree(&view(), SIZE, &intl),
    );
}
