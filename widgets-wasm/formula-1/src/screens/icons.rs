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

//! Artwork compiled in: the widget's chrome and the constructor marks.
//! Headshots and flags are fetched instead — far too many to carry.

use bmc_wasm_sdk::{Bitmap, Svg, include_bitmap, include_svg};

/// The two swept bars trailing the header, from the design's brand mark.
pub const BRAND_STRIPE: Svg = include_svg!("assets/brand-stripe.svg");

// Constructor marks ride in the binary: a row draws its team before any
// fetch could answer. Bitmaps at exactly their 32 px draw size — the
// renderer flattens SVG to filled paths, and samples bitmaps 2×2 bilinear,
// so either an SVG or a rescale here loses the detail.
const ALPINE: Bitmap = include_bitmap!("assets/logos/alpine.png");
const ASTON_MARTIN: Bitmap = include_bitmap!("assets/logos/aston-martin.png");
const AUDI: Bitmap = include_bitmap!("assets/logos/audi.png");
const CADILLAC: Bitmap = include_bitmap!("assets/logos/cadillac.png");
const FERRARI: Bitmap = include_bitmap!("assets/logos/ferrari.png");
const HAAS: Bitmap = include_bitmap!("assets/logos/haas.png");
const MCLAREN: Bitmap = include_bitmap!("assets/logos/mclaren.png");
const MERCEDES: Bitmap = include_bitmap!("assets/logos/mercedes.png");
const RACING_BULLS: Bitmap = include_bitmap!("assets/logos/racing-bulls.png");
const RED_BULL: Bitmap = include_bitmap!("assets/logos/red-bull.png");
const WILLIAMS: Bitmap = include_bitmap!("assets/logos/williams.png");

/// Constructors keyed by a word inside whatever the payload calls a team.
/// Order breaks ties: a Haas is often named for its Ferrari power unit,
/// and Racing Bulls would otherwise answer to Red Bull.
const MARKS: &[(&str, &Bitmap)] = &[
    ("haas", &HAAS),
    ("racing bulls", &RACING_BULLS),
    ("red bull", &RED_BULL),
    ("ferrari", &FERRARI),
    ("mclaren", &MCLAREN),
    ("mercedes", &MERCEDES),
    ("williams", &WILLIAMS),
    ("aston martin", &ASTON_MARTIN),
    ("alpine", &ALPINE),
    ("audi", &AUDI),
    ("cadillac", &CADILLAC),
];

/// The mark for a team, or `None` where this build carries no artwork;
/// screens then fall back to the livery colour.
#[must_use]
pub fn team_mark(team_name: &str) -> Option<&'static Bitmap> {
    let name = team_name.to_lowercase();
    MARKS
        .iter()
        .find_map(|(key, mark)| name.contains(key).then_some(*mark))
}

#[cfg(test)]
mod tests {
    use super::team_mark;

    #[test]
    fn a_team_is_found_however_its_sponsors_dress_the_name() {
        for name in [
            "Ferrari",
            "Scuderia Ferrari",
            "Oracle Red Bull Racing",
            "Visa Cash App Racing Bulls",
            "Mercedes-AMG Petronas",
            "Atlassian Williams",
        ] {
            assert!(team_mark(name).is_some(), "no mark for `{name}`");
        }
    }

    #[test]
    fn an_engine_supplier_in_the_name_does_not_win_over_the_team() {
        let haas = team_mark("Haas Ferrari").expect("BUG: Haas must resolve");
        assert!(std::ptr::eq(haas, team_mark("Haas").expect("BUG: Haas")));

        let bulls = team_mark("Visa Cash App Racing Bulls").expect("BUG: Racing Bulls");
        assert!(std::ptr::eq(
            bulls,
            team_mark("Racing Bulls").expect("BUG: Racing Bulls"),
        ));
        assert!(!std::ptr::eq(
            bulls,
            team_mark("Red Bull Racing").expect("BUG: Red Bull"),
        ));
    }

    #[test]
    fn a_team_this_build_has_no_artwork_for_falls_back() {
        assert!(team_mark("Stake F1 Kick Sauber").is_none());
    }
}
