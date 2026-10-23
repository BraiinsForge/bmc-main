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

//! Entity state as a `subscribe_entities` subscription delivers it.
//!
//! Each event carries up to three parts, keyed by entity id:
//! `a` adds an entity with its full compressed state (`s` state, `a` attributes),
//! `c` changes one, `+` holding what was set and `-` the attributes removed,
//! and `r` lists the entities that went away.

use std::collections::BTreeMap;

use super::json::{JsonLookup, Pointer, PointerBuf};

/// Longest text attribute kept. A longer one reads as absent.
pub const MAX_ATTRIBUTE_TEXT_BYTES: usize = 1024;
/// Longest list of texts kept, counting one byte between each two of its texts.
/// WLED's 218 effect names come to about 2.2 KB. A longer list reads as absent.
pub const MAX_ATTRIBUTE_LIST_BYTES: usize = 4096;
/// Longest list of numbers kept, as much memory as the longest text.
/// A light's longest, `rgbww_color`, holds five. A longer list reads as absent.
pub const MAX_ATTRIBUTE_NUMBERS: usize = MAX_ATTRIBUTE_TEXT_BYTES / size_of::<f64>();

/// The value of one tracked attribute.
///
/// Objects are not kept, and an array only when it holds texts alone, such as a thermostat's `hvac_modes`,
/// or numbers alone, such as a light's `rgb_color`. An empty array is an empty [`Self::List`].
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
    Numbers(Vec<f64>),
}

impl AttributeValue {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Number(_) | Self::Bool(_) | Self::List(_) | Self::Numbers(_) => None,
        }
    }

    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Text(_) | Self::Bool(_) | Self::List(_) | Self::Numbers(_) => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(flag) => Some(*flag),
            Self::Text(_) | Self::Number(_) | Self::List(_) | Self::Numbers(_) => None,
        }
    }

    #[must_use]
    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            Self::List(items) => Some(items),
            Self::Text(_) | Self::Number(_) | Self::Bool(_) | Self::Numbers(_) => None,
        }
    }

    /// The numbers, also for an empty array, which reads as an empty [`Self::List`].
    #[must_use]
    pub fn as_numbers(&self) -> Option<&[f64]> {
        match self {
            Self::Numbers(items) => Some(items),
            Self::List(items) if items.is_empty() => Some(&[]),
            Self::Text(_) | Self::Number(_) | Self::Bool(_) | Self::List(_) => None,
        }
    }
}

/// Whether the entity exists in Home Assistant, as far as its subscription knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// The subscription's first event has not arrived yet.
    Awaiting,
    Present,
    /// It was present when the connection dropped: the values are the last ones seen,
    /// not current ones, until the next snapshot.
    Stale,
    /// Home Assistant does not know the entity, or removed it.
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub presence: Presence,
    /// The state as Home Assistant reports it, such as `on`, `21.5`, `unknown` or `unavailable`.
    pub state: Option<String>,
    /// When the state last changed, in seconds since the Unix epoch.
    /// An attribute changing alone does not move it.
    pub last_changed: Option<f64>,
    attributes: BTreeMap<String, AttributeValue>,
}

impl Entity {
    pub(crate) fn awaiting() -> Self {
        Self {
            presence: Presence::Awaiting,
            state: None,
            last_changed: None,
            attributes: BTreeMap::new(),
        }
    }

    /// The attribute `name`, `None` unless the subscription tracks it and the entity has it.
    #[must_use]
    pub fn attribute(&self, name: &str) -> Option<&AttributeValue> {
        self.attributes.get(name)
    }

    /// A text attribute, `None` when absent or not text.
    #[must_use]
    pub fn text(&self, name: &str) -> Option<&str> {
        self.attribute(name)?.as_text()
    }

    /// A numeric attribute, `None` when absent or not a number.
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.attribute(name)?.as_number()
    }

    /// A boolean attribute, such as a media player's `is_volume_muted`,
    /// `None` when absent or not a boolean.
    #[must_use]
    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.attribute(name)?.as_bool()
    }

    /// A list attribute, `None` when absent or not a list.
    #[must_use]
    pub fn list(&self, name: &str) -> Option<&[String]> {
        self.attribute(name)?.as_list()
    }

    /// A list of numbers, such as a light's `rgb_color`, `None` when absent or not one.
    #[must_use]
    pub fn numbers(&self, name: &str) -> Option<&[f64]> {
        self.attribute(name)?.as_numbers()
    }

    /// Mark the values of a present entity as last seen. Returns whether that changed anything.
    pub(crate) fn go_stale(&mut self) -> bool {
        let present = self.presence == Presence::Present;
        if present {
            self.presence = Presence::Stale;
        }
        present
    }

    /// Apply one `subscribe_entities` event to this entity.
    /// Returns whether anything the widget can read changed.
    ///
    /// The first event of a subscription is its snapshot:
    /// an entity missing from its `a` part does not exist.
    /// Any other event costs an entity it does not name two lookups and no allocation.
    pub(crate) fn apply(
        &mut self,
        entity_id: &str,
        tracked: &[String],
        event: &mut EntityEvent<'_, impl JsonLookup>,
    ) -> bool {
        let EntityEvent {
            message,
            removed,
            snapshot,
            path,
        } = event;
        let message = *message;
        let mut changed = false;
        let event_root = path.mark();

        path.push("a");
        path.push(entity_id);
        if message.len(path.as_str()).is_some() {
            let added = path.mark();
            changed |= replace(&mut self.presence, Presence::Present);
            changed |= replace(&mut self.state, message.str(path.push("s")));
            path.truncate(added);
            changed |= replace(&mut self.last_changed, message.f64(path.push("lc")));
            path.truncate(added);
            path.push("a");
            let attributes_at = path.mark();
            let mut attributes = BTreeMap::new();
            for name in tracked {
                path.truncate(attributes_at);
                path.push(name);
                if let Some(value) = read_value(message, path) {
                    attributes.insert(name.clone(), value);
                }
            }
            changed |= replace(&mut self.attributes, attributes);
        } else if *snapshot {
            changed |= self.forget();
        }
        path.truncate(event_root);

        path.push("c");
        path.push(entity_id);
        // As the reference client does: a change says nothing of an entity not held.
        if self.presence == Presence::Present && message.len(path.as_str()).is_some() {
            let changed_entity = path.mark();
            path.push("+");
            let set = path.mark();
            if let Some(state) = message.str(path.push("s")) {
                changed |= replace(&mut self.state, Some(state));
            }
            path.truncate(set);
            if let Some(changed_at) = message.f64(path.push("lc")) {
                changed |= replace(&mut self.last_changed, Some(changed_at));
            }
            path.truncate(set);
            path.push("a");
            let set_attributes = path.mark();
            for name in tracked {
                path.truncate(set_attributes);
                path.push(name);
                match read_value(message, path) {
                    Some(value) => changed |= self.set_attribute(name, value),
                    // Such as a light's brightness going null as it turns off:
                    // keeping the old value would show it as still current.
                    None if message.exists(path.as_str()) => changed |= self.remove_attribute(name),
                    None => {}
                }
            }
            path.truncate(changed_entity);
            path.push("-");
            let dropped_count = message.len(path.push("a")).unwrap_or(0);
            let dropped = path.mark();
            for index in 0..dropped_count {
                path.truncate(dropped);
                if let Some(name) = message.str(path.push_index(index)) {
                    changed |= self.remove_attribute(&name);
                }
            }
        }
        path.truncate(event_root);

        if removed.iter().any(|id| id == entity_id) {
            changed |= self.forget();
        }
        changed
    }

    /// Mark the entity missing. Returns whether that changed anything.
    fn forget(&mut self) -> bool {
        let changed = self.presence != Presence::Missing
            || self.state.is_some()
            || self.last_changed.is_some()
            || !self.attributes.is_empty();
        self.presence = Presence::Missing;
        self.state = None;
        self.last_changed = None;
        self.attributes.clear();
        changed
    }

    /// Returns whether the value changed.
    fn set_attribute(&mut self, name: &str, value: AttributeValue) -> bool {
        if let Some(slot) = self.attributes.get_mut(name) {
            replace(slot, value)
        } else {
            self.attributes.insert(name.to_owned(), value);
            true
        }
    }

    /// Returns whether the entity had it.
    fn remove_attribute(&mut self, name: &str) -> bool {
        self.attributes.remove(name).is_some()
    }
}

/// Set `slot` to `value`, returning whether it differed.
fn replace<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        false
    } else {
        *slot = value;
        true
    }
}

/// One `subscribe_entities` event, read once for every entity of a subscription:
/// its `r` part, and one pointer buffer the entities share.
pub(crate) struct EntityEvent<'a, J> {
    message: &'a J,
    /// The ids its `r` part lists.
    removed: Vec<String>,
    snapshot: bool,
    path: PointerBuf,
}

impl<'a, J: JsonLookup> EntityEvent<'a, J> {
    pub(crate) fn new(message: &'a J, snapshot: bool) -> Self {
        let mut path = PointerBuf::new(Pointer::root().key("event"));
        let event_root = path.mark();
        let removed_count = message.len(path.push("r")).unwrap_or(0);
        let removed_at = path.mark();
        let removed = (0..removed_count)
            .filter_map(|index| {
                path.truncate(removed_at);
                message.str(path.push_index(index))
            })
            .collect();
        path.truncate(event_root);
        Self {
            message,
            removed,
            snapshot,
            path,
        }
    }
}

/// The value at `path`, with text or a list past its cap, or a mixed array, read as absent.
/// Leaves `path` as it found it.
pub(crate) fn read_value(event: &impl JsonLookup, path: &mut PointerBuf) -> Option<AttributeValue> {
    if let Some(text) = event.str_max(path.as_str(), MAX_ATTRIBUTE_TEXT_BYTES) {
        return Some(AttributeValue::Text(text));
    }
    if let Some(number) = event.f64(path.as_str()) {
        return Some(AttributeValue::Number(number));
    }
    if let Some(flag) = event.bool(path.as_str()) {
        return Some(AttributeValue::Bool(flag));
    }
    if event.is_array(path.as_str()) {
        let array = path.mark();
        let numbers = event.f64(path.push_index(0)).is_some();
        path.truncate(array);
        let value = if numbers {
            read_numbers(event, path).map(AttributeValue::Numbers)
        } else {
            read_list(event, path).map(AttributeValue::List)
        };
        path.truncate(array);
        return value;
    }
    None
}

/// The texts of the array at `path`, `None` when it holds anything else or runs past the cap.
fn read_list(event: &impl JsonLookup, path: &mut PointerBuf) -> Option<Vec<String>> {
    let array = path.mark();
    let mut items = Vec::new();
    let mut bytes = 0;
    for index in 0..event.len(path.as_str())? {
        path.truncate(array);
        let separator = usize::from(index > 0);
        let room = MAX_ATTRIBUTE_LIST_BYTES.checked_sub(bytes + separator)?;
        let item = event.str_max(path.push_index(index), room)?;
        bytes += item.len() + separator;
        items.push(item);
    }
    Some(items)
}

/// The numbers of the array at `path`, `None` when it holds anything else or runs past the cap.
fn read_numbers(event: &impl JsonLookup, path: &mut PointerBuf) -> Option<Vec<f64>> {
    let len = event.len(path.as_str())?;
    if len > MAX_ATTRIBUTE_NUMBERS {
        return None;
    }
    let array = path.mark();
    (0..len)
        .map(|index| {
            path.truncate(array);
            event.f64(path.push_index(index))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::json::test_support::Message;
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    const ID: &str = "sensor.outdoor";

    fn tracked() -> Vec<String> {
        ["friendly_name", "unit_of_measurement", "device_class"]
            .map(str::to_owned)
            .to_vec()
    }

    fn apply_to(
        entity: &mut Entity,
        entity_id: &str,
        tracked: &[String],
        event: &Message,
        snapshot: bool,
    ) -> bool {
        entity.apply(entity_id, tracked, &mut EntityEvent::new(event, snapshot))
    }

    fn apply(entity: &mut Entity, event: &str, snapshot: bool) -> bool {
        apply_to(entity, ID, &tracked(), &Message::parse(event), snapshot)
    }

    fn present() -> Entity {
        let mut entity = Entity::awaiting();
        apply(
            &mut entity,
            r#"{"event":{"a":{"sensor.outdoor":{"s":"21.5","a":{
                "friendly_name":"Outdoor","unit_of_measurement":"°C","state_class":"measurement"
            },"c":"01HX","lc":1700000000.1}}}}"#,
            true,
        );
        entity
    }

    #[wasm_test]
    fn a_snapshot_keeps_the_state_and_only_the_tracked_attributes() {
        let entity = present();

        assert_eq!(entity.presence, Presence::Present);
        assert_eq!(entity.state.as_deref(), Some("21.5"));
        assert_eq!(entity.text("friendly_name"), Some("Outdoor"));
        assert_eq!(entity.text("unit_of_measurement"), Some("°C"));
        assert_eq!(
            entity.attribute("state_class"),
            None,
            "an untracked attribute is not kept"
        );
        assert_eq!(
            entity.attribute("device_class"),
            None,
            "a tracked one the entity lacks is absent"
        );
    }

    #[wasm_test]
    fn an_entity_missing_from_the_snapshot_does_not_exist() {
        let mut entity = Entity::awaiting();

        assert!(apply(&mut entity, r#"{"event":{"a":{}}}"#, true));
        assert_eq!(entity.presence, Presence::Missing);
    }

    #[wasm_test]
    fn the_same_attributes_reached_in_another_order_are_no_change() {
        let tracked = ["brightness", "color_mode"].map(str::to_owned);
        let mut entity = Entity::awaiting();
        let mut apply = |event: &str, snapshot| {
            apply_to(
                &mut entity,
                "light.desk",
                &tracked,
                &Message::parse(event),
                snapshot,
            )
        };
        apply(
            r#"{"event":{"a":{"light.desk":{"s":"on","a":{"color_mode":"hs"}}}}}"#,
            true,
        );
        apply(
            r#"{"event":{"c":{"light.desk":{"+":{"a":{"brightness":128}}}}}}"#,
            false,
        );

        assert!(
            !apply(
                r#"{"event":{"a":{"light.desk":{"s":"on","a":{"brightness":128,"color_mode":"hs"}}}}}"#,
                false,
            ),
            "the full state repeats what the entity already holds"
        );
    }

    #[wasm_test]
    fn a_boolean_attribute_reads_as_one_and_nothing_else_does() {
        let tracked = ["is_volume_muted", "volume_level"].map(str::to_owned);
        let mut entity = Entity::awaiting();
        apply_to(
            &mut entity,
            "media_player.den",
            &tracked,
            &Message::parse(
                r#"{"event":{"a":{"media_player.den":{"s":"on","a":{
                    "is_volume_muted":false,"volume_level":0.4}}}}}"#,
            ),
            true,
        );

        assert_eq!(entity.boolean("is_volume_muted"), Some(false));
        assert_eq!(
            entity.boolean("volume_level"),
            None,
            "a number is no boolean"
        );
        assert_eq!(
            entity.boolean("source"),
            None,
            "an absent attribute is none"
        );
    }

    #[wasm_test]
    fn a_change_to_an_entity_not_held_is_ignored() {
        let mut entity = Entity::awaiting();
        apply(&mut entity, r#"{"event":{"a":{}}}"#, true);

        assert!(!apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"+":{"s":"22","lc":1700000100.0}}}}}"#,
            false
        ));
        assert_eq!(entity.presence, Presence::Missing);
        assert_eq!(entity.state, None);
    }

    #[wasm_test]
    fn a_change_in_the_event_that_adds_the_entity_applies() {
        let mut entity = Entity::awaiting();
        apply(&mut entity, r#"{"event":{"a":{}}}"#, true);

        assert!(apply(
            &mut entity,
            r#"{"event":{"a":{"sensor.outdoor":{"s":"21","a":{}}},
                         "c":{"sensor.outdoor":{"+":{"s":"22"}}}}}"#,
            false
        ));
        assert_eq!(entity.presence, Presence::Present);
        assert_eq!(entity.state.as_deref(), Some("22"));
    }

    #[wasm_test]
    fn a_later_event_without_the_entity_leaves_it_alone() {
        let mut entity = present();

        assert!(!apply(
            &mut entity,
            r#"{"event":{"a":{"sensor.other":{"s":"1","a":{}}}}}"#,
            false
        ));
        assert_eq!(entity.presence, Presence::Present);
    }

    #[wasm_test]
    fn a_change_sets_the_state_and_attributes_it_carries() {
        let mut entity = present();

        assert!(apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"+":{"s":"22.0","lu":1700000060.0,
                "a":{"device_class":"temperature","icon":"mdi:sun"}}}}}}"#,
            false,
        ));
        assert_eq!(entity.state.as_deref(), Some("22.0"));
        assert_eq!(entity.text("device_class"), Some("temperature"));
        assert_eq!(
            entity.text("friendly_name"),
            Some("Outdoor"),
            "attributes not in the change stay"
        );
        assert_eq!(
            entity.attribute("icon"),
            None,
            "an untracked attribute is ignored on change too"
        );
    }

    #[wasm_test]
    fn a_snapshot_and_a_state_change_set_when_the_state_changed() {
        let mut entity = present();
        assert_eq!(entity.last_changed, Some(1_700_000_000.1));

        assert!(apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"+":{"s":"22.0","lc":1700000060.5}}}}}"#,
            false,
        ));
        assert_eq!(entity.last_changed, Some(1_700_000_060.5));
    }

    #[wasm_test]
    fn an_update_that_leaves_the_state_alone_keeps_when_it_changed() {
        let mut entity = present();

        assert!(!apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"+":{"lu":1700000099.0}}}}}"#,
            false,
        ));
        assert_eq!(entity.last_changed, Some(1_700_000_000.1));
    }

    #[wasm_test]
    fn a_change_removes_the_attributes_it_lists() {
        let mut entity = present();

        assert!(apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"-":{"a":["unit_of_measurement","state_class"]}}}}}"#,
            false,
        ));
        assert_eq!(entity.attribute("unit_of_measurement"), None);
        assert_eq!(entity.text("friendly_name"), Some("Outdoor"));
    }

    #[wasm_test]
    fn a_change_to_null_clears_the_attribute() {
        let mut entity = present();

        assert!(apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"+":{"a":{"unit_of_measurement":null}}}}}}"#,
            false,
        ));
        assert_eq!(entity.attribute("unit_of_measurement"), None);
        assert_eq!(entity.text("friendly_name"), Some("Outdoor"));
    }

    #[wasm_test]
    fn a_change_to_a_value_not_kept_clears_the_attribute() {
        for value in [
            serde_json::json!({"nested": "object"}),
            serde_json::json!("x".repeat(MAX_ATTRIBUTE_TEXT_BYTES + 1)),
            serde_json::json!(["text", 1]),
        ] {
            let mut entity = present();
            let event = serde_json::json!({"event":{"c":{ID:{"+":{"a":{"friendly_name":value}}}}}});

            assert!(apply(&mut entity, &event.to_string(), false), "{value}");
            assert_eq!(entity.attribute("friendly_name"), None, "{value}");
        }
    }

    #[wasm_test]
    fn a_change_that_touches_only_untracked_parts_changes_nothing() {
        let mut entity = present();

        assert!(!apply(
            &mut entity,
            r#"{"event":{"c":{"sensor.outdoor":{"+":{"lu":1700000099.0,"a":{"icon":"mdi:x"}}}}}}"#,
            false,
        ));
    }

    #[wasm_test]
    fn a_removal_forgets_the_entity() {
        let mut entity = present();

        assert!(apply(
            &mut entity,
            r#"{"event":{"r":["sensor.other","sensor.outdoor"]}}"#,
            false
        ));
        assert_eq!(entity.presence, Presence::Missing);
        assert_eq!(entity.state, None);
        assert_eq!(entity.last_changed, None);
        assert_eq!(entity.attribute("friendly_name"), None);
    }

    #[wasm_test]
    fn values_keep_their_json_type() {
        let mut entity = Entity::awaiting();
        apply_to(
            &mut entity,
            "light.desk",
            &[
                "brightness".to_owned(),
                "friendly_name".to_owned(),
                "is_on".to_owned(),
            ],
            &Message::parse(
                r#"{"event":{"a":{"light.desk":{"s":"on","a":{"brightness":180,"friendly_name":"Desk","is_on":true}}}}}"#,
            ),
            true,
        );

        assert_eq!(entity.number("brightness"), Some(180.0));
        assert_eq!(entity.attribute("is_on"), Some(&AttributeValue::Bool(true)));
        assert_eq!(
            entity.number("friendly_name"),
            None,
            "text does not read as a number"
        );
    }

    fn thermostat(attributes: &serde_json::Value) -> Entity {
        let mut entity = Entity::awaiting();
        apply_to(
            &mut entity,
            "climate.living_room",
            &["hvac_modes".to_owned()],
            &Message::parse(
                &serde_json::json!({"event":{"a":{"climate.living_room":{"s":"heat","a":attributes}}}})
                    .to_string(),
            ),
            true,
        );
        entity
    }

    #[wasm_test]
    fn an_array_of_texts_is_kept_as_a_list() {
        let entity = thermostat(&serde_json::json!({"hvac_modes": ["off", "heat", "auto"]}));

        assert_eq!(
            entity.list("hvac_modes"),
            Some(["off", "heat", "auto"].map(str::to_owned).as_slice())
        );
        assert_eq!(entity.text("hvac_modes"), None);
    }

    #[wasm_test]
    fn an_empty_array_is_an_empty_list() {
        let entity = thermostat(&serde_json::json!({"hvac_modes": []}));

        assert_eq!(entity.list("hvac_modes"), Some([].as_slice()));
    }

    #[wasm_test]
    fn a_mixed_or_nested_array_reads_as_absent() {
        for modes in [
            serde_json::json!(["off", 1]),
            serde_json::json!([1, "off"]),
            serde_json::json!([["off"]]),
            serde_json::json!({"0": "off"}),
        ] {
            let entity = thermostat(&serde_json::json!({ "hvac_modes": modes }));
            assert_eq!(entity.attribute("hvac_modes"), None, "{modes}");
        }
    }

    fn light(rgb_color: &serde_json::Value) -> Entity {
        let mut entity = Entity::awaiting();
        apply_to(
            &mut entity,
            "light.desk",
            &["rgb_color".to_owned()],
            &Message::parse(
                &serde_json::json!({"event":{"a":{"light.desk":{"s":"on","a":{"rgb_color":rgb_color}}}}})
                    .to_string(),
            ),
            true,
        );
        entity
    }

    #[wasm_test]
    fn an_array_of_numbers_is_kept_as_numbers() {
        let entity = light(&serde_json::json!([255, 128, 0.5]));

        assert_eq!(
            entity.numbers("rgb_color"),
            Some([255.0, 128.0, 0.5].as_slice())
        );
        assert_eq!(entity.list("rgb_color"), None);
    }

    #[wasm_test]
    fn an_empty_array_reads_as_no_numbers_too() {
        assert_eq!(
            light(&serde_json::json!([])).numbers("rgb_color"),
            Some([].as_slice())
        );
    }

    #[wasm_test]
    fn numbers_past_the_cap_read_as_absent() {
        let at_cap = vec![1; MAX_ATTRIBUTE_NUMBERS];
        assert!(
            light(&serde_json::json!(at_cap))
                .numbers("rgb_color")
                .is_some()
        );

        let past_cap = vec![1; MAX_ATTRIBUTE_NUMBERS + 1];
        assert_eq!(
            light(&serde_json::json!(past_cap)).attribute("rgb_color"),
            None
        );
    }

    #[wasm_test]
    fn a_list_past_the_cap_reads_as_absent() {
        let item = "x".repeat(MAX_ATTRIBUTE_LIST_BYTES / 2);
        let shorter = "x".repeat(MAX_ATTRIBUTE_LIST_BYTES / 2 - 1);
        let at_cap = thermostat(&serde_json::json!({"hvac_modes": [item, shorter]}));
        assert!(
            at_cap.list("hvac_modes").is_some(),
            "two halves and their separator fit exactly"
        );

        let past_cap = thermostat(&serde_json::json!({"hvac_modes": [item, item]}));
        assert_eq!(past_cap.attribute("hvac_modes"), None);
    }

    #[wasm_test]
    fn a_change_replaces_a_list() {
        let mut entity = thermostat(&serde_json::json!({"hvac_modes": ["off", "heat"]}));

        assert!(apply_to(
            &mut entity,
            "climate.living_room",
            &["hvac_modes".to_owned()],
            &Message::parse(
                r#"{"event":{"c":{"climate.living_room":{"+":{"a":{"hvac_modes":["off","cool"]}}}}}}"#
            ),
            false,
        ));
        assert_eq!(
            entity.list("hvac_modes"),
            Some(["off", "cool"].map(str::to_owned).as_slice())
        );
    }

    #[wasm_test]
    fn an_oversized_text_attribute_reads_as_absent() {
        let long = "x".repeat(MAX_ATTRIBUTE_TEXT_BYTES + 1);
        let mut entity = Entity::awaiting();
        apply_to(
            &mut entity,
            ID,
            &tracked(),
            &Message::parse(
                &serde_json::json!({"event":{"a":{ID:{"s":"1","a":{"friendly_name":long}}}}})
                    .to_string(),
            ),
            true,
        );

        assert_eq!(entity.attribute("friendly_name"), None);
    }

    #[wasm_test]
    fn an_entity_id_with_a_slash_is_looked_up_escaped() {
        let mut entity = Entity::awaiting();
        apply_to(
            &mut entity,
            "odd/id",
            &[],
            &Message::parse(r#"{"event":{"a":{"odd/id":{"s":"on","a":{}}}}}"#),
            true,
        );

        assert_eq!(entity.state.as_deref(), Some("on"));
    }

    #[wasm_test]
    fn going_stale_marks_only_a_present_entity() {
        let mut entity = present();
        assert!(entity.go_stale());
        assert_eq!(entity.presence, Presence::Stale);
        assert!(!entity.go_stale(), "already stale changes nothing");

        let mut awaiting = Entity::awaiting();
        assert!(
            !awaiting.go_stale(),
            "an entity never seen has no old values to mark"
        );
    }

    #[wasm_test]
    fn the_next_snapshot_clears_staleness() {
        let mut entity = present();
        entity.go_stale();

        assert!(apply(
            &mut entity,
            r#"{"event":{"a":{"sensor.outdoor":{"s":"21.5","a":{
            "friendly_name":"Outdoor","unit_of_measurement":"°C"}}}}}"#,
            true
        ));
        assert_eq!(entity.presence, Presence::Present);
    }

    #[wasm_test]
    fn a_stale_entity_the_snapshot_lacks_is_missing() {
        let mut entity = present();
        entity.go_stale();

        assert!(apply(&mut entity, r#"{"event":{"a":{}}}"#, true));
        assert_eq!(entity.presence, Presence::Missing);
    }
}
