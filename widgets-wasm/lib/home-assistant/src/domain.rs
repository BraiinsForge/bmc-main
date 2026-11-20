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

//! What Home Assistant's domains mean, one module per domain:
//! the attributes its readers need, how to read them, and the services that change it.
//!
//! Every reader takes an [`Entity`], and every change is a [`crate::ServiceCall`] the widget sends with `Client::call`.
//! A builder is named after the service it calls, so `scene::turn_on()` is `scene.turn_on`.
//! Nothing here holds state or talks to Home Assistant.

pub mod automation;
pub mod binary_sensor;
pub mod button;
pub mod input_boolean;
pub mod input_button;
pub mod light;
pub mod remote;
pub mod scene;
pub mod script;
pub mod sensor;
pub mod siren;
pub mod switch;

use crate::{Entity, is_entity_id};

pub const FRIENDLY_NAME: &str = "friendly_name";
/// The state of an entity its integration cannot reach.
pub const UNAVAILABLE: &str = "unavailable";
/// The state of an entity without one yet, such as a button never pressed.
pub const UNKNOWN: &str = "unknown";
const ON: &str = "on";
const OFF: &str = "off";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Automation,
    BinarySensor,
    Button,
    InputBoolean,
    InputButton,
    Light,
    Remote,
    Scene,
    Script,
    Sensor,
    Siren,
    Switch,
}

impl Domain {
    /// The domain `entity_id` belongs to, such as [`Self::Scene`] for `scene.evening`;
    /// none for text that is not an entity ID, or for a domain this crate does not know.
    #[must_use]
    pub fn of(entity_id: &str) -> Option<Self> {
        let (name, _) = entity_id.split_once('.')?;
        if !is_entity_id(entity_id) {
            return None;
        }
        match name {
            automation::DOMAIN => Some(Self::Automation),
            binary_sensor::DOMAIN => Some(Self::BinarySensor),
            button::DOMAIN => Some(Self::Button),
            input_boolean::DOMAIN => Some(Self::InputBoolean),
            input_button::DOMAIN => Some(Self::InputButton),
            light::DOMAIN => Some(Self::Light),
            remote::DOMAIN => Some(Self::Remote),
            scene::DOMAIN => Some(Self::Scene),
            script::DOMAIN => Some(Self::Script),
            sensor::DOMAIN => Some(Self::Sensor),
            siren::DOMAIN => Some(Self::Siren),
            switch::DOMAIN => Some(Self::Switch),
            _ => None,
        }
    }

    /// The attributes its readers need a subscription to keep.
    #[must_use]
    pub fn attributes(self) -> &'static [&'static str] {
        match self {
            Self::Automation => automation::ATTRIBUTES,
            Self::BinarySensor => binary_sensor::ATTRIBUTES,
            Self::Button => button::ATTRIBUTES,
            Self::InputBoolean => input_boolean::ATTRIBUTES,
            Self::InputButton => input_button::ATTRIBUTES,
            Self::Light => light::ATTRIBUTES,
            Self::Remote => remote::ATTRIBUTES,
            Self::Scene => scene::ATTRIBUTES,
            Self::Script => script::ATTRIBUTES,
            Self::Sensor => sensor::ATTRIBUTES,
            Self::Siren => siren::ATTRIBUTES,
            Self::Switch => switch::ATTRIBUTES,
        }
    }
}

/// Whether an entity that turns on and off, such as a switch, is on;
/// none in any other state, such as `unknown` or `unavailable`.
#[must_use]
pub fn on_off(entity: &Entity) -> Option<bool> {
    match entity.state.as_deref() {
        Some(ON) => Some(true),
        Some(OFF) => Some(false),
        _ => None,
    }
}

/// The entity's state; none before the first one, or while the entity cannot be reached.
fn known_state(entity: &Entity) -> Option<&str> {
    entity
        .state
        .as_deref()
        .filter(|state| ![UNKNOWN, UNAVAILABLE].contains(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn only_an_entity_id_of_a_known_domain_names_one() {
        assert_eq!(Domain::of("automation.porch"), Some(Domain::Automation));
        assert_eq!(
            Domain::of("binary_sensor.front_door"),
            Some(Domain::BinarySensor)
        );
        assert_eq!(Domain::of("button.gate"), Some(Domain::Button));
        assert_eq!(
            Domain::of("input_boolean.guests"),
            Some(Domain::InputBoolean)
        );
        assert_eq!(Domain::of("input_button.gate"), Some(Domain::InputButton));
        assert_eq!(Domain::of("light.kitchen"), Some(Domain::Light));
        assert_eq!(Domain::of("remote.living_room"), Some(Domain::Remote));
        assert_eq!(Domain::of("scene.evening"), Some(Domain::Scene));
        assert_eq!(Domain::of("script.wake_up"), Some(Domain::Script));
        assert_eq!(Domain::of("sensor.outdoor"), Some(Domain::Sensor));
        assert_eq!(Domain::of("siren.hallway"), Some(Domain::Siren));
        assert_eq!(Domain::of("switch.kettle"), Some(Domain::Switch));
        for other in [
            "climate.living_room",
            "scene",
            "scene.",
            ".evening",
            "Scene.evening",
            "scene.evening light",
            "",
        ] {
            assert_eq!(Domain::of(other), None, "{other}");
        }
    }

    #[wasm_test]
    fn a_subscription_keeps_what_the_domains_readers_read() {
        assert_eq!(Domain::Script.attributes(), ["last_triggered"]);
        assert_eq!(Domain::Switch.attributes(), ["device_class"]);
        for bare in [
            Domain::Automation,
            Domain::Button,
            Domain::InputBoolean,
            Domain::InputButton,
            Domain::Remote,
            Domain::Scene,
            Domain::Siren,
        ] {
            assert!(bare.attributes().is_empty(), "{bare:?}");
        }
    }

    #[wasm_test]
    fn only_on_and_off_read_as_a_position() {
        let position = |state| on_off(&Entity::reporting(state, vec![]));
        assert_eq!(position("on"), Some(true));
        assert_eq!(position("off"), Some(false));
        for other in [UNKNOWN, UNAVAILABLE, "open"] {
            assert_eq!(position(other), None, "{other}");
        }
    }

    #[wasm_test]
    fn a_known_state_is_none_before_the_first_one_or_while_unreachable() {
        let at = |state| known_state(&Entity::reporting(state, vec![])).map(str::to_owned);
        assert_eq!(
            at("2026-11-13T10:00:00+00:00").as_deref(),
            Some("2026-11-13T10:00:00+00:00")
        );
        assert_eq!(at(UNKNOWN), None);
        assert_eq!(at(UNAVAILABLE), None);
    }
}
