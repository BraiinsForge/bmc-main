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

//! Fixture suite that locks the validation split between the JSON Schema and the Rust validator.
//!
//! The table covers every `ParamKind` variant with one structural error rejected by both
//! validators and one semantic error rejected only by Rust.
//! Structural constraints live in the schema; cross-field invariants live in
//! `ParamDefinition::validate`.

use bmc_widget_manifest::{MAX_PARAM_KEY_LENGTH, MAX_PARAM_STRING_LENGTH, Manifest};
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
    json: &'static str,
    /// Expected verdict from the JSON Schema validator.
    schema_accepts: bool,
    /// Expected verdict from `Manifest::from_str`.
    manifest_accepts: bool,
}

// Every fixture is a full manifest with a unique uid so the schema's UUID v4 check passes.
// The only variability is the params block; everything else is boilerplate.

const FIXTURES: &[Negative] = &[
    // ── String variant ────────────────────────────────────────────────
    Negative {
        label: "string: default_value is a number (structural)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440001",
            "version": "0.1.0",
            "name": "X",
            "description": "Bad default type",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "label": {"name": "L", "type": "string", "default_value": 42}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    Negative {
        label: "string: default_value not in enum_values (semantic)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440002",
            "version": "0.1.0",
            "name": "X",
            "description": "Default outside enum",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "color": {
                    "name": "C",
                    "type": "string",
                    "default_value": "blue",
                    "enum_values": [
                        {"value": "red", "label": "R"},
                        {"value": "green", "label": "G"}
                    ]
                }
            }
        }"#,
        schema_accepts: true,
        manifest_accepts: false,
    },
    // ── Double variant ────────────────────────────────────────────────
    Negative {
        label: "double: default_value is a string (structural)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440003",
            "version": "0.1.0",
            "name": "X",
            "description": "Bad default type",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "ratio": {"name": "R", "type": "double", "default_value": "huge"}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    Negative {
        label: "double: default_value below min (semantic)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440004",
            "version": "0.1.0",
            "name": "X",
            "description": "Default below min",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "ratio": {
                    "name": "R",
                    "type": "double",
                    "default_value": 0.0,
                    "min": 10.0,
                    "max": 20.0
                }
            }
        }"#,
        schema_accepts: true,
        manifest_accepts: false,
    },
    // ── Integer variant ───────────────────────────────────────────────
    Negative {
        label: "integer: default_value is fractional (structural)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440005",
            "version": "0.1.0",
            "name": "X",
            "description": "Fractional default",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "count": {"name": "N", "type": "integer", "default_value": 3.14}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    Negative {
        label: "integer: step zero (structural via exclusiveMinimum)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440006",
            "version": "0.1.0",
            "name": "X",
            "description": "Zero step",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "count": {"name": "N", "type": "integer", "default_value": 1, "step": 0}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    Negative {
        label: "integer: default_value above max (semantic)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440007",
            "version": "0.1.0",
            "name": "X",
            "description": "Default above max",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "count": {
                    "name": "N",
                    "type": "integer",
                    "default_value": 15,
                    "min": 0,
                    "max": 10
                }
            }
        }"#,
        schema_accepts: true,
        manifest_accepts: false,
    },
    // ── Boolean variant ───────────────────────────────────────────────
    Negative {
        label: "boolean: default_value is a string (structural)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440008",
            "version": "0.1.0",
            "name": "X",
            "description": "Bad default type",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "flag": {"name": "F", "type": "boolean", "default_value": "yes"}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    // ── Timezone variant ──────────────────────────────────────────────
    Negative {
        label: "timezone: default_value is a number (structural)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440009",
            "version": "0.1.0",
            "name": "X",
            "description": "Bad default type",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "tz": {"name": "T", "type": "timezone", "default_value": 42}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    // ── Envelope-level structural rejects ─────────────────────────────
    Negative {
        label: "envelope: empty supported_viewports (structural via minItems)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440010",
            "version": "0.1.0",
            "name": "X",
            "description": "No viewports",
            "binary": "bin/x",
            "supported_viewports": []
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    Negative {
        label: "envelope: param key starts with a digit (structural via regex)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440011",
            "version": "0.1.0",
            "name": "X",
            "description": "Bad param key",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "params": {
                "1bad": {"name": "B", "type": "string", "default_value": "x"}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    // ── Credential slots ──────────────────────────────────────────────
    Negative {
        label: "credentials: slot type is a number (structural)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440012",
            "version": "0.1.0",
            "name": "X",
            "description": "Bad slot type literal",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "credentials": {
                "pool": {"type": 42, "label": "Pool"}
            }
        }"#,
        schema_accepts: false,
        manifest_accepts: false,
    },
    Negative {
        // A firmware constant, so JSON Schema cannot know the ids — only the Rust validator can.
        label: "credentials: unknown credential type id (semantic)",
        json: r#"{
            "uid": "550e8400-e29b-41d4-a716-446655440013",
            "version": "0.1.0",
            "name": "X",
            "description": "Unknown slot type",
            "binary": "bin/x",
            "supported_viewports": [{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}],
            "credentials": {
                "pool": {"type": "braiins_pool", "label": "Pool"}
            }
        }"#,
        schema_accepts: true,
        manifest_accepts: false,
    },
];

#[test]
fn negative_fixtures_lock_schema_vs_validator_split() {
    let validator = schema_validator();

    let mut failures: Vec<String> = Vec::new();

    for fixture in FIXTURES {
        let instance: serde_json::Value = serde_json::from_str(fixture.json)
            .unwrap_or_else(|e| panic!("BUG: fixture {:?} is not valid JSON: {e}", fixture.label));
        let schema_actual = validator.is_valid(&instance);
        let manifest_actual = Manifest::from_str(fixture.json).is_ok();

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

// ── Length caps (split fixtures with long strings — kept separate from FIXTURES
//    because the const-table shape can't carry runtime-built JSON bodies) ───────

#[test]
fn over_cap_param_key_rejected_by_manifest_but_accepted_by_schema() {
    // `patternProperties` only constrains key shape via regex, not length — so the schema
    // accepts even an over-cap key. The Rust validator is the authoritative gate on length.
    let key = "a".repeat(MAX_PARAM_KEY_LENGTH + 1);
    let json = format!(
        r#"{{
            "uid": "550e8400-e29b-41d4-a716-446655440101",
            "version": "0.1.0",
            "name": "X",
            "description": "Over-cap key",
            "binary": "bin/x",
            "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}}],
            "params": {{
                "{key}": {{"name": "K", "type": "string", "default_value": "x"}}
            }}
        }}"#
    );
    let validator = schema_validator();
    let instance: serde_json::Value =
        serde_json::from_str(&json).expect("BUG: fixture must be valid JSON");
    assert!(
        validator.is_valid(&instance),
        "JSON Schema unexpectedly rejected over-cap param key; patternProperties is regex-only"
    );
    assert!(
        Manifest::from_str(&json).is_err(),
        "Manifest::from_str accepted over-cap param key"
    );
}

#[test]
fn over_cap_string_default_value_rejected_by_both() {
    let big = "x".repeat(MAX_PARAM_STRING_LENGTH + 1);
    let json = format!(
        r#"{{
            "uid": "550e8400-e29b-41d4-a716-446655440102",
            "version": "0.1.0",
            "name": "X",
            "description": "Over-cap default_value",
            "binary": "bin/x",
            "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}}],
            "params": {{
                "label": {{"name": "L", "type": "string", "default_value": "{big}"}}
            }}
        }}"#
    );
    let validator = schema_validator();
    let instance: serde_json::Value =
        serde_json::from_str(&json).expect("BUG: fixture must be valid JSON");
    assert!(
        !validator.is_valid(&instance),
        "JSON Schema accepted over-cap String default_value"
    );
    assert!(
        Manifest::from_str(&json).is_err(),
        "Manifest::from_str accepted over-cap String default_value"
    );
}

#[test]
fn over_cap_enum_values_entry_rejected_by_both() {
    let big = "x".repeat(MAX_PARAM_STRING_LENGTH + 1);
    let json = format!(
        r#"{{
            "uid": "550e8400-e29b-41d4-a716-446655440103",
            "version": "0.1.0",
            "name": "X",
            "description": "Over-cap enum_values value",
            "binary": "bin/x",
            "supported_viewports": [{{"type":"rectangular","min_width":317,"max_width":317,"min_height":238,"max_height":238,"min_dpi":1,"max_dpi":1}}],
            "params": {{
                "color": {{
                    "name": "C",
                    "type": "string",
                    "default_value": "ok",
                    "enum_values": [
                        {{"value": "ok", "label": "OK"}},
                        {{"value": "{big}", "label": "Big"}}
                    ]
                }}
            }}
        }}"#
    );
    let validator = schema_validator();
    let instance: serde_json::Value =
        serde_json::from_str(&json).expect("BUG: fixture must be valid JSON");
    assert!(
        !validator.is_valid(&instance),
        "JSON Schema accepted over-cap enum_values entry"
    );
    assert!(
        Manifest::from_str(&json).is_err(),
        "Manifest::from_str accepted over-cap enum_values entry"
    );
}
