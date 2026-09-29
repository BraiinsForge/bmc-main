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

//! Which layout a viewport gets, and the launch the layouts draw.

use bmc_wasm_sdk::typography::TIMES;
use bmc_wasm_sdk::{SizeVariant, WidgetSize, WidgetViewport, fmt, ufmt};

/// The frames a layout is picked for: the four BMC100 slots and BMM101's 480×320.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeBucket {
    Full,
    Large,
    Medium,
    Small,
    Bmm101,
}

impl SizeBucket {
    #[must_use]
    pub const fn design_size(self) -> (u32, u32) {
        match self {
            Self::Full => (1_280, 480),
            Self::Large => (638, 480),
            Self::Medium => (638, 238),
            Self::Small => (317, 238),
            Self::Bmm101 => (480, 320),
        }
    }
}

/// A rectangular viewport classified once for every layout:
/// its pixels with their closest BMC100 variant, and the bucket that picks the layout.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub size: WidgetSize,
    pub bucket: SizeBucket,
}

impl Frame {
    #[must_use]
    pub fn of(viewport: WidgetViewport) -> Self {
        Self {
            size: WidgetSize::from_dimensions(viewport.width, viewport.height),
            bucket: size_bucket(viewport.width, viewport.height),
        }
    }
}

/// The two BMM frames the narrow bucket has to tell apart.
const BMM100_HEIGHT: u32 = 240;
const BMM101_HEIGHT: u32 = SizeBucket::Bmm101.design_size().1;
/// Split at their midpoint, so either frame keeps its bucket a few pixels either way.
const BMM101_MIN_HEIGHT: u32 = u32::midpoint(BMM100_HEIGHT, BMM101_HEIGHT);
const BMM101_MAX_WIDTH: u32 = SizeBucket::Bmm101.design_size().0;

/// A landscape frame no wider than BMM101 and at least as tall as the BMM split
/// is BMM101; everything else takes the closest BMC100 variant, as the SDK does.
#[must_use]
pub fn size_bucket(width: u32, height: u32) -> SizeBucket {
    if width <= BMM101_MAX_WIDTH && height < width && height >= BMM101_MIN_HEIGHT {
        return SizeBucket::Bmm101;
    }
    match SizeVariant::closest(width, height) {
        SizeVariant::Full => SizeBucket::Full,
        SizeVariant::Large => SizeBucket::Large,
        SizeVariant::Medium => SizeBucket::Medium,
        SizeVariant::Small => SizeBucket::Small,
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
    fn every_design_size_lands_in_its_own_bucket() {
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let (width, height) = bucket.design_size();
            assert_eq!(size_bucket(width, height), bucket);
        }
    }

    #[test]
    fn the_bmm101_bucket_ends_at_the_midpoint_of_the_bmm_heights_and_at_its_width() {
        assert_eq!(size_bucket(320, 240), SizeBucket::Small, "BMM100");
        assert_eq!(size_bucket(480, 280), SizeBucket::Bmm101);
        assert_eq!(size_bucket(480, 279), SizeBucket::Medium);
        assert_eq!(size_bucket(481, 320), SizeBucket::Large);
    }

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
