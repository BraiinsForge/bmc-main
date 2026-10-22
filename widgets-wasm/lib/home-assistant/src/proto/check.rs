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

//! The shapes a request's names, ids and times must have before anything is sent.

use std::collections::BTreeSet;

use super::client::{
    MAX_ENTITIES_PER_SUBSCRIPTION, MAX_ENTITY_ID_BYTES, MAX_NAME_BYTES, RequestError,
};

/// Home Assistant's slug shape, which domains, services and both halves of an entity id share:
/// lowercase letters, digits and underscores, no leading, trailing or doubled underscore.
pub(super) fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !text.starts_with('_')
        && !text.ends_with('_')
        && !text.contains("__")
}

/// `entity_ids` without repeats, so the limits count each once.
pub(super) fn distinct<'a>(entity_ids: &[&'a str]) -> BTreeSet<&'a str> {
    entity_ids.iter().copied().collect()
}

/// Between one and [`MAX_ENTITIES_PER_SUBSCRIPTION`] entity ids, each well formed.
pub(super) fn check_entity_ids(entity_ids: &BTreeSet<&str>) -> Result<(), RequestError> {
    if entity_ids.is_empty() {
        return Err(RequestError::NoEntities);
    }
    if entity_ids.len() > MAX_ENTITIES_PER_SUBSCRIPTION {
        return Err(RequestError::TooManyEntities);
    }
    if !entity_ids.iter().all(|id| is_entity_id(id)) {
        return Err(RequestError::InvalidEntityId);
    }
    Ok(())
}

/// A domain or a service, which Home Assistant requires to be slugs.
pub(super) fn is_name(text: &str) -> bool {
    text.len() <= MAX_NAME_BYTES && is_slug(text)
}

/// A command type or a service data key, which Home Assistant takes as any text.
pub(super) fn is_free_name(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_NAME_BYTES
}

/// Whether `text` has the shape of an entity id, such as `sensor.outdoor_temperature`.
#[must_use]
pub fn is_entity_id(text: &str) -> bool {
    text.len() <= MAX_ENTITY_ID_BYTES
        && text
            .split_once('.')
            .is_some_and(|(domain, object)| is_slug(domain) && is_slug(object))
}
