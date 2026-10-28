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

//! Params migrator for ticker-list, protocol 0:
//! turns the 0.1.x `symbol_1..8` slots into the 0.2.0 `symbols` rows.
// TODO(BDK-723): replace this program with the migration contract for wasm widgets.

use std::io::{Read, Write};
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const LEGACY_SLOTS: RangeInclusive<usize> = 1..=8;

/// The part of the migrator input this program reads.
#[derive(Debug, Deserialize)]
struct Input {
    params: Map<String, Value>,
}

#[derive(Debug, Serialize)]
struct Overlay<'a> {
    /// `None` leaves `symbols` out, so the manifest default fills it.
    #[serde(skip_serializing_if = "Option::is_none")]
    symbols: Option<Vec<Row<'a>>>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Row<'a> {
    base: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    quote: Option<&'a str>,
}

pub fn run(input: impl Read, mut output: impl Write) -> std::io::Result<()> {
    let input: Input = serde_json::from_reader(input)?;
    serde_json::to_writer(&mut output, &migrate(&input.params))?;
    writeln!(output)
}

/// Turns the 0.1.x `symbol_N` slots into `symbols` rows in slot order,
/// skipping empty slots and repeats.
fn migrate(params: &Map<String, Value>) -> Overlay<'_> {
    let mut rows = Vec::new();
    for slot in LEGACY_SLOTS {
        let Some(symbol) = params
            .get(&format!("symbol_{slot}"))
            .and_then(Value::as_str)
            // Trims as the widget does.
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
        else {
            continue;
        };
        let row = row(symbol);
        if !rows.contains(&row) {
            rows.push(row);
        }
    }
    Overlay {
        symbols: (!rows.is_empty()).then_some(rows),
    }
}

/// `BASE-QUOTE` splits at the first dash; a dash with an empty side stays in the base.
fn row(symbol: &str) -> Row<'_> {
    let pair = symbol
        .split_once('-')
        .map(|(base, quote)| (base.trim(), quote.trim()))
        .filter(|(base, quote)| !base.is_empty() && !quote.is_empty());
    match pair {
        Some((base, quote)) => Row {
            base,
            quote: Some(quote),
        },
        None => Row {
            base: symbol,
            quote: None,
        },
    }
}

#[cfg(test)]
mod tests;
