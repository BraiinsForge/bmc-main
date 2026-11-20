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
use crate::fixtures::gate;
use crate::interaction::gesture::{Gesture, OnTap};
use crate::interaction::pending::Pending;
use crate::present::Reach;
use bmc_wasm_sdk::wasm_test;
use home_assistant::AttributeValue;
use home_assistant::domain::{UNAVAILABLE, UNKNOWN};

const ONE_DECIMAL: Precision = Precision::Known(Some(1));

fn entity(state: &str, attributes: &[(&str, &str)]) -> Entity {
    Entity::reporting(
        state,
        attributes
            .iter()
            .map(|(name, value)| (*name, AttributeValue::Text((*value).to_owned())))
            .collect(),
    )
}

fn celsius(state: &str) -> Entity {
    entity(
        state,
        &[
            ("unit_of_measurement", "°C"),
            ("device_class", "temperature"),
        ],
    )
}

fn class(state: &str, device_class: &str) -> Entity {
    entity(state, &[("device_class", device_class)])
}

fn tile(kind: Kind, entity: &Entity, precision: Precision, reach: Reach) -> Tile {
    let pending = Pending::default();
    let gesture = Gesture::new(OnTap::Fires, None);
    let context = gate(&pending, &gesture, reach);
    Tile::new(&State::read(kind, entity, precision), &context)
}

fn reads(kind: Kind, entity: &Entity, precision: Precision) -> Reading {
    tile(kind, entity, precision, Reach::Ready).reading
}

fn words(text: &str) -> Reading {
    Reading::Words(text.to_owned())
}

#[wasm_test]
fn a_number_reads_at_home_assistants_precision_with_its_unit() {
    assert_eq!(
        reads(Kind::Sensor, &celsius("21.456"), ONE_DECIMAL),
        words("21,5\u{a0}°C"),
        "the default number format separates decimals with a comma, and no line break splits the unit off"
    );
}

#[wasm_test]
fn a_precision_pads_a_number_written_with_fewer_decimals() {
    assert_eq!(
        reads(Kind::Sensor, &celsius("21"), Precision::Known(Some(2))),
        words("21,00\u{a0}°C")
    );
}

#[wasm_test]
fn without_a_precision_a_number_keeps_the_decimals_it_was_written_with() {
    assert_eq!(
        reads(Kind::Sensor, &celsius("21.40"), Precision::Known(None)),
        words("21,40\u{a0}°C"),
        "the trailing zero is written, so it shows"
    );
}

#[wasm_test]
fn a_number_waits_for_the_precision_rather_than_jump_to_other_decimals() {
    assert_eq!(
        reads(Kind::Sensor, &celsius("21.456"), Precision::Awaiting),
        Reading::none()
    );
}

#[wasm_test]
fn words_and_digits_without_a_unit_read_as_written() {
    assert_eq!(
        reads(Kind::Sensor, &entity("partly_cloudy", &[]), ONE_DECIMAL),
        words("partly_cloudy")
    );
    assert_eq!(
        reads(Kind::Sensor, &entity("42", &[]), Precision::Awaiting),
        words("42"),
        "a state the frontend shows as written waits for no precision"
    );
}

#[wasm_test]
fn a_timestamp_reads_as_a_time() {
    let at = class("2026-11-13T10:00:00+00:00", "timestamp");
    assert_eq!(
        reads(Kind::Sensor, &at, Precision::Awaiting),
        Reading::Time(SystemTime {
            unix_secs: 1_794_564_000
        })
    );
    assert_eq!(
        reads(Kind::Sensor, &class("soon", "timestamp"), ONE_DECIMAL),
        words("soon"),
        "a time that does not parse reads as written"
    );
}

#[wasm_test]
fn a_sensor_without_a_value_reads_as_a_dash_in_grey() {
    for state in [UNKNOWN, UNAVAILABLE] {
        let tile = tile(Kind::Sensor, &celsius(state), ONE_DECIMAL, Reach::Ready);
        assert_eq!(tile.reading, Reading::none(), "{state}");
        assert_eq!(tile.card.color, None, "{state}");
    }
}

#[wasm_test]
fn a_binary_sensor_reads_as_its_device_classs_words() {
    let door = |state| reads(Kind::BinarySensor, &class(state, "door"), ONE_DECIMAL);
    assert_eq!(door("on"), words("Open"));
    assert_eq!(door("off"), words("Closed"));
    assert_eq!(door(UNKNOWN), Reading::none());
}

#[wasm_test]
fn a_binary_sensor_is_amber_while_on_red_for_a_danger_and_grey_while_off() {
    let color = |entity: &Entity| {
        tile(Kind::BinarySensor, entity, ONE_DECIMAL, Reach::Ready)
            .card
            .color
    };
    assert_eq!(color(&class("on", "door")), Some(Accent::Amber));
    assert_eq!(color(&class("on", "smoke")), Some(Accent::Red));
    assert_eq!(color(&class("on", "carbon_monoxide")), Some(Accent::Red));
    assert_eq!(color(&class("on", "motion")), Some(Accent::Amber));
    assert_eq!(color(&entity("on", &[])), Some(Accent::Amber));
    assert_eq!(color(&class("off", "smoke")), None);
}

#[wasm_test]
fn a_sensor_wears_the_icon_colour_and_a_battery_its_charges() {
    let color = |entity: &Entity| {
        tile(Kind::Sensor, entity, ONE_DECIMAL, Reach::Ready)
            .card
            .color
    };
    assert_eq!(color(&celsius("21")), Some(Accent::Neutral));
    let cases = [
        ("70", Accent::Green),
        ("69.9", Accent::Orange),
        ("30", Accent::Orange),
        ("29", Accent::Red),
    ];
    for (charge, accent) in cases {
        let battery = entity(
            charge,
            &[("unit_of_measurement", "%"), ("device_class", BATTERY)],
        );
        assert_eq!(color(&battery), Some(accent), "{charge} %");
    }
}

#[wasm_test]
fn a_battery_awaiting_its_precision_already_shows_its_charge() {
    let battery = entity(
        "12",
        &[("unit_of_measurement", "%"), ("device_class", BATTERY)],
    );
    let tile = tile(Kind::Sensor, &battery, Precision::Awaiting, Reach::Ready);
    assert_eq!(tile.card.color, Some(Accent::Red));
    assert_eq!(tile.card.icon, icons::sensor(Some(BATTERY), Some(12.0)));
}

#[wasm_test]
fn the_line_tells_when_the_value_last_changed() {
    let mut outdoor = celsius("21");
    outdoor.last_changed = Some(1_794_564_000.7);
    assert_eq!(
        tile(Kind::Sensor, &outdoor, ONE_DECIMAL, Reach::Ready)
            .card
            .line,
        Some(Line::Since(SystemTime {
            unix_secs: 1_794_564_000
        }))
    );
}

#[wasm_test]
fn a_reading_takes_no_touch_and_fires_nothing() {
    let tile = tile(
        Kind::BinarySensor,
        &class("on", "door"),
        ONE_DECIMAL,
        Reach::Ready,
    );
    assert_eq!(tile.card_action(), CardAction::None);
    assert_eq!(
        tile.card.action, None,
        "the icon is not dimmed as a blocked action"
    );
}

#[wasm_test]
fn an_unavailable_sensor_says_so_and_keeps_no_colour() {
    let tile = tile(
        Kind::Sensor,
        &celsius(UNAVAILABLE),
        ONE_DECIMAL,
        Reach::Unavailable,
    );
    assert_eq!(tile.card.line, Some(Line::State("Unavailable".to_owned())));
    assert_eq!(tile.card.color, None);
}
