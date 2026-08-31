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

//! The widget's remote images, resolved out of the asset cache.
//!
//! Every image the payloads point at is cached as decoded RGBA under a
//! URL-derived tag, with the URL as the entry's identity. [`resolve`]
//! turns a URL back into a drawable bitmap — the deck restores what its
//! fetch pipeline cached, the gallery what a scene seeded — and the
//! screens stay agnostic about which of the two filled the cache.

use std::cell::RefCell;
use std::collections::HashMap;

#[cfg_attr(
    not(test),
    expect(
        clippy::wildcard_imports,
        reason = "widget code uses many SDK types, macros, and helpers"
    )
)]
use bmc_wasm_sdk::*;

use crate::model::ImageUrl;

/// What an image shows, which sizes its decode and names its tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Headshot,
    TeamLogo,
    Flag,
    Circuit,
}

impl ImageKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Headshot => "headshot",
            Self::TeamLogo => "team-logo",
            Self::Flag => "flag",
            Self::Circuit => "circuit",
        }
    }

    /// The decode bound: the largest box the design draws the kind in,
    /// never the viewport, which would cache pixels nothing shows.
    #[must_use]
    pub fn decode_size(self) -> (u32, u32) {
        match self {
            Self::Headshot => (327, 330),
            Self::TeamLogo => (40, 40),
            Self::Flag => (40, 28),
            Self::Circuit => (560, 380),
        }
    }
}

/// The cache tag for `url`: stable across restarts, so a wake finds
/// what the previous run cached, and free of the separators cache keys
/// reject.
#[must_use]
pub fn tag_for(kind: ImageKind, url: &ImageUrl) -> String {
    fmt!("f1-{}-{}", kind.as_str(), fnv1a(url.as_str().as_bytes()))
}

/// FNV-1a, chosen over the std hasher because the tag persists on the
/// deck's flash — std documents its output as unstable across releases,
/// which would orphan every cached image on a toolchain bump.
fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// A resolved image: its bitmap and the pixel size it was decoded at,
/// for boxes that follow the image's aspect rather than fix their own.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub bitmap: BitmapId,
    pub width: u32,
    pub height: u32,
}

thread_local! {
    /// Resolved bitmaps by tag. Restoring uploads a texture, so a
    /// per-frame call must answer from here rather than re-upload.
    static RESOLVED: RefCell<HashMap<String, Resolved>> = RefCell::new(HashMap::new());
}

/// The pixel size the cache holds this image at.
/// `None` when it holds no image of this URL: nothing under the tag,
/// or an entry whose identity is a different URL —
/// as a hash collision or a reused tag produces.
#[must_use]
pub fn cached_size(kind: ImageKind, url: &ImageUrl) -> Option<(u32, u32)> {
    if !url.is_present() {
        return None;
    }
    let stat = cache::stat(&tag_for(kind, url))?;
    let (width, height, identity) = decode_image_meta(&stat.metadata)?;
    (identity == url.as_str().as_bytes()).then_some((width, height))
}

/// The bitmap for `url`, if its image is in the cache.
///
/// `None` while nothing has arrived — the screens hold a placeholder.
#[must_use]
pub fn resolve(kind: ImageKind, url: &ImageUrl) -> Option<Resolved> {
    let tag = tag_for(kind, url);
    if let Some(hit) = RESOLVED.with(|memo| memo.borrow().get(&tag).copied()) {
        return Some(hit);
    }
    let (width, height) = cached_size(kind, url)?;
    let bitmap = assets::register_image(cache::lazy_get(&tag))?;
    let resolved = Resolved {
        bitmap,
        width,
        height,
    };
    RESOLVED.with(|memo| memo.borrow_mut().insert(tag, resolved));
    Some(resolved)
}

/// Forget every resolved bitmap, so one test's memo
/// cannot answer the next one's lookup.
#[cfg(test)]
pub(crate) fn invalidate_all() {
    RESOLVED.with(|memo| memo.borrow_mut().clear());
}

#[derive(Debug)]
pub struct Wanted<'a> {
    pub kind: ImageKind,
    pub url: &'a ImageUrl,
}

/// Every image the held payloads point at, repeats and absent URLs left
/// in for the caller to collapse.
#[must_use]
pub fn wanted(data: &crate::model::Data) -> Vec<Wanted<'_>> {
    let mut images = Vec::new();
    let mut want = |kind, url| images.push(Wanted { kind, url });
    for board in [&data.live_race, &data.live_quali, &data.live_practice] {
        let Some(board) = board.board() else { continue };
        want(ImageKind::Flag, &board.country_flag_url);
        for row in &board.rows {
            want(ImageKind::TeamLogo, &row.team_logo_url);
        }
    }
    if let Some(race) = data.next_race.as_ref() {
        want(ImageKind::Flag, &race.country_flag_url);
        want(ImageKind::Circuit, &race.circuit_image_url);
    }
    // Only the selected row: the statistics screen is the one view drawing
    // a headshot or a nationality flag, and it draws a single driver.
    // Sweeping the table would decode ~22 unseen pairs, evicting what is drawn.
    for driver in data.driver.iter().chain(data.selected_driver_stats()) {
        want(ImageKind::Headshot, &driver.headshot_url);
        want(ImageKind::Flag, &driver.nationality_flag_url);
    }
    // The statistics screen's constructor mark, which nothing else on that
    // screen reaches: a driver payload names a team without identifying
    // one, so the mark lives behind the index and the teams snapshot.
    // Absent, it draws only where a board or the standings has already
    // cached that logo — and out of season neither of them holds a row.
    if let Some(team) = data
        .selected_driver_stats()
        .and_then(|driver| data.team_of(&driver.jolpica_id))
    {
        want(ImageKind::TeamLogo, &team.logo_url);
    }
    for row in &data.standings {
        want(ImageKind::TeamLogo, &row.team_logo_url);
        want(ImageKind::Flag, &row.country_flag_url);
        // No headshot in this view
    }
    images
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::{assets, cache, encode_image_meta};

    use super::{ImageKind, invalidate_all, resolve, tag_for, wanted};
    use crate::model::ImageUrl;

    fn url(s: &str) -> ImageUrl {
        ImageUrl::from(s.to_owned())
    }

    fn seed(kind: ImageKind, url: &ImageUrl) {
        let meta = encode_image_meta(2, 2, url.as_str().as_bytes());
        cache::put(&tag_for(kind, url), &meta, &[0_u8; 16]);
    }

    #[test]
    fn a_seeded_url_resolves() {
        assets::init_test_registrars();
        invalidate_all();
        let flag = url("https://cdn.example.test/flag/nl.png");
        seed(ImageKind::Flag, &flag);
        assert!(resolve(ImageKind::Flag, &flag).is_some());
    }

    #[test]
    fn an_absent_url_resolves_to_nothing() {
        assets::init_test_registrars();
        invalidate_all();
        assert!(resolve(ImageKind::Flag, &ImageUrl::default()).is_none());
        assert!(resolve(ImageKind::Flag, &url("https://x.test/missing.png")).is_none());
    }

    #[test]
    fn anothers_entry_under_the_tag_is_not_this_image() {
        assets::init_test_registrars();
        invalidate_all();
        let wanted = url("https://x.test/new.png");
        let meta = encode_image_meta(2, 2, b"https://x.test/old.png");
        cache::put(&tag_for(ImageKind::Headshot, &wanted), &meta, &[0_u8; 16]);
        assert!(resolve(ImageKind::Headshot, &wanted).is_none());
    }

    /// Both directions: a field left out never arrives on the screen showing it,
    /// and one swept needlessly evicts artwork that is drawn.
    #[test]
    fn the_sweep_covers_every_drawn_image_and_nothing_else() {
        use crate::model::{
            Data, DriverStats, LiveBoard, NextRace, StandingsRow, TimingBoard, TimingRow,
        };

        let data = Data {
            standings: vec![StandingsRow {
                team_logo_url: url("https://x.test/standings-logo.png"),
                country_flag_url: url("https://x.test/standings-flag.png"),
                headshot_url: url("https://x.test/standings-face.png"),
                ..StandingsRow::default()
            }],
            driver_stats: vec![
                DriverStats {
                    jolpica_id: "selected".to_owned(),
                    headshot_url: url("https://x.test/stats-face.png"),
                    nationality_flag_url: url("https://x.test/stats-flag.png"),
                    ..DriverStats::default()
                },
                DriverStats {
                    jolpica_id: "bystander".to_owned(),
                    headshot_url: url("https://x.test/bystander-face.png"),
                    nationality_flag_url: url("https://x.test/bystander-flag.png"),
                    ..DriverStats::default()
                },
            ],
            driver: Some(DriverStats {
                jolpica_id: "selected".to_owned(),
                headshot_url: url("https://x.test/driver-face.png"),
                nationality_flag_url: url("https://x.test/driver-flag.png"),
                ..DriverStats::default()
            }),
            next_race: Some(NextRace {
                country_flag_url: url("https://x.test/race-flag.png"),
                circuit_image_url: url("https://x.test/circuit.png"),
                ..NextRace::default()
            }),
            live_race: LiveBoard::from_board(TimingBoard {
                country_flag_url: url("https://x.test/board-flag.png"),
                rows: vec![TimingRow {
                    team_logo_url: url("https://x.test/board-logo.png"),
                    ..TimingRow::default()
                }],
                ..TimingBoard::default()
            }),
            ..Data::default()
        };

        let swept: Vec<&str> = wanted(&data).iter().map(|want| want.url.as_str()).collect();
        for unwanted in [
            // No standings screen draws a headshot.
            "https://x.test/standings-face.png",
            // Only the selected driver's row reaches a screen.
            "https://x.test/bystander-face.png",
            "https://x.test/bystander-flag.png",
        ] {
            assert!(
                !swept.contains(&unwanted),
                "{unwanted} is fetched but never drawn"
            );
        }
        for expected in [
            "https://x.test/standings-logo.png",
            "https://x.test/standings-flag.png",
            "https://x.test/stats-face.png",
            "https://x.test/stats-flag.png",
            "https://x.test/driver-face.png",
            "https://x.test/driver-flag.png",
            "https://x.test/race-flag.png",
            "https://x.test/circuit.png",
            "https://x.test/board-flag.png",
            "https://x.test/board-logo.png",
        ] {
            assert!(swept.contains(&expected), "{expected} is never fetched");
        }
    }

    /// Out of season the standings are empty and no board is live, so
    /// nothing else asks for a constructor's logo. The statistics screen
    /// has to fetch its own mark, or it draws the livery square in the one
    /// state where no other view can have cached it first.
    #[test]
    fn the_statistics_screen_fetches_its_own_constructors_mark() {
        use crate::model::{Data, DriverStats, DriverTeam, Team};

        let data = Data {
            driver: Some(DriverStats {
                jolpica_id: "max_verstappen".to_owned(),
                ..DriverStats::default()
            }),
            teams: vec![Team {
                id: 276_183,
                name: "Red Bull Racing".to_owned(),
                logo_url: url("https://cdn.test/red-bull.png"),
                ..Team::default()
            }],
            driver_teams: vec![DriverTeam {
                jolpica_id: "max_verstappen".to_owned(),
                team_id: 276_183,
            }],
            ..Data::default()
        };
        let marks: Vec<&str> = wanted(&data)
            .iter()
            .filter(|want| want.kind == ImageKind::TeamLogo)
            .map(|want| want.url.as_str())
            .collect();
        assert_eq!(marks, ["https://cdn.test/red-bull.png"]);
    }

    #[test]
    fn a_headshot_and_a_flag_of_one_driver_are_told_apart() {
        use crate::model::{Data, DriverStats};

        let data = Data {
            driver: Some(DriverStats {
                headshot_url: url("https://x.test/shared.png"),
                nationality_flag_url: url("https://x.test/shared.png"),
                ..DriverStats::default()
            }),
            ..Data::default()
        };
        let kinds: Vec<ImageKind> = wanted(&data).iter().map(|want| want.kind).collect();
        assert!(kinds.contains(&ImageKind::Headshot) && kinds.contains(&ImageKind::Flag));
    }

    #[test]
    fn kinds_of_one_url_cache_apart() {
        let shared = url("https://x.test/img.png");
        assert_ne!(
            tag_for(ImageKind::Flag, &shared),
            tag_for(ImageKind::TeamLogo, &shared)
        );
    }
}
