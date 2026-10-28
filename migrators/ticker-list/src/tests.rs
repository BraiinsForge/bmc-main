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

use serde_json::{Value, json};

use super::run;

fn migrate(params: &Value) -> Value {
    let input = json!({
        "params_version": null,
        "widget_version": "0.2.0",
        "width": 317,
        "height": 238,
        "viewport_shape": "rectangular",
        "display": {"width": 1280, "height": 480, "shape": "rectangular", "dpi": 1},
        "params": params,
    });
    let mut output = Vec::new();
    run(input.to_string().as_bytes(), &mut output).expect("BUG: a protocol-0 input migrates");
    serde_json::from_slice(&output).expect("BUG: the overlay is JSON")
}

#[test]
fn current_shape_is_left_alone() {
    assert_eq!(
        migrate(&json!({"period": "7d", "symbols": [{"base": "NVDA"}]})),
        json!({})
    );
}

#[test]
fn all_empty_slots_defer_to_the_manifest_default() {
    assert_eq!(
        migrate(&json!({"period": "7d", "symbol_1": "", "symbol_2": "  ", "symbol_3": null})),
        json!({})
    );
}

#[test]
fn slots_become_rows_in_order_pairs_split_index_kept() {
    assert_eq!(
        migrate(&json!({
            "period": "7d",
            "symbol_1": "NVDA",
            "symbol_2": "EUR-USD",
            "symbol_3": "^GSPC",
            "symbol_4": "",
            "symbol_5": "BTC-USD",
            "symbol_6": null,
            "symbol_7": "",
            "symbol_8": "",
        })),
        json!({"symbols": [
            {"base": "NVDA"},
            {"base": "EUR", "quote": "USD"},
            {"base": "^GSPC"},
            {"base": "BTC", "quote": "USD"},
        ]})
    );
}

#[test]
fn whitespace_is_trimmed_and_repeats_dropped() {
    assert_eq!(
        migrate(&json!({
            "symbol_1": " AAPL ",
            "symbol_2": "AAPL",
            "symbol_3": "EUR - USD",
            "symbol_8": "aapl",
        })),
        json!({"symbols": [
            {"base": "AAPL"},
            {"base": "EUR", "quote": "USD"},
            {"base": "aapl"},
        ]})
    );
}

#[test]
fn pairs_that_split_to_the_same_row_are_one_row() {
    assert_eq!(
        migrate(&json!({"symbol_1": "EUR-USD", "symbol_2": "EUR - USD", "symbol_3": " EUR-USD "})),
        json!({"symbols": [{"base": "EUR", "quote": "USD"}]})
    );
}

#[test]
fn a_dash_with_an_empty_side_stays_one_base() {
    assert_eq!(
        migrate(&json!({"symbol_1": "-USD", "symbol_2": "EUR-"})),
        json!({"symbols": [{"base": "-USD"}, {"base": "EUR-"}]})
    );
}

#[test]
fn a_single_slot_key_is_enough_to_migrate() {
    assert_eq!(
        migrate(&json!({"period": "1d", "symbol_5": "TSLA"})),
        json!({"symbols": [{"base": "TSLA"}]})
    );
}
