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

use bmc_shared_utils::typography::NBSP;

use super::*;

fn map<'a>(entries: &[(&str, &str)]) -> Map<'a> {
    Map::new(entries.iter().copied()).expect("BUG: a fixture is far under a map's size")
}

fn map2d<'a>(entries: &[(&str, Plural, &str)]) -> Map2d<'a> {
    Map2d::new(entries.iter().copied()).expect("BUG: a fixture is far under a map's size")
}

fn width<'a>(celsius: &str) -> Width<'a> {
    let celsius = [Plural::One, Plural::Few, Plural::Many, Plural::Other]
        .map(|plural| ("temperature-celsius", plural, celsius));
    Width {
        units: map2d(&celsius),
        powers: map2d(&[("power2", Plural::Other, "{0}²")]),
        per_unit: map(&[("duration-hour", "{0}/h")]),
        prefixes: map(&[("10p9", "G{0}"), ("1024p3", "Gi{0}")]),
        compounds: map(&[("per", "{0}/{1}"), ("times", "{0}⋅{1}")]),
    }
}

fn spacing<'a>() -> Spacing<'a> {
    Spacing {
        currency_match: "[[:^S:]&[:^Z:]]".into(),
        surrounding_match: "[:digit:]".into(),
        insert_between: NBSP.into(),
    }
}

/// Czech, cut short.
fn czech<'a>() -> Units<'a> {
    let plural_ranges = [
        (Plural::One, Plural::Few, Plural::Few),
        (Plural::Few, Plural::Other, Plural::Other),
    ]
    .into_iter()
    .collect();
    let one_then_compound = [Derive::Fixed(Plural::One), Derive::Compound];
    Units {
        format: FORMAT,
        short: width("{0} °C"),
        narrow: width("{0}°C"),
        range: "{0}–{1}".into(),
        plural_ranges,
        derivations: Derivations {
            per: [Derive::Compound, Derive::Fixed(Plural::One)],
            times: one_then_compound,
            power: one_then_compound,
            prefix: one_then_compound,
        },
        currency: Currency {
            standard: format!("#,##0.00{NBSP}¤").into(),
            alpha_next_to_number: format!("#,##0.00{NBSP}¤").into(),
            before: spacing(),
            after: spacing(),
        },
    }
}

#[test]
fn units_read_back_as_they_were_written() {
    let bytes = czech().to_bytes();
    assert_eq!(Units::from_bytes(&bytes), Ok(czech()));
}

#[test]
fn units_are_read_in_place() {
    let bytes = czech().to_bytes();
    let units = Units::from_bytes(&bytes).expect("BUG: just written");
    assert!(
        matches!(units.range, Cow::Borrowed(_)),
        "a string borrows from the bytes"
    );
    assert_eq!(
        units.short.units.get("temperature-celsius", Plural::Few),
        Some("{0} °C"),
        "a pattern is found where it lies"
    );
    assert_eq!(
        units.plural_range(Plural::One, Plural::Few),
        Some(Plural::Few)
    );
}

/// `bytes` with the two same-length keys swapped where they are written, so they no longer ascend.
fn swapped(mut bytes: Vec<u8>, first: &[u8], second: &[u8]) -> Vec<u8> {
    assert_eq!(first.len(), second.len(), "swapped in place");
    let at = |key: &[u8], bytes: &[u8]| {
        bytes
            .windows(key.len())
            .position(|window| window == key)
            .expect("BUG: the key is written")
    };
    let (at_first, at_second) = (at(first, &bytes), at(second, &bytes));
    bytes[at_first..at_first + first.len()].copy_from_slice(second);
    bytes[at_second..at_second + second.len()].copy_from_slice(first);
    bytes
}

#[test]
fn units_of_another_format_are_refused_by_it() {
    let units = Units {
        format: FORMAT + 1,
        ..czech()
    };
    assert_eq!(
        Units::from_bytes(&units.to_bytes()),
        Err(Refused::Format(FORMAT + 1))
    );
}

#[test]
fn bytes_that_are_no_units_are_refused() {
    let bytes = [FORMAT.to_le_bytes()[0], 0xff];
    assert!(
        matches!(Units::from_bytes(&bytes), Err(Refused::Malformed(_))),
        "the format leads, then nothing units are made of"
    );
}

#[test]
fn a_map_whose_keys_are_out_of_order_is_refused() {
    let mut units = czech();
    units.short.per_unit = map(&[("pu-aaa", "{0}/a"), ("pu-bbb", "{0}/b")]);
    let bytes = swapped(units.to_bytes(), b"pu-aaa", b"pu-bbb");
    let refused = Units::from_bytes(&bytes).expect_err("BUG: the keys no longer ascend");
    assert_eq!(refused.to_string(), "`short.per_unit`: keys out of order");
}

#[test]
fn a_two_level_map_whose_keys_are_out_of_order_is_refused() {
    let mut units = czech();
    units.short.units = map2d(&[
        ("un-aaa", Plural::Other, "{0} a"),
        ("un-bbb", Plural::Other, "{0} b"),
    ]);
    let bytes = swapped(units.to_bytes(), b"un-aaa", b"un-bbb");
    let refused = Units::from_bytes(&bytes).expect_err("BUG: the keys no longer ascend");
    assert_eq!(refused.to_string(), "`short.units`: keys out of order");
}

#[test]
fn every_row_of_a_read_two_level_map_answers_its_own_lookups() {
    let mut units = czech();
    units.short.units = map2d(&[
        ("length-meter", Plural::One, "{0} m one"),
        ("length-meter", Plural::Other, "{0} m"),
        ("area-acre", Plural::Few, "{0} ac few"),
        ("area-acre", Plural::Other, "{0} ac"),
        ("volume-liter", Plural::Other, "{0} l"),
    ]);
    let bytes = units.to_bytes();
    let read = Units::from_bytes(&bytes).expect("BUG: written units read back");
    let rows = &read.short.units;
    assert_eq!(rows.get("area-acre", Plural::Few), Some("{0} ac few"));
    assert_eq!(rows.get("length-meter", Plural::One), Some("{0} m one"));
    assert_eq!(rows.get("length-meter", Plural::Other), Some("{0} m"));
    assert_eq!(rows.get("volume-liter", Plural::Other), Some("{0} l"));
    assert_eq!(
        rows.get("length-meter", Plural::Few),
        None,
        "a category the row lacks is not borrowed from the next row"
    );
    assert_eq!(rows.get("volume-liter", Plural::One), None);
}

#[test]
fn a_map_of_more_text_than_its_offsets_reach_is_refused_not_built() {
    let long = "x".repeat(usize::from(u16::MAX));
    assert!(Map::new([("a", long.as_str()), ("b", long.as_str())]).is_err());
}

#[test]
fn bytes_after_the_units_are_refused() {
    let mut bytes = czech().to_bytes();
    bytes.extend([0, 0]);
    assert_eq!(Units::from_bytes(&bytes), Err(Refused::Trailing(2)));
}
