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

//! Schema artifact tests.
//!
//! `schema_drift_guard` enforces that the committed `manifest.schema.json` matches what `schemars`
//! emits for the current Rust types. If a developer edits the manifest types without regenerating
//! the schema, this test fires with a regenerate-command hint.
//!
//! `schema_descriptions_cover_every_definition` enforces that every type/variant/field that lands in
//! the schema carries a non-empty `description`. The intent is that the schema is the contract
//! editor tooling reads; missing descriptions silently degrade hover help.

use bmc_widget_manifest::Manifest;

const COMMITTED_SCHEMA: &str = include_str!("../manifest.schema.json");

fn render_schema() -> String {
    let schema = schemars::schema_for!(Manifest);
    serde_json::to_string_pretty(&schema).expect("BUG: schema must serialise")
}

#[test]
fn schema_drift_guard() {
    let rendered = render_schema();
    let committed = COMMITTED_SCHEMA.trim_end_matches('\n');
    let rendered_trimmed = rendered.trim_end_matches('\n');

    assert!(
        committed == rendered_trimmed,
        "manifest.schema.json is stale relative to the Rust types.\n\
         Regenerate with: just manifest::gen-schema\n\
         Then re-run this test.",
    );
}

#[test]
fn schema_descriptions_cover_every_definition() {
    let rendered = render_schema();
    let value: serde_json::Value =
        serde_json::from_str(&rendered).expect("BUG: rendered schema must parse as JSON");

    let mut missing: Vec<String> = Vec::new();
    check_descriptions(&value, "$", &mut missing);

    assert!(
        missing.is_empty(),
        "every public type, variant, and field that appears in the schema must carry a `///` doc \
         comment that schemars surfaces as `description`. Missing at:\n  - {}",
        missing.join("\n  - "),
    );
}

/// Walks the rendered schema and records every type-level / field-level entry that is missing
/// a `description`. The traversal targets the two places schemars writes user docs:
///
///  - `$defs.<TypeName>` — one entry per public type with `JsonSchema`
///  - `properties.<field>` — one entry per named field on a struct or named-variant carrier
fn check_descriptions(value: &serde_json::Value, path: &str, missing: &mut Vec<String>) {
    let Some(obj) = value.as_object() else {
        return;
    };

    if let Some(defs) = obj.get("$defs").and_then(|d| d.as_object()) {
        for (name, def) in defs {
            let def_path = format!("{path}.$defs.{name}");
            require_description(def, &def_path, missing);
            check_descriptions(def, &def_path, missing);
        }
    }

    if let Some(props) = obj.get("properties").and_then(|p| p.as_object()) {
        for (field, schema) in props {
            let field_path = format!("{path}.properties.{field}");
            require_description(schema, &field_path, missing);
            check_descriptions(schema, &field_path, missing);
        }
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(arr) = obj.get(key).and_then(|a| a.as_array()) {
            for (i, item) in arr.iter().enumerate() {
                let item_path = format!("{path}.{key}[{i}]");
                check_descriptions(item, &item_path, missing);
            }
        }
    }

    if let Some(items) = obj.get("items") {
        check_descriptions(items, &format!("{path}.items"), missing);
    }
}

fn require_description(schema: &serde_json::Value, path: &str, missing: &mut Vec<String>) {
    if let Some(obj) = schema.as_object() {
        // `$ref` indirection: docs live on the referenced definition, not the reference site.
        if obj.contains_key("$ref") {
            return;
        }
        // schemars synthesises the discriminator field on `#[serde(tag = "...")]` enums
        // as `{ "type": "string", "const": "<variant>" }`.
        // It is a marker, not a user-declared field — no rustdoc is meaningful for it.
        if obj.contains_key("const") {
            return;
        }
        match obj.get("description") {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {}
            _ => missing.push(path.to_owned()),
        }
    }
}
