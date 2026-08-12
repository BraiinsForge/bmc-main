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

//! Launch data model and nexus payload parsing.

#[expect(clippy::wildcard_imports, reason = "widget code uses many SDK exports")]
use bmc_wasm_sdk::*;

/// One upcoming-launch snapshot from nexus, flattened to the strings the
/// panels render.
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

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchParseError {
    InvalidDocument,
    /// A launch is present but its `net` timestamp could not be parsed.
    InvalidDate,
}

#[cfg(target_arch = "wasm32")]
impl LaunchData {
    /// `Some` launch, `None` if none upcoming (`data: null`), `Err` if malformed.
    pub fn parse(doc: &JsonDoc) -> Result<Option<Self>, LaunchParseError> {
        if !doc.is_valid() {
            return Err(LaunchParseError::InvalidDocument);
        }

        // No `net` means nexus reports nothing upcoming.
        let Some(net) = doc.str("/data/net") else {
            return Ok(None);
        };
        let launch_unix = parse_datetime(&net).ok_or(LaunchParseError::InvalidDate)?;

        let mission_name = doc
            .str("/data/mission/name")
            .or_else(|| doc.str("/data/name"))
            .unwrap_or_else(|| "Unknown Mission".into());

        let status = doc.str("/data/status/name").unwrap_or_else(|| "TBD".into());

        let rocket = doc
            .str("/data/rocket/configuration/full_name")
            .or_else(|| doc.str("/data/rocket/configuration/name"))
            .unwrap_or_else(|| "Unknown".into());

        let location = doc
            .str("/data/pad/location/name")
            .unwrap_or_else(|| "Unknown".into());
        let pad = doc.str("/data/pad/name").unwrap_or_default();
        let place = abbreviate_place(&location, &pad);

        let landing = match doc.bool("/data/rocket/launcher_stage/0/landing/attempt") {
            Some(false) => "No attempt".into(),
            Some(true) => doc
                .str("/data/rocket/launcher_stage/0/landing/type/abbrev")
                .unwrap_or_else(|| "Unknown".into()),
            None => "Not confirmed".into(),
        };

        let booster = doc
            .i64("/data/rocket/launcher_stage/0/launcher_flight_number")
            .map_or_else(|| "N/A".into(), format_booster);

        let payload = doc
            .str("/data/mission/type")
            .unwrap_or_else(|| "N/A".into());

        let spacecraft = doc
            .str("/data/rocket/spacecraft_stage/0/spacecraft/name")
            .unwrap_or_else(|| "N/A".into());

        Ok(Some(Self {
            mission_name,
            launch_unix,
            status,
            rocket,
            place,
            landing,
            booster,
            payload,
            spacecraft,
        }))
    }
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
        fmt!("{}\u{00d7} flown", flights)
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
        assert_eq!(format_booster(3), "3\u{00d7} flown");
    }
}
