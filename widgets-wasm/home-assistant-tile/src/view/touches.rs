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

//! What fingers did as of the last frame, as the host reports it: the keys a finger was lifted on,
//! and the keys a finger is on. Each block reads its own keys out of it, so no caller parses a key.

use std::collections::HashMap;

use bmc_wasm_sdk::*;

#[derive(Debug, Clone, Default)]
pub struct Touches {
    released: HashMap<String, TouchHit>,
    held: HashMap<String, TouchHit>,
}

impl Touches {
    /// The touches `render_ui` reported in `result`.
    #[must_use]
    pub fn read(result: TreeRenderResult) -> Self {
        Self {
            released: result.clicks,
            held: result.drags,
        }
    }

    /// A finger lifted from `key` at `hit`.
    pub(crate) fn tap(key: &str, hit: TouchHit) -> Self {
        Self {
            released: HashMap::from([(key.to_owned(), hit)]),
            held: HashMap::new(),
        }
    }

    /// A finger on `key` at `hit`.
    pub(crate) fn hold(key: &str, hit: TouchHit) -> Self {
        Self {
            released: HashMap::new(),
            held: HashMap::from([(key.to_owned(), hit)]),
        }
    }

    pub(crate) fn released(&self, key: &str) -> Option<&TouchHit> {
        self.released.get(key)
    }

    /// The part of the block keyed `key` a finger was lifted from, and where.
    pub(crate) fn released_part(&self, key: &str) -> Option<(&str, &TouchHit)> {
        self.released
            .iter()
            .find_map(|(touched, hit)| Some((part_of(key, touched)?, hit)))
    }

    pub(crate) fn is_held(&self, key: &str) -> bool {
        self.held.contains_key(key)
    }
}

pub(crate) fn part_key(key: &str, part: &str) -> String {
    fmt!("{key}:{part}")
}

fn part_of<'a>(key: &str, touched: &'a str) -> Option<&'a str> {
    touched.strip_prefix(key)?.strip_prefix(':')
}

/// A touch in the middle of a target `width` by `height`, for a target that does not care where.
pub(crate) fn middle(width: f32, height: f32) -> TouchHit {
    TouchHit {
        x: width / 2.0,
        y: height / 2.0,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn only_the_blocks_own_parts_read_back() {
        let part = |touched: &str| {
            Touches::tap(touched, middle(10.0, 10.0))
                .released_part("c1")
                .map(|(part, _)| part.to_owned())
        };
        assert_eq!(part("c1:up").as_deref(), Some("up"));
        for other in ["c2:up", "c10:up", "c1", "card"] {
            assert_eq!(part(other), None, "{other}");
        }
    }

    #[wasm_test]
    fn a_held_key_is_not_released() {
        let touches = Touches::hold("c1", middle(10.0, 10.0));
        assert!(touches.is_held("c1"));
        assert!(touches.released("c1").is_none());
    }
}
