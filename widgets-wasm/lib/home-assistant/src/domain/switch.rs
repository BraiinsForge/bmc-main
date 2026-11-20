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

//! Switches an integration provides, such as a smart plug or a relay.

use crate::{Entity, ServiceCall};

pub(super) const DOMAIN: &str = "switch";
const DEVICE_CLASS: &str = "device_class";
pub const ATTRIBUTES: &[&str] = &[DEVICE_CLASS];

/// What a switch powers, as its device class names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Outlet,
    Switch,
}

#[must_use]
pub fn turn_on() -> ServiceCall {
    ServiceCall::new(DOMAIN, "turn_on")
}

#[must_use]
pub fn turn_off() -> ServiceCall {
    ServiceCall::new(DOMAIN, "turn_off")
}

/// None for a switch without a device class, or with one this crate does not know.
#[must_use]
pub fn device_class(switch: &Entity) -> Option<DeviceClass> {
    match switch.text(DEVICE_CLASS)? {
        "outlet" => Some(DeviceClass::Outlet),
        "switch" => Some(DeviceClass::Switch),
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
        assert_eq!(
            device_class(&with_class("outlet")),
            Some(DeviceClass::Outlet)
        );
        assert_eq!(
            device_class(&with_class("switch")),
            Some(DeviceClass::Switch)
        );
    }

    #[wasm_test]
    fn a_switch_without_a_known_device_class_has_none() {
        assert_eq!(device_class(&with_class("valve")), None);
        assert_eq!(device_class(&Entity::reporting("on", vec![])), None);
    }
}
