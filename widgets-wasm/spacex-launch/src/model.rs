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

//! The frames the layouts are designed at, and the launch they draw.

use bmc_wasm_sdk::typography::TIMES;
use bmc_wasm_sdk::{fmt, ufmt};

/// The frames the layouts are designed at: the four BMC100 slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Full,
    Large,
    Medium,
    Small,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (u32, u32) {
        match self {
            Self::Full => (1_280, 480),
            Self::Large => (638, 480),
            Self::Medium => (638, 238),
            Self::Small => (317, 238),
        }
    }
}

/// What the widget holds of the next launch.
#[derive(Clone, Debug)]
pub enum State {
    Loading,
    Loaded(LaunchData),
    NoLaunch,
    Error(String),
}

/// One upcoming-launch snapshot from nexus, flattened to the strings the
/// panels render.
#[derive(Clone, Debug)]
pub struct LaunchData {
    pub mission_name: String,
    pub launch_unix: i64,
    pub status: String,
    pub rocket: String,
    pub place: String,
    pub landing: String,
    pub booster: String,
    pub payload: String,
    pub spacecraft: String,
}

/// Compact "site pad" label, abbreviating known SpaceX sites and pads.
#[must_use]
pub fn abbreviate_place(location: &str, pad: &str) -> String {
    let loc = if location.contains("Cape Canaveral") {
        "CCSFS"
    } else if location.contains("Kennedy") {
        "KSC"
    } else if location.contains("Vandenberg") {
        "VSFB"
    } else if location.contains("Starbase") || location.contains("SpaceX") {
        "Starbase"
    } else {
        location
    };
    if pad.is_empty() {
        loc.into()
    } else {
        let short_pad = pad
            .replace("Space Launch Complex ", "SLC-")
            .replace("Launch Complex ", "LC-")
            .replace("Orbital Launch Mount ", "OLM-");
        fmt!("{} {}", loc, short_pad)
    }
}

/// "Flight #1" on debut, else a flown count.
#[must_use]
pub fn format_booster(flights: i64) -> String {
    if flights <= 1 {
        "Flight #1".into()
    } else {
        fmt!("{flights}{TIMES} flown")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviates_known_sites_and_pads() {
        assert_eq!(
            abbreviate_place("Cape Canaveral SFS, FL, USA", "Space Launch Complex 40"),
            "CCSFS SLC-40"
        );
        assert_eq!(
            abbreviate_place("Vandenberg SFB, CA, USA", "Space Launch Complex 4E"),
            "VSFB SLC-4E"
        );
        // An unknown location passes through; an empty pad drops the suffix.
        assert_eq!(abbreviate_place("Wallops Island", ""), "Wallops Island");
    }

    #[test]
    fn booster_reads_first_flight_then_flown_count() {
        assert_eq!(format_booster(1), "Flight #1");
        assert_eq!(format_booster(0), "Flight #1");
        assert_eq!(format_booster(3), format!("3{TIMES} flown"));
    }
}
