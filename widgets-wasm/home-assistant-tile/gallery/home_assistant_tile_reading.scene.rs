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

//! A sensor's or a binary sensor's tile: its value set large, at each size and in each state.

use bmc_gallery::prelude::*;
use home_assistant::{AttributeValue, Entity};
use home_assistant_tile::fixtures;
use home_assistant_tile::interaction::gesture::{Gesture, OnTap};
use home_assistant_tile::interaction::pending::Pending;
use home_assistant_tile::present::reading::{Kind, State, Tile};
use home_assistant_tile::present::{Precision, Presenter, Reach};
use home_assistant_tile::view::touches::Touches;

scene_meta! { title: "Widgets / Home Assistant Tile / Reading" }

/// An entity as Home Assistant reports it: its kind, state and attributes,
/// and the display precision its registry entry holds.
struct Sample {
    kind: Kind,
    state: &'static str,
    attributes: &'static [(&'static str, &'static str)],
    precision: Option<u32>,
}

const SAMPLES: [(Sample, &str); 11] = [
    (
        Sample {
            kind: Kind::Sensor,
            state: "21.456",
            attributes: &[
                ("unit_of_measurement", "°C"),
                ("device_class", "temperature"),
            ],
            precision: Some(1),
        },
        "Temperature",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "1234.5",
            attributes: &[("unit_of_measurement", "W"), ("device_class", "power")],
            precision: Some(0),
        },
        "Power",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "84",
            attributes: &[("unit_of_measurement", "%"), ("device_class", "battery")],
            precision: None,
        },
        "Battery full",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "12",
            attributes: &[("unit_of_measurement", "%"), ("device_class", "battery")],
            precision: None,
        },
        "Battery low",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "Partly cloudy with a chance of rain",
            attributes: &[],
            precision: None,
        },
        "Text",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "1970-01-01T00:05:00+00:00",
            attributes: &[("device_class", "timestamp")],
            precision: None,
        },
        "Time ahead",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "1969-12-28T00:00:00+00:00",
            attributes: &[("device_class", "uptime")],
            precision: None,
        },
        "Uptime",
    ),
    (
        Sample {
            kind: Kind::BinarySensor,
            state: "on",
            attributes: &[("device_class", "door")],
            precision: None,
        },
        "Door open",
    ),
    (
        Sample {
            kind: Kind::BinarySensor,
            state: "off",
            attributes: &[("device_class", "door")],
            precision: None,
        },
        "Door closed",
    ),
    (
        Sample {
            kind: Kind::BinarySensor,
            state: "on",
            attributes: &[("device_class", "smoke")],
            precision: None,
        },
        "Smoke",
    ),
    (
        Sample {
            kind: Kind::Sensor,
            state: "unknown",
            attributes: &[
                ("unit_of_measurement", "°C"),
                ("device_class", "temperature"),
            ],
            precision: None,
        },
        "Unknown",
    ),
];
const READINESS: [(Reach, &str); 3] = [
    (Reach::Ready, "Ready"),
    (Reach::Unavailable, "Unavailable"),
    (Reach::Reconnecting, "Reconnecting"),
];
const NAMES: [(&str, &str); 2] = [
    ("Outdoor", "Short"),
    (
        "Outdoor temperature by the north-facing garage wall",
        "Long",
    ),
];
/// How long ago the state changed, in seconds: the gallery's clock reads 0, so the change is that far before it.
const CHANGED_AGO: i64 = 180;

fn select<T: Copy>(ctx: &mut SceneCtx, name: &str, options: &[(T, &str)]) -> T {
    let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
    options[ctx.select(name, &labels, 0)].0
}

fn entity(sample: &Sample, reach: Reach) -> Entity {
    let state = match reach {
        Reach::Unavailable => "unavailable",
        Reach::Ready | Reach::Reconnecting => sample.state,
    };
    let mut entity = Entity::reporting(
        state,
        sample
            .attributes
            .iter()
            .map(|(name, value)| (*name, AttributeValue::Text((*value).to_owned())))
            .collect(),
    );
    #[expect(clippy::cast_precision_loss, reason = "a few minutes, exact in f64")]
    let changed = -CHANGED_AGO as f64;
    entity.last_changed = Some(changed);
    entity
}

#[scene(default)]
fn reading(ctx: &mut SceneCtx, ui: &mut Ui) {
    ctx.group("Reading");
    let labels: Vec<&str> = SAMPLES.iter().map(|(_, label)| *label).collect();
    let sample = &SAMPLES[ctx.select("Sample", &labels, 0)].0;
    let reach = select(ctx, "Readiness", &READINESS);
    let precision = if ctx.toggle("Precision on its way", false) {
        Precision::Awaiting
    } else {
        Precision::Known(sample.precision)
    };
    let name = select(ctx, "Name", &NAMES);
    let size_labels = fixtures::SIZES.map(|(_, label)| label);
    let size = fixtures::SIZES[ctx.select("Size", &size_labels, fixtures::DEFAULT_SIZE)].0;

    let state = State::read(sample.kind, &entity(sample, reach), precision);
    ctx.wrapping_flow(
        ui,
        [titled("Tile", size, move || {
            let pending = Pending::default();
            let gesture = Gesture::new(OnTap::Fires, None);
            let context = fixtures::context(&pending, &gesture, name, (None, reach), size);
            Tile::new(&state, &context).view(&Touches::default())
        })],
    );
}
