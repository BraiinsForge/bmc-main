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

//! Scripts, which are on while they run.

use crate::{Entity, ServiceCall};

pub(super) const DOMAIN: &str = "script";
const LAST_TRIGGERED: &str = "last_triggered";
pub const ATTRIBUTES: &[&str] = &[LAST_TRIGGERED];
const RUNNING: &str = "on";

#[must_use]
pub fn turn_on() -> ServiceCall {
    ServiceCall::new(DOMAIN, "turn_on")
}

/// Stops every run the script has going.
#[must_use]
pub fn turn_off() -> ServiceCall {
    ServiceCall::new(DOMAIN, "turn_off")
}

#[must_use]
pub fn running(script: &Entity) -> bool {
    script.state.as_deref() == Some(RUNNING)
}

/// When the script last ran, as Home Assistant's ISO 8601 timestamp.
#[must_use]
pub fn last_triggered(script: &Entity) -> Option<&str> {
    script.text(LAST_TRIGGERED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AttributeValue;
    use crate::domain::UNAVAILABLE;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn the_last_run_is_read_from_a_tracked_attribute() {
        let at = "2026-11-13T10:00:00+00:00";
        let script = Entity::tracked(
            "off",
            vec![(LAST_TRIGGERED, AttributeValue::Text(at.to_owned()))],
            ATTRIBUTES,
        );
        assert_eq!(last_triggered(&script), Some(at));
    }

    #[wasm_test]
    fn only_a_script_that_is_on_runs() {
        assert!(running(&Entity::reporting("on", vec![])));
        assert!(!running(&Entity::reporting("off", vec![])));
        assert!(!running(&Entity::reporting(UNAVAILABLE, vec![])));
    }
}
