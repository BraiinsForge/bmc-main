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

//! Fixture suite that locks the validation split between
//! the JSON Schema and the Rust validator.
//!
//! The table covers every `ParamKind` variant with one structural error
//! rejected by both validators and one semantic error rejected only by Rust.
//!
//! Structural constraints live in the schema; cross-field
//! invariants live in `ParamDefinition::validate`.

use bmc_widget_manifest::{
    MAX_ARRAY_ITEMS, MAX_PARAM_KEY_LENGTH, MAX_PARAM_STRING_LENGTH, Manifest,
};
use serde_json::{Value, json};
use std::str::FromStr;

const COMMITTED_SCHEMA: &str = include_str!("../manifest.schema.json");

fn schema_validator() -> jsonschema::Validator {
    let schema: serde_json::Value =
        serde_json::from_str(COMMITTED_SCHEMA).expect("BUG: committed schema must parse as JSON");
    jsonschema::validator_for(&schema).expect("BUG: committed schema must compile to a validator")
}

/// One row per negative fixture.
struct Negative {
    /// Short label used in failure messages.
    label: &'static str,
    /// Full manifest JSON.
    manifest: Value,
    /// Expected verdict from the JSON Schema validator.
    schema_accepts: bool,
    /// Expected verdict from `Manifest::from_str`.
    manifest_accepts: bool,
}

/// A manifest both validators accept;
/// each fixture breaks one field of it.
fn envelope() -> Value {
    json!({
        "uid": "550e8400-e29b-41d4-a716-446655440000",
        "version": "0.1.0",
        "name": "X",
        "description": "Fixture",
        "binary": "bin/x",
        "supported_viewports": [{
            "type": "rectangular",
            "min_width": 317,
            "max_width": 317,
            "min_height": 238,
            "max_height": 238,
            "min_dpi": 1,
            "max_dpi": 1,
        }],
    })
}

fn manifest_with(key: &str, value: Value) -> Value {
    let mut manifest = envelope();
    manifest[key] = value;
    manifest
}

#[expect(
    clippy::too_many_lines,
    reason = "one flat row per fixture; splitting the table hides what it covers"
)]
fn fixtures() -> Vec<Negative> {
    let over_cap_string = "x".repeat(MAX_PARAM_STRING_LENGTH + 1);
    let over_cap_key = "a".repeat(MAX_PARAM_KEY_LENGTH + 1);
    let bounded_string = |bounds: Value| {
        let mut param = json!({ "name": "S", "type": "string", "optional": true });
        param
            .as_object_mut()
            .expect("BUG: the param is an object")
            .extend(
                bounds
                    .as_object()
                    .expect("BUG: the bounds are an object")
                    .clone(),
            );
        manifest_with("params", json!({ "symbol": param }))
    };
    let unique_links = |unique_items: Value, default_value: Value| {
        manifest_with(
            "params",
            json!({
                "links": {
                    "name": "L",
                    "type": "array",
                    "items": {
                        "type": "object",
                        "fields": {
                            "label": { "name": "Label", "type": "string" },
                            "url": { "name": "URL", "type": "string", "optional": true },
                        },
                    },
                    "max_items": 3,
                    "unique_items": unique_items,
                    "default_value": default_value,
                },
            }),
        )
    };
    vec![
        // ── String variant ────────────────────────────────────────────────
        Negative {
            label: "string: default_value is a number (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "label": { "name": "L", "type": "string", "default_value": 42 },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "string: default_value not in enum_values (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "color": {
                        "name": "C",
                        "type": "string",
                        "default_value": "blue",
                        "enum_values": [
                            { "value": "red", "label": "R" },
                            { "value": "green", "label": "G" },
                        ],
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "string: radio enum_control without enum_values (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "color": { "name": "C", "type": "string", "enum_control": "radio" },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "string: unknown enum_control (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "color": {
                        "name": "C",
                        "type": "string",
                        "enum_control": "slider",
                        "enum_values": [{ "value": "red", "label": "R" }],
                    },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "string: over-cap default_value (structural via maxLength)",
            manifest: manifest_with(
                "params",
                json!({
                    "label": { "name": "L", "type": "string", "default_value": over_cap_string },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "string: over-cap enum_values entry (structural via maxLength)",
            manifest: manifest_with(
                "params",
                json!({
                    "color": {
                        "name": "C",
                        "type": "string",
                        "default_value": "ok",
                        "enum_values": [
                            { "value": "ok", "label": "OK" },
                            { "value": over_cap_string, "label": "Big" },
                        ],
                    },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "string: max_length zero (structural via minimum)",
            manifest: bounded_string(json!({ "max_length": 0 })),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "string: max_length above the cap (structural via maximum)",
            manifest: bounded_string(json!({ "max_length": MAX_PARAM_STRING_LENGTH + 1 })),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "string: min_length above max_length (semantic)",
            manifest: bounded_string(json!({ "min_length": 4, "max_length": 2 })),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "string: enum_values entry outside the length bounds (semantic)",
            manifest: bounded_string(json!({
                "max_length": 3,
                "default_value": "BTC",
                "enum_values": [
                    { "value": "BTC", "label": "Bitcoin" },
                    { "value": "BTCUSD", "label": "Bitcoin in dollars" },
                ],
            })),
            schema_accepts: true,
            manifest_accepts: false,
        },
        // ── Double variant ────────────────────────────────────────────────
        Negative {
            label: "double: default_value is a string (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "ratio": { "name": "R", "type": "double", "default_value": "huge" },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "double: default_value below min (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "ratio": {
                        "name": "R",
                        "type": "double",
                        "default_value": 0.0,
                        "min": 10.0,
                        "max": 20.0,
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        // ── Integer variant ───────────────────────────────────────────────
        Negative {
            label: "integer: default_value is fractional (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "count": { "name": "N", "type": "integer", "default_value": 1.5 },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "integer: step zero (structural via exclusiveMinimum)",
            manifest: manifest_with(
                "params",
                json!({
                    "count": { "name": "N", "type": "integer", "default_value": 1, "step": 0 },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "integer: default_value above max (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "count": {
                        "name": "N",
                        "type": "integer",
                        "default_value": 15,
                        "min": 0,
                        "max": 10,
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        // ── Boolean variant ───────────────────────────────────────────────
        Negative {
            label: "boolean: default_value is a string (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "flag": { "name": "F", "type": "boolean", "default_value": "yes" },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        // ── Timezone variant ──────────────────────────────────────────────
        Negative {
            label: "timezone: default_value is a number (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "tz": { "name": "T", "type": "timezone", "default_value": 42 },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        // ── Array variant ─────────────────────────────────────────────────
        Negative {
            label: "array: max_items missing (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": { "name": "S", "type": "array", "items": { "type": "string" } },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "array: max_items zero (structural via minimum)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "items": { "type": "string" },
                        "max_items": 0,
                    },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "array: max_items above the cap (structural via maximum)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "items": { "type": "string" },
                        "max_items": MAX_ARRAY_ITEMS + 1,
                    },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "array: default_value is not a list (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "items": { "type": "string" },
                        "max_items": 4,
                        "default_value": "NVDA",
                    },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "array: min_items above max_items (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "items": { "type": "string" },
                        "min_items": 5,
                        "max_items": 2,
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: default item outside the item bounds (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "counts": {
                        "name": "C",
                        "type": "array",
                        "items": { "type": "integer", "max": 5 },
                        "max_items": 3,
                        "default_value": [7],
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: over-cap default item (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "items": { "type": "string" },
                        "max_items": 1,
                        "default_value": [over_cap_string],
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "object item: fields missing (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "links": { "name": "L", "type": "array", "items": { "type": "object" }, "max_items": 2 },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "object item: a field that is itself a list (structural)",
            manifest: manifest_with(
                "params",
                json!({
                    "links": {
                        "name": "L",
                        "type": "array",
                        "items": {
                            "type": "object",
                            "fields": {
                                "tags": {
                                    "name": "T",
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "max_items": 2,
                                },
                            },
                        },
                        "max_items": 2,
                    },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "object item: default row missing a required field (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "links": {
                        "name": "L",
                        "type": "array",
                        "items": {
                            "type": "object",
                            "fields": { "label": { "name": "Label", "type": "string" } },
                        },
                        "max_items": 2,
                        "default_value": [{}],
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: optional (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "optional": true,
                        "items": { "type": "string" },
                        "max_items": 4,
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: unique_items a bare string (structural)",
            manifest: unique_links(json!("label"), json!([])),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "array: unique_items key not a param key (structural via regex)",
            manifest: unique_links(json!(["Bad Key"]), json!([])),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "array: unique_items keys on a scalar list (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    "symbols": {
                        "name": "S",
                        "type": "array",
                        "items": { "type": "string" },
                        "max_items": 4,
                        "unique_items": ["label"],
                        "default_value": ["BTC"],
                    },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: unique_items with no keys (semantic)",
            manifest: unique_links(json!([]), json!([])),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: unique_items key that names no field (semantic)",
            manifest: unique_links(json!(["icon"]), json!([])),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: unique_items key listed twice (semantic)",
            manifest: unique_links(json!(["label", "label"]), json!([])),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: default repeats a whole row under unique_items (semantic)",
            manifest: unique_links(
                json!(true),
                json!([{ "label": "Home" }, { "label": "Home" }]),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        Negative {
            label: "array: default repeats a key under unique_items (semantic)",
            manifest: unique_links(
                json!(["label"]),
                json!([
                    { "label": "Home", "url": "https://braiins.com" },
                    { "label": "Home", "url": "https://braiins.com/pool" },
                ]),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        // ── Envelope-level rejects ────────────────────────────────────────
        Negative {
            label: "envelope: empty supported_viewports (structural via minItems)",
            manifest: manifest_with("supported_viewports", json!([])),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            label: "envelope: param key starts with a digit (structural via regex)",
            manifest: manifest_with(
                "params",
                json!({
                    "1bad": { "name": "B", "type": "string", "default_value": "x" },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            // `patternProperties` constrains a key's shape by regex, never its length,
            // so the Rust validator is the only gate on it.
            label: "envelope: over-cap param key (semantic)",
            manifest: manifest_with(
                "params",
                json!({
                    over_cap_key: { "name": "K", "type": "string", "default_value": "x" },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
        // ── Credential slots ──────────────────────────────────────────────
        Negative {
            label: "credentials: slot type is a number (structural)",
            manifest: manifest_with(
                "credentials",
                json!({
                    "pool": { "type": 42, "label": "Pool" },
                }),
            ),
            schema_accepts: false,
            manifest_accepts: false,
        },
        Negative {
            // A firmware constant, so JSON Schema cannot know
            // the ids — only the Rust validator can.
            label: "credentials: unknown credential type id (semantic)",
            manifest: manifest_with(
                "credentials",
                json!({
                    "pool": { "type": "braiins_pool", "label": "Pool" },
                }),
            ),
            schema_accepts: true,
            manifest_accepts: false,
        },
    ]
}

#[test]
fn the_envelope_passes_both_validators() {
    let manifest = envelope();
    assert!(
        schema_validator().is_valid(&manifest),
        "the JSON Schema rejected the envelope every fixture builds on"
    );
    Manifest::from_str(&manifest.to_string())
        .expect("BUG: the envelope every fixture builds on must parse");
}

#[test]
fn negative_fixtures_lock_schema_vs_validator_split() {
    let validator = schema_validator();

    let mut failures: Vec<String> = Vec::new();

    for fixture in fixtures() {
        let schema_actual = validator.is_valid(&fixture.manifest);
        let manifest_actual = Manifest::from_str(&fixture.manifest.to_string()).is_ok();

        if schema_actual != fixture.schema_accepts {
            failures.push(format!(
                "{}: schema verdict mismatch — expected accepts={}, got accepts={}",
                fixture.label, fixture.schema_accepts, schema_actual,
            ));
        }
        if manifest_actual != fixture.manifest_accepts {
            failures.push(format!(
                "{}: Manifest::from_str verdict mismatch — expected accepts={}, got accepts={}",
                fixture.label, fixture.manifest_accepts, manifest_actual,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "fixture-split mismatches:\n  - {}",
        failures.join("\n  - "),
    );
}
