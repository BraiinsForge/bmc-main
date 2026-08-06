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

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::str::FromStr;

use bmc::manifest_test_support::{default_widgets, migration_manifest_expectations, widget_uids};
use bmc_widget_manifest::{CredentialKey, Manifest, ParamKind, ParamValue};

fn load_wasm_manifest(name: &str) -> Manifest {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("BUG: manifest test crate must be below workspace root")
        .join("widgets-wasm")
        .join(name)
        .join("manifest.json");
    let json = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("BUG: read {}: {error}", path.display()));
    Manifest::from_str(&json)
        .unwrap_or_else(|error| panic!("BUG: parse {}: {error}", path.display()))
}

#[test]
fn every_default_widget_matches_its_manifest() {
    let manifests: HashMap<_, _> = [
        "clock",
        "weather",
        "blockheight",
        "mining-info",
        "mining-clock",
    ]
    .into_iter()
    .map(|name| {
        let manifest = load_wasm_manifest(name);
        (manifest.uid, manifest)
    })
    .collect();

    for widget in default_widgets() {
        let manifest = manifests
            .get(&widget.widget_type_id)
            .expect("BUG: default scene uses a widget without a shipped manifest");
        for (key, definition) in &manifest.params {
            assert!(
                definition.is_optional || widget.params.contains_key(key),
                "{:?} default for {} is missing required param {key:?}",
                widget.product,
                manifest.name
            );
        }
        for (key, value) in &widget.params {
            let definition = manifest.params.get(key).unwrap_or_else(|| {
                panic!(
                    "{:?} default for {} carries undeclared param {key:?}",
                    widget.product, manifest.name
                )
            });
            let kind_matches = match value {
                ParamValue::Null => definition.is_optional,
                ParamValue::Boolean(_) => matches!(definition.kind, ParamKind::Boolean { .. }),
                ParamValue::Integer(_) => matches!(definition.kind, ParamKind::Integer { .. }),
                ParamValue::Double(_) => matches!(definition.kind, ParamKind::Double { .. }),
                ParamValue::String(_) => {
                    matches!(
                        definition.kind,
                        ParamKind::String { .. } | ParamKind::Timezone { .. }
                    )
                }
            };
            assert!(
                kind_matches,
                "{:?} default for {} has wrong type for {key:?}",
                widget.product, manifest.name
            );
            if let (ParamValue::String(value), ParamKind::String { enum_values, .. }) =
                (value, &definition.kind)
            {
                assert!(
                    enum_values.is_empty()
                        || enum_values.iter().any(|option| option.value == *value),
                    "{:?} default for {} uses undeclared enum value {value:?}",
                    widget.product,
                    manifest.name
                );
            }
        }
    }
}

#[test]
fn manifest_uids_match_the_shipped_manifests() {
    for (name, uid) in widget_uids() {
        assert_eq!(
            load_wasm_manifest(name).uid,
            uid,
            "{name} UID constant does not match its shipped manifest"
        );
    }
}

fn param_kind<'manifest>(manifest: &'manifest Manifest, key: &str) -> &'manifest ParamKind {
    &manifest
        .params
        .get(key)
        .unwrap_or_else(|| panic!("BUG: manifest has no param {key:?}"))
        .kind
}

fn string_default(manifest: &Manifest, key: &str) -> String {
    let ParamKind::String { default_value, .. } = param_kind(manifest, key) else {
        panic!("BUG: manifest param {key:?} is not a string");
    };
    default_value
        .clone()
        .unwrap_or_else(|| panic!("BUG: manifest param {key:?} has no default"))
}

fn integer_default(manifest: &Manifest, key: &str) -> i32 {
    let ParamKind::Integer { default_value, .. } = param_kind(manifest, key) else {
        panic!("BUG: manifest param {key:?} is not an integer");
    };
    default_value.unwrap_or_else(|| panic!("BUG: manifest param {key:?} has no default"))
}

fn boolean_default(manifest: &Manifest, key: &str) -> bool {
    let ParamKind::Boolean { default_value, .. } = param_kind(manifest, key) else {
        panic!("BUG: manifest param {key:?} is not a boolean");
    };
    default_value.unwrap_or_else(|| panic!("BUG: manifest param {key:?} has no default"))
}

fn string_enum(manifest: &Manifest, key: &str) -> BTreeSet<String> {
    let ParamKind::String { enum_values, .. } = param_kind(manifest, key) else {
        panic!("BUG: manifest param {key:?} is not a string");
    };
    enum_values
        .iter()
        .map(|option| option.value.clone())
        .collect()
}

#[test]
fn migration_fallbacks_match_the_shipped_manifests() {
    let expected = migration_manifest_expectations();
    let clock = load_wasm_manifest("clock");
    let block_height = load_wasm_manifest("blockheight");
    let halving_countdown = load_wasm_manifest("halving-countdown");
    let weather = load_wasm_manifest("weather");
    let image = load_wasm_manifest("image");
    let nameday = load_wasm_manifest("nameday");
    let pool = load_wasm_manifest("braiins-pool");

    assert_eq!(
        string_default(&clock, "numbers_font_style"),
        expected.clock_font
    );
    assert_eq!(
        string_default(&block_height, "numbers_font_style"),
        expected.block_height_font
    );
    assert_eq!(
        string_default(&halving_countdown, "numbers_font_style"),
        expected.halving_countdown_font
    );
    assert_eq!(
        string_default(&weather, "location"),
        expected.weather_location
    );
    assert_eq!(
        string_default(&weather, "time_zone"),
        expected.weather_time_zone
    );
    assert_eq!(
        integer_default(&image, "refresh_seconds"),
        expected.image_refresh_seconds
    );
    assert_eq!(
        string_default(&nameday, "country"),
        expected.nameday_country
    );

    assert_eq!(
        string_enum(&nameday, "country"),
        expected
            .nameday_countries
            .iter()
            .map(|country| (*country).to_owned())
            .collect()
    );
    let font_enum = string_enum(&clock, "numbers_font_style");
    for mapped in expected.translated_font_styles {
        assert!(
            font_enum.contains(mapped),
            "remapped weight {mapped:?} is not in the manifest font enum"
        );
    }

    assert_eq!(string_default(&pool, "style"), expected.pool_style);
    assert_eq!(
        string_default(&pool, "chart_frame"),
        expected.pool_chart_frame
    );
    assert_eq!(
        boolean_default(&pool, "worker_states"),
        expected.pool_worker_states
    );
    let pool_styles = string_enum(&pool, "style");
    for style in expected.pool_styles {
        assert!(
            pool_styles.contains(style),
            "v0 pool style {style:?} is not in the manifest style enum"
        );
    }
    let pool_chart_frames = string_enum(&pool, "chart_frame");
    for frame in expected.translated_pool_chart_frames {
        assert!(
            pool_chart_frames.contains(frame),
            "remapped window {frame:?} is not in the manifest frame enum"
        );
    }
    let credential_slot = CredentialKey::try_new(expected.pool_credential_slot.to_owned())
        .expect("BUG: migration credential slot must be valid");
    assert!(
        pool.credentials.contains_key(&credential_slot),
        "the migration binds a slot the pool manifest does not declare"
    );
}

#[test]
fn ticker_migration_fallbacks_match_the_shipped_manifests() {
    let expected = migration_manifest_expectations();
    let ticker_single = load_wasm_manifest("ticker-single");
    let ticker_list = load_wasm_manifest("ticker-list");

    assert_eq!(string_default(&ticker_single, "pair"), expected.ticker_pair);
    assert_eq!(
        string_default(&ticker_single, "period"),
        expected.ticker_period
    );
    assert_eq!(
        string_default(&ticker_list, "period"),
        expected.ticker_period
    );
    for (index, default) in expected.ticker_list_symbols.iter().enumerate() {
        assert_eq!(
            &string_default(&ticker_list, &format!("symbol_{}", index + 1)),
            default
        );
    }

    let single_periods = string_enum(&ticker_single, "period");
    let list_periods = string_enum(&ticker_list, "period");
    for mapped in expected.translated_ticker_periods {
        assert!(
            single_periods.contains(mapped) && list_periods.contains(mapped),
            "remapped period {mapped:?} is not in the ticker period enums"
        );
    }
    for mapped in expected.translated_btc_time_frames {
        assert!(
            single_periods.contains(mapped),
            "remapped time frame {mapped:?} is not in the ticker-single period enum"
        );
    }
    let views = string_enum(&ticker_single, "view");
    for view in expected.ticker_views {
        assert!(
            views.contains(view),
            "dispatched view {view:?} is not in the ticker-single view enum"
        );
    }
}
