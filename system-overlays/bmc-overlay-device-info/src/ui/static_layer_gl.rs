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

use std::net::Ipv4Addr;

use bmc_system_overlay::gl_check::{
    CAPTURED, GlRun, REUSED, assert_layered_run_matches_full_paint,
};
use bmc_system_overlay::{AccessPoint, FrameKind, Language, TouchEvent};

use super::{
    DeviceInfoRenderState, DeviceInfoView, LanguageSwitch, Uplinks, language_key,
    offers_language_switch, render_device_info,
};

const SIZE: (u32, u32) = (1_280, 480);

fn paint(
    run: &mut GlRun<'_>,
    state: &mut DeviceInfoRenderState,
    view: &DeviceInfoView,
    kind: FrameKind,
) {
    paint_speaking(run, state, view, None, kind);
}

/// Paint `view` with the language switch marking `current`, if any.
fn paint_speaking(
    run: &mut GlRun<'_>,
    state: &mut DeviceInfoRenderState,
    view: &DeviceInfoView,
    current: Option<Language>,
    kind: FrameKind,
) {
    let switch = current.map_or(LanguageSwitch::Hidden, |current| LanguageSwitch::Shown {
        current: Some(current),
    });
    run.paint(kind, |renderer, frame| {
        render_device_info(
            renderer,
            SIZE,
            state,
            view,
            &bmc_intl::formatter(bmc_intl::Language::default()),
            "Braiins Deck",
            false,
            switch,
            frame,
        )
        .outcome
    });
}

fn screen(run: &mut GlRun<'_>, state: &mut DeviceInfoRenderState, view: &DeviceInfoView) {
    paint(run, state, view, FrameKind::Fresh);
    for _ in 0..9 {
        paint(run, state, view, FrameKind::AnimationOnly);
    }
}

#[test]
fn a_screen_change_paints_as_a_full_paint_does() {
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let mut state = DeviceInfoRenderState::new(run.now());
        screen(run, &mut state, &DeviceInfoView::TurningApOff);
        let success = DeviceInfoView::Success {
            ip: Ipv4Addr::new(10, 0, 0, 5),
        };
        screen(run, &mut state, &success);
        screen(run, &mut state, &DeviceInfoView::SetupCompleted);
    });
    let Some(outcomes) = outcomes else { return };
    let screen = [[CAPTURED].as_slice(), &[REUSED; 9]].concat();
    assert_eq!(outcomes, [screen.as_slice(), &screen, &screen].concat());
}

#[test]
fn a_flag_press_on_the_language_switch_paints_as_a_full_paint_does() {
    let offered = bmc_intl::Translations::shared().languages();
    let [spoken, picked, ..] = offered[..] else {
        panic!("BUG: the switch needs two languages to choose between, got {offered:?}");
    };
    let setup = DeviceInfoView::SetupStart {
        ap: Some(AccessPoint {
            ssid: "Deck setup".to_owned(),
            setup_url: "http://10.0.0.21/".to_owned(),
        }),
        uplinks: Uplinks {
            wifi: true,
            ethernet: false,
        },
    };
    assert!(offers_language_switch(&setup));
    let outcomes = assert_layered_run_matches_full_paint(SIZE, |run| {
        let mut state = DeviceInfoRenderState::new(run.now());
        let still = |run: &mut GlRun<'_>, state: &mut DeviceInfoRenderState, current| {
            for _ in 0..5 {
                paint_speaking(run, state, &setup, Some(current), FrameKind::AnimationOnly);
            }
        };
        paint_speaking(run, &mut state, &setup, Some(spoken), FrameKind::Fresh);
        still(run, &mut state, spoken);
        let chip = state
            .element_bounds(&language_key(picked))
            .expect("BUG: the switch lays out a chip per language");
        let (x, y) = (
            f64::from(chip.x + chip.w / 2.0),
            f64::from(chip.y + chip.h / 2.0),
        );
        state.push_touch(TouchEvent::Down { id: 0, x, y });
        paint_speaking(run, &mut state, &setup, Some(spoken), FrameKind::Fresh);
        assert!(
            state.tree.is_pressed(&language_key(picked)),
            "the press must land on the chip"
        );
        still(run, &mut state, spoken);
        state.push_touch(TouchEvent::Up { id: 0 });
        paint_speaking(run, &mut state, &setup, Some(spoken), FrameKind::Fresh);
        // The device takes the language up, and the switch marks it current.
        paint_speaking(run, &mut state, &setup, Some(picked), FrameKind::Fresh);
        still(run, &mut state, picked);
    });
    let Some(outcomes) = outcomes else { return };
    let still = [REUSED; 5];
    let capture = [CAPTURED].as_slice();
    assert_eq!(
        outcomes,
        [
            capture, &still,
            // The press marks the chip, the release lets it go,
            // and the language the device takes up moves the switch's mark.
            capture, &still, capture, capture, &still,
        ]
        .concat()
    );
}
