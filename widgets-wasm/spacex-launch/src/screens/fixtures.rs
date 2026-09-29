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

//! Launches, states and viewports to stage the layouts at, off-device.

use bmc_wasm_sdk::{ViewportShape, WidgetViewport};

use crate::model::{
    LaunchData, SizeBucket, State, abbreviate_mission_type, abbreviate_place,
    abbreviate_spacecraft, format_booster,
};
use crate::screens::ViewData;

/// One state, drawn into whichever viewport it is handed.
pub type StateFixture = fn(WidgetViewport) -> ViewData;

/// 18 June 2026, 12:11:21 UTC — when the capture fixtures were recorded.
const NOW: i64 = 1_781_784_681;

/// 19 June 2026, 08:40 UTC — the `net` Nexus served at [`NOW`].
const RECORDED_NET: i64 = 1_781_858_400;

/// Three-digit days out.
const DISTANT_NET: i64 = NOW + 888 * 86_400 + 8 * 3_600 + 8 * 60 + 8;

/// Figma's long mission name, too wide for one line at the hero's top size.
pub const LONG_MISSION: &str = "Transporter 17 (Dedicated SSO Rideshare)";

/// A launch without a mission block reads as its own name: the rocket, then the mission.
pub const LONGEST_MISSION: &str = "Falcon 9 Block 5 | Transporter 17 (Dedicated SSO Rideshare)";

/// What Nexus served at [`NOW`], as the parser reads it.
fn recorded() -> LaunchData {
    LaunchData {
        mission_name: "NROL-179".to_owned(),
        launch_unix: RECORDED_NET,
        status: "Go for Launch".to_owned(),
        rocket: "Falcon 9 Block 5".to_owned(),
        place: abbreviate_place("Vandenberg SFB, CA, USA", "Space Launch Complex 4E"),
        landing: "RTLS".to_owned(),
        booster: format_booster(3),
        payload: abbreviate_mission_type("Government/Top Secret"),
        spacecraft: "N/A".to_owned(),
    }
}

#[must_use]
pub fn rectangular(width: u32, height: u32) -> WidgetViewport {
    WidgetViewport {
        width,
        height,
        shape: ViewportShape::Rectangular,
    }
}

#[must_use]
pub fn at_bucket(bucket: SizeBucket) -> WidgetViewport {
    let (width, height) = bucket.design_size();
    rectangular(width, height)
}

fn view(viewport: WidgetViewport, state: State) -> ViewData {
    ViewData {
        viewport,
        state,
        now_secs: NOW,
    }
}

#[must_use]
pub fn healthy(viewport: WidgetViewport) -> ViewData {
    view(viewport, State::Loaded(recorded()))
}

#[must_use]
pub fn loading(viewport: WidgetViewport) -> ViewData {
    view(viewport, State::Loading)
}

/// A valid reply with nothing upcoming.
#[must_use]
pub fn no_launch(viewport: WidgetViewport) -> ViewData {
    view(viewport, State::NoLaunch)
}

/// Nexus's cold start, before any launch has loaded.
#[must_use]
pub fn failed(viewport: WidgetViewport) -> ViewData {
    view(
        viewport,
        State::Error("API request failed (503)".to_owned()),
    )
}

/// The recorded launch once its `net` has passed.
#[must_use]
pub fn launched(viewport: WidgetViewport) -> ViewData {
    ViewData {
        now_secs: RECORDED_NET,
        ..healthy(viewport)
    }
}

fn named(viewport: WidgetViewport, mission_name: &str) -> ViewData {
    let launch = LaunchData {
        mission_name: mission_name.to_owned(),
        ..recorded()
    };
    view(viewport, State::Loaded(launch))
}

#[must_use]
pub fn long_mission(viewport: WidgetViewport) -> ViewData {
    named(viewport, LONG_MISSION)
}

#[must_use]
pub fn longest_mission(viewport: WidgetViewport) -> ViewData {
    named(viewport, LONGEST_MISSION)
}

/// The widest value every row can show.
/// Any status wider than `To Be Determined` comes only after the net time,
/// when the view reads `Launched` instead.
#[must_use]
pub fn widest_values(viewport: WidgetViewport) -> ViewData {
    let launch = LaunchData {
        mission_name: LONGEST_MISSION.to_owned(),
        launch_unix: DISTANT_NET,
        status: "To Be Determined".to_owned(),
        rocket: "Falcon 9 Block 5".to_owned(),
        place: abbreviate_place("SpaceX Starbase, TX, USA", "Orbital Launch Pad 2"),
        landing: "Not confirmed".to_owned(),
        booster: format_booster(29),
        payload: abbreviate_mission_type("Communications"),
        spacecraft: abbreviate_spacecraft("Cargo Dragon"),
    };
    view(viewport, State::Loaded(launch))
}
