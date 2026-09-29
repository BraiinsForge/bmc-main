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

//! Nexus's next-launch endpoint: what a reply means for the launch the widget holds,
//! and the envelope read into a [`crate::model::LaunchData`].

pub const NEXUS_URL: &str = "https://nexus.braiinsforge.com/api/v1/data/spacex/next-launch";

/// How a poll reply classifies.
pub enum Reply<T> {
    Data(T),
    Empty,
    Error,
}

/// What to do with a classified reply.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome<T> {
    Store(T),
    NoLaunch,
    Keep,
    Fail,
}

/// Empty clears even when a launch is held; failure keeps the last launch, else errors.
#[must_use]
pub fn outcome<T>(reply: Reply<T>, has_data: bool) -> Outcome<T> {
    match reply {
        Reply::Data(data) => Outcome::Store(data),
        Reply::Empty => Outcome::NoLaunch,
        Reply::Error if has_data => Outcome::Keep,
        Reply::Error => Outcome::Fail,
    }
}

#[cfg(target_arch = "wasm32")]
pub use payload::LaunchParseError;

/// Wasm-only: `JsonDoc` and `parse_datetime` are host calls.
#[cfg(target_arch = "wasm32")]
mod payload {
    use bmc_wasm_sdk::{JsonDoc, parse_datetime};

    use crate::model::{
        LaunchData, abbreviate_mission_type, abbreviate_place, abbreviate_spacecraft,
        format_booster,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum LaunchParseError {
        InvalidDocument,
        /// A launch is present but its `net` timestamp could not be parsed.
        InvalidDate,
    }

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
                .map_or_else(|| "N/A".into(), |kind| abbreviate_mission_type(&kind));

            let spacecraft = doc
                .str("/data/rocket/spacecraft_stage/0/spacecraft/name")
                .map_or_else(|| "N/A".into(), |name| abbreviate_spacecraft(&name));

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_reply_is_stored() {
        assert_eq!(outcome(Reply::Data(7), false), Outcome::Store(7));
        assert_eq!(outcome(Reply::Data(7), true), Outcome::Store(7));
    }

    #[test]
    fn empty_reply_clears_regardless_of_prior_data() {
        assert_eq!(outcome::<i32>(Reply::Empty, false), Outcome::NoLaunch);
        assert_eq!(outcome::<i32>(Reply::Empty, true), Outcome::NoLaunch);
    }

    #[test]
    fn failure_keeps_data_when_present_else_errors() {
        assert_eq!(outcome::<i32>(Reply::Error, true), Outcome::Keep);
        assert_eq!(outcome::<i32>(Reply::Error, false), Outcome::Fail);
    }
}
