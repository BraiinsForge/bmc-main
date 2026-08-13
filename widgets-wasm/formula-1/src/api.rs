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

//! The Nexus Formula 1 resources this widget reads, and the JSON keys
//! their payloads use.

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "API code uses the SDK's macros and re-exports"
    )
)]
use bmc_wasm_sdk::*;

use crate::manifest_params::View;

/// Nexus deployment serving the Formula 1 resources.
pub const BASE_URL: &str = "https://nexus.braiinsforge.com";

/// Cadence for resources that change on the order of minutes —
/// standings, driver data, and the race calendar.
pub const STATIC_INTERVAL_MS: u32 = 60_000;
/// Cadence for a session that is running,
/// matching the shortest TTL the live resources advertise.
pub const LIVE_INTERVAL_MS: u32 = 3_000;
/// Cadence for the live resources while no session is running.
/// They answer `{"live": false}` all week,
/// so polling three of them at the live cadence would spend
/// the whole off-season asking a question
/// whose answer changes a handful of times a year.
pub const LIVE_PROBE_INTERVAL_MS: u32 = 60_000;

/// Every resource the widget polls.
/// The order is the handle order in [`crate::live`] —
/// a poll's handle index maps back to its resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    Standings,
    NextRace,
    DriverStats,
    Driver,
    LiveRace,
    LiveQuali,
    LivePractice,
}

impl Resource {
    pub const ALL: [Self; 7] = [
        Self::Standings,
        Self::NextRace,
        Self::DriverStats,
        Self::Driver,
        Self::LiveRace,
        Self::LiveQuali,
        Self::LivePractice,
    ];

    /// Name for log lines; guest logging formats with ufmt,
    /// which has no `Debug`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Standings => "standings",
            Self::NextRace => "next-race",
            Self::DriverStats => "driver-stats",
            Self::Driver => "driver",
            Self::LiveRace => "live-race",
            Self::LiveQuali => "live-quali",
            Self::LivePractice => "live-practice",
        }
    }

    /// Whether this resource reports a session in progress,
    /// and so follows the live cadence rather than the static one.
    #[must_use]
    pub fn is_live_board(self) -> bool {
        matches!(self, Self::LiveRace | Self::LiveQuali | Self::LivePractice)
    }

    /// Full request URL. The per-driver resource is keyed by the slug
    /// the operator picked, so it is the only one that varies.
    #[must_use]
    pub fn url(self, driver_slug: &str) -> String {
        if self == Self::Driver {
            fmt!("{BASE_URL}/api/v1/data/formula-1/driver/{}", driver_slug)
        } else {
            fmt!("{BASE_URL}/api/v1/data/formula-1/{}", self.name())
        }
    }
}

/// Whether `view` reads `resource`,
/// so a poll nothing renders stays closed.
///
/// The automatic view walks live session → next race → standings,
/// so it needs all three even though it shows one;
/// the explicit views need only their own.
#[must_use]
pub fn resource_needed(resource: Resource, view: View) -> bool {
    match view {
        View::Auto => {
            resource.is_live_board() || matches!(resource, Resource::NextRace | Resource::Standings)
        }
        View::NextRace => resource == Resource::NextRace,
        View::Standings => resource == Resource::Standings,
        // The statistics screen joins the per-driver resource
        // with the all-drivers table, so it reads both.
        View::Driver => matches!(resource, Resource::Driver | Resource::DriverStats),
    }
}

/// Which screen the widget draws. This build has no live screens,
/// so a running session shows as the race it is part of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    NextRace,
    Standings,
    Driver,
}

/// The screen for `view` over the data at hand.
///
/// An explicit view always wins, shown empty until its data arrives.
/// `Auto` walks the fallback chain:
/// the next race while one is announced, else the standings.
#[must_use]
pub fn select_screen(view: View, data: &crate::model::Data) -> Screen {
    match view {
        View::NextRace => Screen::NextRace,
        View::Driver => Screen::Driver,
        View::Auto if data.next_race.is_some() => Screen::NextRace,
        View::Standings | View::Auto => Screen::Standings,
    }
}

/// JSON pointers into a Nexus reply.
pub mod wire {
    /// Present and `false` when a live resource has no session running.
    pub const LIVE_FLAG: &str = "/data/live";

    // `fmt!` expands against the SDK's re-exports, which a nested
    // module does not inherit from the file above it.
    #[cfg_attr(
        not(test),
        expect(
            clippy::wildcard_imports,
            reason = "API code uses the SDK's macros and re-exports"
        )
    )]
    use super::*;

    /// Field of the `data` object at `index`, for the array payloads.
    #[must_use]
    pub fn row(index: usize, field: &str) -> String {
        fmt!("/data/{}/{}", index, field)
    }

    /// Field of a `data` object, for the single-object payloads.
    #[must_use]
    pub fn field(name: &str) -> String {
        fmt!("/data/{}", name)
    }

    /// Field of the `index`-th session of the race weekend.
    #[must_use]
    pub fn session(index: usize, field: &str) -> String {
        fmt!("/data/sessions/{}/{}", index, field)
    }

    /// Field of the `index`-th row of a timing board.
    #[must_use]
    pub fn entry(index: usize, field: &str) -> String {
        fmt!("/data/entries/{}/{}", index, field)
    }

    /// Field of one sector — `which` is 1, 2, or 3 — of the `index`-th
    /// row of a timing board.
    #[must_use]
    pub fn sector(index: usize, which: u8, field: &str) -> String {
        fmt!("/data/entries/{}/sector{}/{}", index, which, field)
    }
}

#[cfg(test)]
mod tests {
    use super::{BASE_URL, Resource, Screen, View, resource_needed, select_screen};
    use crate::model::Data;
    use crate::screens::fixtures;

    #[test]
    fn an_explicit_view_wins_over_whatever_data_holds() {
        let empty = Data::default();
        assert_eq!(select_screen(View::Driver, &empty), Screen::Driver);
        assert_eq!(select_screen(View::NextRace, &empty), Screen::NextRace);
        assert_eq!(select_screen(View::Standings, &empty), Screen::Standings);
    }

    #[test]
    fn the_automatic_view_shows_the_race_while_one_is_announced() {
        let data = Data {
            next_race: Some(fixtures::next_race_weekend()),
            ..Data::default()
        };
        assert_eq!(select_screen(View::Auto, &data), Screen::NextRace);
    }

    #[test]
    fn the_automatic_view_falls_to_the_standings_between_seasons() {
        assert_eq!(
            select_screen(View::Auto, &Data::default()),
            Screen::Standings
        );
    }

    #[test]
    fn every_resource_is_addressable_and_named_once() {
        let mut names: Vec<&str> = Resource::ALL.iter().map(|r| r.name()).collect();
        names.sort_unstable();
        let total = names.len();
        names.dedup();
        assert_eq!(names.len(), total, "two resources share a name");
    }

    #[test]
    fn the_per_driver_url_carries_the_slug_and_the_rest_do_not() {
        assert_eq!(
            Resource::Driver.url("hamilton"),
            format!("{BASE_URL}/api/v1/data/formula-1/driver/hamilton"),
        );
        assert_eq!(
            Resource::Standings.url("hamilton"),
            format!("{BASE_URL}/api/v1/data/formula-1/standings"),
        );
    }

    #[test]
    fn an_explicit_view_opens_only_what_it_renders() {
        for view in [View::NextRace, View::Standings, View::Driver] {
            let open: Vec<&str> = Resource::ALL
                .iter()
                .filter(|r| resource_needed(**r, view))
                .map(|r| r.name())
                .collect();
            assert!(
                !open.iter().any(|name| name.starts_with("live-")),
                "{} polled a session board it never shows: {open:?}",
                view.as_manifest_value(),
            );
        }
    }

    #[test]
    fn the_automatic_view_opens_its_whole_fallback_chain() {
        // Live session, else next race, else standings —
        // it has to watch all three to know which to show.
        for resource in [
            Resource::LiveRace,
            Resource::LiveQuali,
            Resource::LivePractice,
            Resource::NextRace,
            Resource::Standings,
        ] {
            assert!(
                resource_needed(resource, View::Auto),
                "the automatic view must watch {}",
                resource.name(),
            );
        }
        assert!(
            !resource_needed(Resource::Driver, View::Auto),
            "the automatic view never shows a driver card",
        );
    }

    #[test]
    fn the_driver_view_reads_both_halves_of_the_statistics() {
        // The per-driver resource carries the slug, the table the full
        // figures; the screen needs the join of the two.
        assert!(resource_needed(Resource::Driver, View::Driver));
        assert!(resource_needed(Resource::DriverStats, View::Driver));
    }

    #[test]
    fn only_the_session_boards_follow_the_live_cadence() {
        for resource in Resource::ALL {
            let live = matches!(
                resource,
                Resource::LiveRace | Resource::LiveQuali | Resource::LivePractice
            );
            assert_eq!(
                resource.is_live_board(),
                live,
                "{} is classified wrongly",
                resource.name(),
            );
        }
    }
}
