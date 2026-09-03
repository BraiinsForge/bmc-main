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

//! Shared hashboard parsing: a JSON-pointer lookup abstraction
//! plus the chip model and count folded across a miner's hashboards.

use bmc_wasm_sdk::ufmt;

/// JSON-pointer lookup over a parsed miner/device document. The wasm host backs
/// it with `JsonDoc`; host tests back it with a map-backed double.
pub trait JsonLookup {
    fn str(&self, path: &str) -> Option<String>;
    fn i64(&self, path: &str) -> Option<i64>;
    fn f64(&self, path: &str) -> Option<f64>;
}

#[cfg(target_arch = "wasm32")]
impl JsonLookup for bmc_wasm_sdk::json::JsonDoc {
    fn str(&self, path: &str) -> Option<String> {
        self.str(path)
    }

    fn i64(&self, path: &str) -> Option<i64> {
        self.i64(path)
    }

    fn f64(&self, path: &str) -> Option<f64> {
        self.f64(path)
    }
}

// Upper bound on hashboard slots to probe.
const MAX_HASHBOARDS: usize = 16;

/// Chip model and total chip count folded across a miner's hashboards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChipSummary {
    /// The first non-empty board's chip type.
    pub model: Option<String>,
    /// Chips summed across every present board.
    pub count: Option<usize>,
}

/// Fold the chip model and count across all hashboard slots. Scans every slot
/// rather than stopping at the first gap, so a failed or disabled board between
/// two live ones doesn't hide the boards past it. The model is the first
/// non-empty board's chip type; the count sums chips across all boards.
#[must_use]
pub fn sum_chips<J: JsonLookup + ?Sized>(json: &J) -> ChipSummary {
    let mut model = None;
    let mut count: Option<usize> = None;
    for i in 0..MAX_HASHBOARDS {
        if model.is_none() {
            model = json
                .str(&bmc_wasm_sdk::fmt!("/hashboards/{i}/chip_type"))
                .filter(|s| !s.is_empty());
        }
        if let Some(chips) = json
            .i64(&bmc_wasm_sdk::fmt!("/hashboards/{i}/chips_count"))
            .and_then(|v| usize::try_from(v).ok())
        {
            count = Some(count.unwrap_or(0).saturating_add(chips));
        }
    }
    ChipSummary { model, count }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MapJson {
        strings: BTreeMap<&'static str, &'static str>,
        ints: BTreeMap<&'static str, i64>,
    }

    impl JsonLookup for MapJson {
        fn str(&self, path: &str) -> Option<String> {
            self.strings.get(path).map(|s| (*s).to_owned())
        }

        fn i64(&self, path: &str) -> Option<i64> {
            self.ints.get(path).copied()
        }

        fn f64(&self, _path: &str) -> Option<f64> {
            None
        }
    }

    #[test]
    fn empty_document_yields_no_chips() {
        assert_eq!(sum_chips(&MapJson::default()), ChipSummary::default());
    }

    #[test]
    fn single_board_reports_model_and_count() {
        let mut j = MapJson::default();
        j.strings.insert("/hashboards/0/chip_type", "BM1370");
        j.ints.insert("/hashboards/0/chips_count", 108);
        let s = sum_chips(&j);
        assert_eq!(s.model.as_deref(), Some("BM1370"));
        assert_eq!(s.count, Some(108));
    }

    #[test]
    fn sums_count_and_takes_first_model_across_boards() {
        let mut j = MapJson::default();
        j.strings.insert("/hashboards/0/chip_type", "BM1370");
        j.ints.insert("/hashboards/0/chips_count", 108);
        j.ints.insert("/hashboards/1/chips_count", 108);
        j.strings.insert("/hashboards/2/chip_type", "BM1371");
        j.ints.insert("/hashboards/2/chips_count", 108);
        let s = sum_chips(&j);
        assert_eq!(s.model.as_deref(), Some("BM1370"));
        assert_eq!(s.count, Some(324));
    }

    #[test]
    fn skips_gaps_and_takes_first_present_board() {
        // Board 0 absent; boards past the gap still count, and the model comes
        // from the first present board rather than the literal first slot.
        let mut j = MapJson::default();
        j.strings.insert("/hashboards/1/chip_type", "BM1370");
        j.ints.insert("/hashboards/1/chips_count", 70);
        j.ints.insert("/hashboards/2/chips_count", 76);
        let s = sum_chips(&j);
        assert_eq!(s.model.as_deref(), Some("BM1370"));
        assert_eq!(s.count, Some(146));
    }
}
