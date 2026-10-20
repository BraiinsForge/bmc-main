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

// `include!`d by `build.rs` and by the crate's tests, so no inner doc comment.

use std::collections::BTreeMap;

use formatjs_icu_messageformat::Options;
use formatjs_icu_messageformat_parser::parser::Parser;
use serde_json::{Map, Value};

/// The `id → translation` catalog of a translator file, `text` being its content.
///
/// An entry is `{ "source", "translation", "description"? }`.
/// An untranslated entry (`null`) is left out and falls back to the English in the code.
/// Every translation is parsed the way the runtime parses it,
/// so a broken pattern fails the build instead of silently falling back on the device.
///
/// The error names `file`, the entry and the offending pattern.
fn compile(file: &str, text: &str) -> Result<BTreeMap<String, String>, String> {
    let entries: Map<String, Value> = serde_json::from_str(text)
        .map_err(|error| format!("`{file}` is not a JSON object: {error}"))?;
    let mut catalog = BTreeMap::new();
    for (id, entry) in entries {
        match entry.get("translation") {
            Some(Value::String(translation)) => {
                Parser::new(translation, Options::default_parser_options())
                    .parse()
                    .map_err(|error| {
                        format!(
                            "`{file}`: entry `{id}` is not valid ICU MessageFormat: {error}\n  {translation}"
                        )
                    })?;
                catalog.insert(id, translation.clone());
            }
            Some(Value::Null) | None => {}
            Some(other) => {
                return Err(format!(
                    "`{file}`: entry `{id}` has a non-string translation: {other}"
                ));
            }
        }
    }
    Ok(catalog)
}
