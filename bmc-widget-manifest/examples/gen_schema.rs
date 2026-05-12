// Copyright (C) 2026  Braiins Systems s.r.o.
//
// Regenerator for `manifest.schema.json`. Invoke via
// `just widget-manifest::gen-schema` from the workspace root.
// The committed schema is the source of truth for editor tooling;
// the `schema_drift_guard` integration test enforces that this
// generator's current output matches the committed file byte-for-byte.

use bmc_widget_manifest::Manifest;

fn main() {
    let schema = schemars::schema_for!(Manifest);
    let rendered = serde_json::to_string_pretty(&schema).expect("BUG: schema must serialise");
    println!("{rendered}");
}
