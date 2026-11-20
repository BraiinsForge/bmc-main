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
use crate::fixtures::{self, SIZES};
use crate::interaction::gesture::{Gesture, OnTap};
use crate::interaction::pending::Pending;
use crate::present::Reach;
use crate::view::card::Line;
use bmc_wasm_sdk::wasm_test;
use home_assistant::{CallId, RequestError, ServiceCall};

const SMALL: (u32, u32) = SIZES[0].0;
const MEDIUM: (u32, u32) = SIZES[1].0;
const LARGE: (u32, u32) = SIZES[2].0;
const FULL: (u32, u32) = SIZES[3].0;

fn context<'a>(pending: &'a Pending, gesture: &'a Gesture, size: (u32, u32)) -> Context<'a> {
    fixtures::context(pending, gesture, "Kitchen", (None, Reach::Ready), size)
}

fn state(on: bool, brightness: Option<u8>) -> State {
    State {
        on: Some(on),
        brightness,
        dims: true,
        warmth: Some(Kelvins {
            warmest: 2000.0,
            coolest: 6000.0,
        }),
        kelvin: Some(2000.0),
        colours: true,
        colour: Some(full(0.0)),
    }
}

fn full(hue: f64) -> HueSaturation {
    HueSaturation {
        hue,
        saturation: 100.0,
    }
}

fn switching(on: bool) -> State {
    State {
        dims: false,
        warmth: None,
        kelvin: None,
        colours: false,
        colour: None,
        ..state(on, None)
    }
}

fn tile(state: &State) -> Tile {
    Tile::new(
        state,
        &context(&Pending::default(), &Gesture::new(OnTap::Fires, None), FULL),
    )
}

/// What a tap making `touched` on `stepper` asks of `tile`.
fn tap(tile: &Tile, stepper: Option<Stepper>, touched: Touched) -> Option<Request> {
    let stepper = stepper.expect("BUG: the tile shows that stepper");
    tile.touched(&stepper.tap(touched))
}

fn fired(tile: &Tile) -> Option<Request> {
    match tile.card_action() {
        CardAction::Ready(request) => Some(request),
        CardAction::None | CardAction::Blocked => None,
    }
}

#[wasm_test]
fn the_card_toggles_the_light() {
    assert_eq!(
        fired(&tile(&state(true, Some(60)))),
        Some(
            Request::flip(POWER, false, light::turn_off()).superseding(&[BRIGHTNESS, WARMTH, HUE])
        )
    );
    assert_eq!(
        fired(&tile(&state(false, None))),
        Some(Request::flip(POWER, true, light::turn_on()))
    );
}

#[wasm_test]
fn turning_it_off_drops_what_still_waits_to_be_sent() {
    let mut pending = Pending::default();
    pending.submit(set_brightness(70, Timing::AfterTaps), |_, _| {
        Err(RequestError::NotConnected)
    });
    pending.submit(turn(false), |_, _| Ok(CallId::from_raw(1)));
    assert_eq!(
        pending.number(BRIGHTNESS),
        None,
        "it would turn the light back on"
    );
}

#[wasm_test]
fn plus_turns_a_light_that_is_off_on_at_the_brightness_it_had() {
    let off = tile(&state(false, None));
    assert_eq!(
        tap(&off, off.brightness(), Touched::Up),
        Some(Request::flip(POWER, true, light::turn_on()))
    );
    assert_eq!(off.brightness().map(|stepper| stepper.down), Some(false));
}

#[wasm_test]
fn a_step_lands_on_the_next_ten_once_the_taps_stop() {
    let cases = [
        (60, Touched::Up, 70),
        (65, Touched::Up, 70),
        (95, Touched::Up, 100),
        (1, Touched::Up, 10),
        (65, Touched::Down, 60),
        (11, Touched::Down, 10),
    ];
    for (from, touched, to) in cases {
        let on = tile(&state(true, Some(from)));
        assert_eq!(
            tap(&on, on.brightness(), touched),
            Some(set_brightness(to, Timing::AfterTaps)),
            "{from} {touched:?}"
        );
    }
}

#[wasm_test]
fn plus_has_nothing_to_do_at_full_brightness() {
    let full = tile(&state(true, Some(100)));
    assert_eq!(full.brightness().map(|stepper| stepper.up), Some(false));
}

#[wasm_test]
fn minus_from_ten_percent_or_below_turns_the_light_off() {
    for from in [10, 4] {
        let on = tile(&state(true, Some(from)));
        assert_eq!(
            tap(&on, on.brightness(), Touched::Down),
            Some(turn(false)),
            "{from}"
        );
    }
}

#[wasm_test]
fn a_tap_on_the_bar_sets_the_brightness_at_once_and_never_turns_it_off() {
    let off = tile(&state(false, None));
    assert_eq!(
        tap(&off, off.brightness(), Touched::Set(0.374)),
        Some(set_brightness(37, Timing::Now))
    );
    assert_eq!(
        tap(&off, off.brightness(), Touched::Set(0.0)),
        Some(set_brightness(1, Timing::Now))
    );
}

#[wasm_test]
fn a_preset_sets_its_brightness_at_once_and_the_matching_one_is_selected() {
    let on = tile(&state(true, Some(50)));
    let presets = on.presets().expect("BUG: a light that dims has presets");
    assert_eq!(presets.selected, Some(1));
    assert_eq!(
        on.touched(&presets.tap(2)),
        Some(set_brightness(75, Timing::Now))
    );
}

#[wasm_test]
fn a_brightness_asked_for_shows_the_light_on_until_confirmed() {
    let mut pending = Pending::default();
    pending.submit(set_brightness(80, Timing::AfterTaps), |_, _| {
        Err(RequestError::NotConnected)
    });
    let gesture = Gesture::new(OnTap::Fires, None);
    let tile = Tile::new(&state(false, None), &context(&pending, &gesture, FULL));
    assert_eq!(tile.percent(), Some(80));
    assert_eq!(tile.brightness().map(|stepper| stepper.pending), Some(true));
    assert_eq!(tile.card.line, Some(Line::State("On".to_owned())));
}

#[wasm_test]
fn a_value_picked_outright_is_not_drawn_faint() {
    let gesture = Gesture::new(OnTap::Fires, None);
    let picked = [
        set_brightness(37, Timing::Now),
        Request::number(WARMTH, 4250.0, light::turn_on_at_kelvin(4250), Timing::Now),
        Request::number(HUE, 90.0, light::turn_on_at_hue(90.0, 100.0), Timing::Now),
    ];
    for request in picked {
        let mut pending = Pending::default();
        pending.submit(request, |_, _| Ok(CallId::from_raw(1)));
        let tile = Tile::new(&state(true, Some(60)), &context(&pending, &gesture, FULL));
        let faint = [tile.brightness(), tile.warmth(), tile.hue()].map(|stepper| {
            stepper
                .expect("BUG: a colour light shows every stepper at full")
                .pending
        });
        assert_eq!(faint, [false; 3]);
    }
}

#[wasm_test]
fn a_toggle_on_its_way_shows_at_once() {
    let mut pending = Pending::default();
    pending.submit(turn(false), |_, _| Ok(CallId::from_raw(1)));
    let gesture = Gesture::new(OnTap::Fires, None);
    let tile = Tile::new(&state(true, Some(60)), &context(&pending, &gesture, FULL));
    assert_eq!(tile.card.line, Some(Line::State("Off".to_owned())));
    assert_eq!(tile.card.color, None);
    assert_eq!(tile.card.icon, Icon(&LIGHTBULB_OFF));
}

#[wasm_test]
fn a_setting_asked_for_during_a_turn_off_shows_the_light_on() {
    let on = tile(&state(true, Some(60)));
    let settings = [
        set_brightness(75, Timing::Now),
        tap(&on, on.warmth(), Touched::Set(0.5)).expect("BUG: the warmth takes a tap"),
        tap(&on, on.hue(), Touched::Set(0.5)).expect("BUG: the hue takes a tap"),
    ];
    let gesture = Gesture::new(OnTap::Fires, None);
    for setting in settings {
        let mut pending = Pending::default();
        pending.submit(turn(false), |_, _| Ok(CallId::from_raw(1)));
        pending.submit(setting, |_, _| Ok(CallId::from_raw(2)));
        let tile = Tile::new(&state(true, Some(60)), &context(&pending, &gesture, FULL));
        assert_eq!(
            tile.card.line,
            Some(Line::State("On".to_owned())),
            "a setting turns the light on, so the off still on its way is moot"
        );
    }
}

#[wasm_test]
fn the_line_tells_the_brightness_while_on_and_its_bar_is_not_on_show() {
    let (pending, gesture) = (Pending::default(), Gesture::new(OnTap::Fires, None));
    let line = |state: &State, room| {
        Tile::new(state, &context(&pending, &gesture, room))
            .card
            .line
    };
    let text = |text: &str| Some(Line::State(text.to_owned()));
    assert_eq!(line(&state(true, Some(60)), SMALL), text("60%"));
    assert_eq!(line(&state(true, Some(60)), FULL), text("On"));
    assert_eq!(line(&state(false, None), SMALL), text("Off"));
    assert_eq!(line(&state(false, None), FULL), text("Off"));
    assert_eq!(line(&switching(true), FULL), text("On"));
}

#[wasm_test]
fn with_neither_a_tap_nor_a_hold_set_it_shows_its_position_undimmed() {
    let neither = Gesture::new(OnTap::Nothing, None);
    let pending = Pending::default();
    let switch = Tile::new(&switching(true), &context(&pending, &neither, FULL));
    assert_eq!(switch.card_action(), CardAction::None);
    assert_eq!(switch.card.line, Some(Line::State("On".to_owned())));
    assert_eq!(switch.card.action, None, "a dimmed icon reads as unusable");
}

#[wasm_test]
fn with_neither_a_tap_nor_a_hold_set_the_blocks_still_act() {
    let neither = Gesture::new(OnTap::Nothing, None);
    let pending = Pending::default();
    let dimmer = Tile::new(&state(true, Some(60)), &context(&pending, &neither, FULL));
    assert!(tap(&dimmer, dimmer.brightness(), Touched::Up).is_some());
}

#[wasm_test]
fn an_unavailable_light_takes_no_touch() {
    let (pending, tap_fires) = (Pending::default(), Gesture::new(OnTap::Fires, None));
    let unavailable = Tile::new(
        &state(false, None),
        &Context {
            reach: Reach::Unavailable,
            ..context(&pending, &tap_fires, FULL)
        },
    );
    assert_eq!(fired(&unavailable), None);
    assert_eq!(
        tap(&unavailable, unavailable.brightness(), Touched::Up),
        None
    );
}

#[wasm_test]
fn a_light_that_only_switches_shows_no_blocks() {
    assert!(tile(&switching(true)).rows.is_empty());
}

#[wasm_test]
fn a_card_with_less_room_drops_the_last_rows() {
    let pending = Pending::default();
    let gesture = Gesture::new(OnTap::Fires, None);
    let rows = |size| Tile::new(&state(true, Some(60)), &context(&pending, &gesture, size)).rows;
    assert!(rows(SMALL).is_empty());
    assert_eq!(rows(MEDIUM), [Row::Brightness, Row::Warmth]);
    assert_eq!(rows(LARGE), [Row::Brightness, Row::Warmth, Row::Hue]);
    assert_eq!(
        rows(FULL),
        [Row::Brightness, Row::Warmth, Row::Hue, Row::Presets]
    );
}

#[wasm_test]
fn a_tap_on_the_temperatures_picks_the_kelvin_under_the_finger() {
    let on = tile(&state(true, Some(60)));
    let warmth = |kelvin: u32, timing| {
        #[expect(clippy::cast_precision_loss, reason = "a few thousand kelvin")]
        Request::number(
            WARMTH,
            kelvin as f32,
            light::turn_on_at_kelvin(kelvin),
            timing,
        )
        .superseding(&[HUE, POWER])
    };
    assert_eq!(
        tap(&on, on.warmth(), Touched::Set(0.5)),
        Some(warmth(4000, Timing::Now))
    );
    assert_eq!(
        tap(&on, on.warmth(), Touched::Up),
        Some(warmth(2400, Timing::AfterTaps)),
        "a tenth of the range cooler"
    );
    let coloured = tile(&State {
        kelvin: None,
        ..state(true, Some(60))
    });
    assert_eq!(
        tap(&coloured, coloured.warmth(), Touched::Up),
        Some(warmth(4400, Timing::AfterTaps)),
        "a light showing no colour temperature steps from the middle of its range"
    );
}

#[wasm_test]
fn a_colour_temperature_drops_a_hue_still_to_be_sent_and_the_other_way_round() {
    let on = tile(&state(true, Some(60)));
    let mut pending = Pending::default();
    let unsent = |_: Slot, _: &ServiceCall| Err::<CallId, _>(RequestError::NotConnected);
    pending.submit(
        tap(&on, on.hue(), Touched::Up).expect("BUG: the hue steps"),
        unsent,
    );
    pending.submit(
        tap(&on, on.warmth(), Touched::Up).expect("BUG: the warmth steps"),
        unsent,
    );
    assert_eq!(pending.number(HUE), None, "the hue would follow the warmth");
    pending.submit(
        tap(&on, on.hue(), Touched::Up).expect("BUG: the hue steps"),
        unsent,
    );
    assert_eq!(pending.number(WARMTH), None);
}

#[wasm_test]
fn a_light_showing_a_colour_has_no_cursor_on_its_temperatures() {
    let coloured = tile(&State {
        kelvin: None,
        ..state(true, Some(60))
    });
    let Some(Stepper {
        track: Track::Gradient(gradient),
        ..
    }) = coloured.warmth()
    else {
        panic!("BUG: the light takes a colour temperature");
    };
    assert_eq!(gradient.share, None);
}

#[wasm_test]
fn a_tap_on_the_hues_sends_that_hue_fully_saturated() {
    let on = tile(&state(true, Some(60)));
    assert_eq!(
        tap(&on, on.hue(), Touched::Set(0.25)),
        Some(
            Request::number(HUE, 90.0, light::turn_on_at_hue(90.0, 100.0), Timing::Now)
                .superseding(&[WARMTH, POWER])
        )
    );
}

fn showing_hue(hue: Option<f64>) -> Tile {
    tile(&State {
        kelvin: None,
        colour: hue.map(full),
        ..state(true, Some(60))
    })
}

#[wasm_test]
fn a_hue_step_moves_30_degrees_within_the_circle() {
    let cases = [
        (Some(90.0), Touched::Up, 90.0 + 30.0),
        (Some(90.0), Touched::Down, 90.0 - 30.0),
        (Some(350.0), Touched::Up, 360.0),
        (Some(10.0), Touched::Down, 0.0),
        (None, Touched::Up, 180.0 + 30.0),
    ];
    for (from, touched, to) in cases {
        let tile = showing_hue(from);
        assert_eq!(
            tap(&tile, tile.hue(), touched),
            Some(
                Request::number(
                    HUE,
                    to,
                    light::turn_on_at_hue(f64::from(to), 100.0),
                    Timing::AfterTaps
                )
                .superseding(&[WARMTH, POWER])
            ),
            "{from:?} {touched:?}: 30° a step, from 180° for a light showing no hue"
        );
    }
}

#[wasm_test]
fn the_hue_ends_have_nothing_to_step_past() {
    let ends = [showing_hue(Some(0.0)), showing_hue(Some(360.0))]
        .map(|tile| tile.hue().map(|stepper| (stepper.down, stepper.up)));
    assert_eq!(ends, [Some((false, true)), Some((true, false))]);
}

#[wasm_test]
fn the_icon_wears_the_lights_colour_whatever_its_brightness() {
    let green = |brightness| State {
        kelvin: None,
        colour: Some(full(120.0)),
        ..state(true, Some(brightness))
    };
    let lit = Some(Accent::Light(Color::from_rgb(0, 255, 0)));
    assert_eq!(tile(&green(100)).card.color, lit);
    assert_eq!(tile(&green(5)).card.color, lit);
}

fn pale_pink() -> State {
    State {
        kelvin: None,
        colour: Some(HueSaturation {
            hue: 0.0,
            saturation: 20.0,
        }),
        ..state(true, Some(100))
    }
}

#[wasm_test]
fn a_pale_light_wears_a_pale_icon_rather_than_a_full_colour() {
    assert_eq!(
        tile(&pale_pink()).card.color,
        Some(Accent::Light(Color::from_rgb(255, 153, 153))),
        "a fifth saturated, raised to two fifths for the icon, not full red"
    );
}

#[wasm_test]
fn a_white_light_greys_its_icon_and_a_pale_one_is_saturated() {
    assert_eq!(
        icon_color(Color::from_rgb(255, 250, 245)),
        Color::from_rgb(225, 220, 216),
        "a white drawn at 225 of 255"
    );
    assert_eq!(
        icon_color(Color::from_rgb(255, 204, 204)),
        Color::from_rgb(255, 153, 153),
        "a fifth saturated, raised to two fifths"
    );
    assert_eq!(
        icon_color(Color::from_rgb(255, 0, 0)),
        Color::from_rgb(255, 0, 0)
    );
}

#[wasm_test]
fn a_light_with_no_colour_keeps_the_amber_icon() {
    assert_eq!(tile(&switching(true)).card.color, Some(Accent::Amber));
}

#[wasm_test]
fn the_bulb_is_struck_through_while_the_light_is_off() {
    assert_eq!(tile(&state(true, Some(60))).card.icon, Icon(&LIGHTBULB));
    assert_eq!(tile(&state(false, None)).card.icon, Icon(&LIGHTBULB_OFF));
}

fn unknown() -> State {
    State {
        on: None,
        ..state(false, None)
    }
}

#[wasm_test]
fn a_light_in_no_known_position_says_so_rather_than_off() {
    let tile = tile(&unknown());
    assert_eq!(
        (tile.card.line, tile.card.mark),
        (Some(Line::State("Unknown".to_owned())), Some(Mark::Unknown))
    );
}

#[wasm_test]
fn a_tap_turns_a_light_in_no_known_position_on() {
    assert_eq!(
        fired(&tile(&unknown())),
        Some(Request::flip(POWER, true, light::turn_on()))
    );
}

#[wasm_test]
fn only_a_light_that_is_on_keeps_the_disc_behind_its_bulb() {
    let disc = |state: &State| tile(state).card.disc;
    assert_eq!(
        [
            disc(&state(true, Some(60))),
            disc(&state(false, None)),
            disc(&unknown())
        ],
        [Disc::Tinted, Disc::Bare, Disc::Bare]
    );
}

#[wasm_test]
fn the_light_reads_its_modes_and_values_from_the_entity() {
    let entity = Entity::tracked(
        "on",
        vec![
            (
                "supported_color_modes",
                home_assistant::AttributeValue::List(vec!["color_temp".to_owned()]),
            ),
            ("brightness", home_assistant::AttributeValue::Number(153.0)),
        ],
        light::ATTRIBUTES,
    );
    let read = State::read(&entity);
    assert_eq!(read.on, Some(true));
    assert!(read.dims, "a colour temperature light dims");
    assert!(!read.colours, "a colour temperature is no colour");
    assert_eq!(read.brightness, Some(60));
    assert_eq!(read.warmth, Some(Kelvins::default()));
}
