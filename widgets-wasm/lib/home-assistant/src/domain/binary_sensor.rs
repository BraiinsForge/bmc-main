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

//! Binary sensors: something an integration finds on or off, such as an open door or smoke.

use crate::Entity;

pub(super) const DOMAIN: &str = "binary_sensor";
const DEVICE_CLASS: &str = "device_class";
pub const ATTRIBUTES: &[&str] = &[DEVICE_CLASS];

/// What on means for the sensor, such as open for a door: the classes Home Assistant documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Battery,
    BatteryCharging,
    CarbonMonoxide,
    Cold,
    Connectivity,
    Door,
    GarageDoor,
    Gas,
    GlassBreak,
    Heat,
    Light,
    Lock,
    Moisture,
    Motion,
    Moving,
    Occupancy,
    Opening,
    Plug,
    Power,
    Presence,
    Problem,
    Running,
    Safety,
    Smoke,
    Sound,
    Tamper,
    Update,
    Vibration,
    Window,
}

/// None for a binary sensor without a device class, or with one this crate does not know.
#[must_use]
pub fn device_class(sensor: &Entity) -> Option<DeviceClass> {
    match sensor.text(DEVICE_CLASS)? {
        "battery" => Some(DeviceClass::Battery),
        "battery_charging" => Some(DeviceClass::BatteryCharging),
        "carbon_monoxide" => Some(DeviceClass::CarbonMonoxide),
        "cold" => Some(DeviceClass::Cold),
        "connectivity" => Some(DeviceClass::Connectivity),
        "door" => Some(DeviceClass::Door),
        "garage_door" => Some(DeviceClass::GarageDoor),
        "gas" => Some(DeviceClass::Gas),
        "glass_break" => Some(DeviceClass::GlassBreak),
        "heat" => Some(DeviceClass::Heat),
        "light" => Some(DeviceClass::Light),
        "lock" => Some(DeviceClass::Lock),
        "moisture" => Some(DeviceClass::Moisture),
        "motion" => Some(DeviceClass::Motion),
        "moving" => Some(DeviceClass::Moving),
        "occupancy" => Some(DeviceClass::Occupancy),
        "opening" => Some(DeviceClass::Opening),
        "plug" => Some(DeviceClass::Plug),
        "power" => Some(DeviceClass::Power),
        "presence" => Some(DeviceClass::Presence),
        "problem" => Some(DeviceClass::Problem),
        "running" => Some(DeviceClass::Running),
        "safety" => Some(DeviceClass::Safety),
        "smoke" => Some(DeviceClass::Smoke),
        "sound" => Some(DeviceClass::Sound),
        "tamper" => Some(DeviceClass::Tamper),
        "update" => Some(DeviceClass::Update),
        "vibration" => Some(DeviceClass::Vibration),
        "window" => Some(DeviceClass::Window),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AttributeValue;
    use bmc_wasm_sdk::wasm_test;

    fn with_class(class: &str) -> Entity {
        Entity::tracked(
            "on",
            vec![(DEVICE_CLASS, AttributeValue::Text(class.to_owned()))],
            ATTRIBUTES,
        )
    }

    #[wasm_test]
    fn the_device_class_is_read_from_a_tracked_attribute() {
        assert_eq!(device_class(&with_class("door")), Some(DeviceClass::Door));
        assert_eq!(
            device_class(&with_class("carbon_monoxide")),
            Some(DeviceClass::CarbonMonoxide)
        );
    }

    #[wasm_test]
    fn a_binary_sensor_without_a_known_device_class_has_none() {
        assert_eq!(device_class(&with_class("teleporter")), None);
        assert_eq!(device_class(&Entity::reporting("on", vec![])), None);
    }
}
