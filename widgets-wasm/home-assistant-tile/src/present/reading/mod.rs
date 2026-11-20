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

//! Sensors and binary sensors: entities the tile shows a reading of, large enough to read across the room.
//! Neither has a card action.

mod icons;
mod labels;
#[cfg(test)]
mod tests;

use bmc_wasm_sdk::typography::unbroken;
use bmc_wasm_sdk::*;
use home_assistant::Entity;
use home_assistant::domain::binary_sensor::{self, DeviceClass};
use home_assistant::domain::{on_off, sensor};
use home_assistant::format::with_unit;

use super::{CardAction, Context, Precision, Presenter, card};
use crate::interaction::pending::Request;
use crate::view::blocks::badge::Disc;
use crate::view::card::{Card, Line, Reading, StateDisplay};
use crate::view::style::{Accent, Icon};
use crate::view::touches::Touches;

const BATTERY: &str = "battery";
/// The charges in percent from which the frontend draws a battery sensor green, then orange.
const BATTERY_HIGH: f64 = 70.0;
const BATTERY_MEDIUM: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sensor,
    BinarySensor,
}

impl Kind {
    /// The frontend's default icon for its domain, without a device class.
    pub(super) fn icon(self) -> Icon {
        match self {
            Self::Sensor => icons::sensor(None, None),
            Self::BinarySensor => icons::binary_sensor(None, false),
        }
    }

    /// Only a sensor's number is shown at a display precision.
    pub(super) fn reads_precision(self) -> bool {
        self == Self::Sensor
    }
}

/// What a sensor or a binary sensor reports now, read as the tile shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    value: Value,
    last_changed: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Sensor {
        device_class: Option<String>,
        /// None before the first state, or while Home Assistant has none.
        value: Option<SensorValue>,
    },
    Binary {
        device_class: Option<DeviceClass>,
        /// None while Home Assistant does not know.
        on: Option<bool>,
    },
}

/// A sensor's value, its decimals settled.
#[derive(Debug, Clone, PartialEq)]
enum SensorValue {
    Number {
        value: f64,
        decimals: u32,
        unit: Option<String>,
    },
    /// A number whose display precision is on its way: its icon and colour show, its digits wait.
    AwaitingPrecision(f64),
    /// When something happened or will.
    Time(SystemTime),
    /// When something started, such as the device's boot.
    Uptime(SystemTime),
    /// The state as written, such as an enum sensor's option.
    Text(String),
}

impl State {
    #[must_use]
    pub fn read(kind: Kind, entity: &Entity, precision: Precision) -> Self {
        let value = match kind {
            Kind::Sensor => Value::Sensor {
                device_class: sensor::device_class(entity).map(str::to_owned),
                value: sensor_value(entity, precision),
            },
            Kind::BinarySensor => Value::Binary {
                device_class: binary_sensor::device_class(entity),
                on: on_off(entity),
            },
        };
        Self {
            value,
            last_changed: entity.last_changed.and_then(unix_secs),
        }
    }
}

fn sensor_value(entity: &Entity, precision: Precision) -> Option<SensorValue> {
    let time = |at: &str| parse_datetime(at).map(|unix_secs| SystemTime { unix_secs });
    let text = |state: &str| SensorValue::Text(state.to_owned());
    Some(match (sensor::value(entity)?, precision) {
        (sensor::Value::Number { value, .. }, Precision::Awaiting) => {
            SensorValue::AwaitingPrecision(value)
        }
        (
            sensor::Value::Number {
                value,
                written_decimals,
            },
            Precision::Known(precision),
        ) => SensorValue::Number {
            value,
            decimals: precision.unwrap_or(written_decimals),
            unit: sensor::unit(entity).map(str::to_owned),
        },
        // A time that does not parse reads as written, as the frontend shows it.
        (sensor::Value::Timestamp(at), _) => time(at).map_or_else(|| text(at), SensorValue::Time),
        (sensor::Value::Uptime(at), _) => time(at).map_or_else(|| text(at), SensorValue::Uptime),
        (sensor::Value::Text(state), _) => text(state),
    })
}

/// Home Assistant's `last_changed`, seconds since the epoch with a fraction, to the whole second.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the cast saturates a time past i64, and the fraction is dropped on purpose"
)]
fn unix_secs(secs: f64) -> Option<SystemTime> {
    secs.is_finite().then(|| SystemTime {
        unix_secs: secs.floor() as i64,
    })
}

impl Value {
    fn reading(&self) -> Reading {
        match self {
            Self::Sensor {
                value: Some(value), ..
            } => value.reading(),
            Self::Binary {
                device_class,
                on: Some(on),
            } => Reading::Words(labels::label(*device_class, *on).to_owned()),
            Self::Sensor { value: None, .. } | Self::Binary { on: None, .. } => Reading::none(),
        }
    }

    fn icon(&self) -> Icon {
        match self {
            Self::Sensor {
                device_class,
                value,
            } => icons::sensor(
                device_class.as_deref(),
                value.as_ref().and_then(SensorValue::number),
            ),
            Self::Binary { device_class, on } => {
                icons::binary_sensor(*device_class, *on == Some(true))
            }
        }
    }

    /// The frontend's colour: a sensor wears its icon colour, a battery its charge's,
    /// a binary sensor amber or a danger's red while on; none while there is no value or it is off.
    fn color(&self) -> Option<Accent> {
        match self {
            Self::Sensor { value: None, .. }
            | Self::Binary {
                on: None | Some(false),
                ..
            } => None,
            Self::Binary {
                device_class,
                on: Some(true),
            } => Some(if warns_of_danger(*device_class) {
                Accent::Red
            } else {
                Accent::Amber
            }),
            Self::Sensor {
                device_class,
                value: Some(value),
            } if device_class.as_deref() == Some(BATTERY) => Some(match value.number() {
                Some(charge) if charge >= BATTERY_HIGH => Accent::Green,
                Some(charge) if charge >= BATTERY_MEDIUM => Accent::Orange,
                Some(_) => Accent::Red,
                None => Accent::Neutral,
            }),
            Self::Sensor { value: Some(_), .. } => Some(Accent::Neutral),
        }
    }
}

impl SensorValue {
    fn reading(&self) -> Reading {
        match self {
            // Unbroken, as the renderer would break between a number and its unit.
            Self::Number {
                value,
                decimals,
                unit,
            } => Reading::Words(unbroken(with_unit(
                format_number!(*value, *decimals),
                unit.as_deref(),
            ))),
            // TODO(BDK-1153): the frontend shows an uptime without "ago", which the host cannot word yet.
            Self::Time(at) | Self::Uptime(at) => Reading::Time(*at),
            Self::Text(text) => Reading::Words(text.clone()),
            Self::AwaitingPrecision(_) => Reading::none(),
        }
    }

    /// The number a device class draws a range of icons for, such as a battery's charge.
    fn number(&self) -> Option<f64> {
        match self {
            Self::Number { value, .. } | Self::AwaitingPrecision(value) => Some(*value),
            Self::Time(_) | Self::Uptime(_) | Self::Text(_) => None,
        }
    }
}

/// Whether the frontend draws a binary sensor of this class red while on.
fn warns_of_danger(device_class: Option<DeviceClass>) -> bool {
    match device_class {
        Some(
            DeviceClass::Battery
            | DeviceClass::CarbonMonoxide
            | DeviceClass::Gas
            | DeviceClass::GlassBreak
            | DeviceClass::Heat
            | DeviceClass::Lock
            | DeviceClass::Moisture
            | DeviceClass::Problem
            | DeviceClass::Safety
            | DeviceClass::Smoke
            | DeviceClass::Sound
            | DeviceClass::Tamper,
        ) => true,
        Some(
            DeviceClass::BatteryCharging
            | DeviceClass::Cold
            | DeviceClass::Connectivity
            | DeviceClass::Door
            | DeviceClass::GarageDoor
            | DeviceClass::Light
            | DeviceClass::Motion
            | DeviceClass::Moving
            | DeviceClass::Occupancy
            | DeviceClass::Opening
            | DeviceClass::Plug
            | DeviceClass::Power
            | DeviceClass::Presence
            | DeviceClass::Running
            | DeviceClass::Update
            | DeviceClass::Vibration
            | DeviceClass::Window,
        )
        | None => false,
    }
}

/// One frame of a reading's tile: the card over its value, taking no touch.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    card: Card,
    reading: Reading,
}

impl Tile {
    #[must_use]
    pub fn new(state: &State, context: &Context) -> Self {
        let display = StateDisplay {
            line: state.last_changed.map(Line::Since),
            color: state.value.color(),
            icon: state.value.icon(),
            disc: Disc::Tinted,
            mark: None,
        };
        Self {
            card: card(context, display, &CardAction::None),
            reading: state.value.reading(),
        }
    }
}

impl Presenter for Tile {
    fn view(&self, touches: &Touches) -> Node {
        self.card.over_reading(&self.reading, touches)
    }

    fn touched(&self, _touches: &Touches) -> Option<Request> {
        None
    }

    fn card_action(&self) -> CardAction {
        CardAction::None
    }
}
