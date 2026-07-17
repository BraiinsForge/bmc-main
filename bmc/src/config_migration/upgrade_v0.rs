// Copyright (C) 2025  Braiins Systems s.r.o.

//! v0 → current schema upgrade.
//!
//! Implements [`Upgrade`] for [`v0::Config`], producing a typed
//! [`crate::config::Config`] directly.
//!
//! Policy — aligned with review feedback:
//!
//! - **No intermediate format for unsupported widgets.** Every v0
//!   widget either maps to a reserved `widget_type_id` in the
//!   current schema or is dropped outright with a `warn!`. There is
//!   no `_legacy` / `_legacy_remote` placeholder in the output.
//! - **Each mapped widget targets a real manifest UID.** Native
//!   kinds (`clock`, `block_height`, `remote_image`) map to the
//!   `uid` declared in their `widgets-wasm/*/manifest.json`.
//!   Braiinsforge-hosted remote widgets use deterministic UUID v5
//!   derived from their URL slug, so adding a new remote widget is
//!   one line in [`REMOTE_WIDGET_SLUGS`] without any UUID bookkeeping.
//! - **Deep translation where param shape changed.** `clock`,
//!   `block_height`, and `remote_image` get value-level translation
//!   (font-weight vocabulary, humantime → seconds, enum renames)
//!   into their shipped manifest's param names. Remote widgets pass
//!   their inner `params` through untouched; that widget's manifest
//!   is authoritative over its own schema and can migrate internally
//!   when it loads.
//! - **Unknown v0 kinds or unrecognised remote-widget URLs drop.**
//!   Per review, users migrate all widgets at once; an unmappable
//!   widget is an edge case, not an inter-state we need to preserve.

use std::collections::BTreeMap;
use std::time::Duration;

use bmc_widget_manifest::{ParamKey, ParamValue, ViewportShape};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use tracing::warn;
use uuid::Uuid;

use super::{Report, Upgrade, Version, v0};
use crate::config::Config;
use crate::scene::{
    Scene, SceneId, SceneKind, Widget, WidgetId, WidgetPlacement, WidgetPosition, WidgetSize,
};

// --- Reserved UIDs for native v0 widget kinds -------------------------------
//
// Each constant is the real `uid` declared in the corresponding widget's
// `manifest.json` under `widgets-wasm/`:
//   CLOCK_UID        -> widgets-wasm/clock
//   BLOCK_HEIGHT_UID -> widgets-wasm/blockheight
//   REMOTE_IMAGE_UID -> widgets-wasm/image

const CLOCK_UID: Uuid = Uuid::from_u128(0xfbc8_67c9_b722_4bdb_8738_c15d_20fe_2b88);
const BLOCK_HEIGHT_UID: Uuid = Uuid::from_u128(0x7cb5_84a8_1f26_42a0_867e_955a_add2_391c);
const REMOTE_IMAGE_UID: Uuid = Uuid::from_u128(0xf9e4_956c_719d_450c_909d_4fc9_d444_0e15);

// --- Reserved UIDs for Braiinsforge remote widgets --------------------------

/// Prefix that identifies a Braiinsforge-hosted remote widget URL.
/// We don't use the `url` crate here — the match is a plain prefix
/// strip, kept deliberately tight so unrelated URLs drop out.
const BRAIINSFORGE_URL_PREFIX: &str = "https://widgets.braiinsforge.com/";

/// UUID namespace for Braiinsforge remote widget UIDs. Hand-crafted
/// constant (not derived): this binds `Uuid::new_v5(NS, slug)` to
/// stable values regardless of what hashing implementation the
/// `uuid` crate uses internally. Changing this constant changes
/// every remote-widget UID, so don't.
const BRAIINSFORGE_WIDGETS_NS: Uuid = Uuid::from_u128(0xb1a1_1f06_4444_4444_8000_0000_0000_0000);

/// The canonical list of Braiinsforge remote-widget slugs. A legacy
/// `remote_widget` whose `widget_url` matches `<prefix><slug>` is
/// mapped to `Uuid::new_v5(NS, slug.as_bytes())`; anything else
/// drops. Adding a widget = one line here.
const REMOTE_WIDGET_SLUGS: &[&str] = &[
    "exchange-rate",
    "formula-1",
    "iss-position",
    "nameday",
    "nasa-picture-of-the-day",
    "random-facts",
    "spacex-launch",
    "ticker-list",
    "ticker-single-candlestick",
    "ticker-single-sparkline",
    "weather",
];

// --- Upgrade entry points ----------------------------------------------------

impl Upgrade for v0::Config {
    type NextVersion = Config;

    fn upgrade_to_next_version(self) -> Config {
        upgrade_with_report(self).0
    }
}

/// Core upgrade. Returns both the upgraded [`Config`] and the
/// [`Report`] counts. [`Upgrade::upgrade_to_next_version`] delegates
/// here so the trait conforms to the boser-style shape while the
/// caller that wants counts (`LoadedConfig::from_str`) can get them
/// without re-walking the result.
pub(super) fn upgrade_with_report(v0: v0::Config) -> (Config, Report) {
    let mut report = Report::default();

    let scenes: IndexMap<SceneId, Scene> = v0
        .scenes
        .iter()
        .filter_map(|scene| upgrade_scene(scene, &mut report).map(|scene| (scene.id, scene)))
        .collect();

    // The current `Config` has more fields than v0 ever knew about
    // (localization, night mode, alarms, autoupgrade, …). Only the
    // scene layout and accounts carry over; `from_migrated_parts`
    // pins the schema version and lets every other field fall back to
    // the same defaults a field-less current config would use.
    let accounts = deserialize_accounts_passthrough(v0.accounts);
    let current = Config::from_migrated_parts(scenes, accounts);
    (current, report)
}

/// Sanity assertions — the chain terminates at the current schema,
/// and version numbers are adjacent.
const _: () = {
    const _V0_IS_ZERO: () = assert!(v0::Config::VERSION == 0);
    const _CURRENT_IS_ONE_ABOVE_V0: () = assert!(Config::VERSION == v0::Config::VERSION + 1);
};

// --- Per-widget dispatch -----------------------------------------------------

fn upgrade_scene(scene: &v0::Scene, report: &mut Report) -> Option<Scene> {
    let widgets: IndexMap<WidgetId, Widget> = scene
        .widgets
        .iter()
        .filter_map(|widget| {
            if let Some(w) = upgrade_widget(widget) {
                report.translated_widgets += 1;
                Some((w.id, w))
            } else {
                report.dropped_widgets += 1;
                None
            }
        })
        .collect();

    if widgets.is_empty() {
        report.dropped_scenes += 1;
        return None;
    }

    report.scenes += 1;
    Some(Scene {
        id: SceneId::from(scene.id),
        enabled: scene.enabled,
        cycle_duration: None,
        kind: match scene.kind {
            v0::SceneKind::Fullscreen => SceneKind::Fullscreen,
            v0::SceneKind::Combined => SceneKind::Combined,
        },
        widgets,
    })
}

/// Map a v0 widget to a current-schema [`Widget`], or drop it.
///
/// A `Some` return always carries a non-nil `widget_type_id`; there
/// is no placeholder bucket. Callers treat `None` as "this widget
/// does not survive the upgrade" and count it accordingly.
fn upgrade_widget(widget: &v0::Widget) -> Option<Widget> {
    let (widget_type_id, params) = match widget.kind.as_str() {
        "clock" => dispatch_clock(widget),
        "block_height" => dispatch_block_height(widget),
        "remote_image" => dispatch_remote_image(widget),
        "remote_widget" => dispatch_remote_widget(widget)?,
        other => {
            warn!(
                kind = %other,
                id = %widget.id,
                "legacy widget kind has no mapping in the current schema; dropping"
            );
            return None;
        }
    };

    Some(Widget {
        id: WidgetId::from(widget.id),
        position: WidgetPosition {
            row: widget.row,
            col: widget.col,
        },
        placement: WidgetPlacement::from(parse_size(&widget.size)),
        widget_type_id,
        // v0 had no per-widget viewport shape; the current default is
        // rectangular. A future manifest can override it when it loads.
        viewport_shape: ViewportShape::Rectangular,
        params: params_from_value(params),
    })
}

/// Convert a legacy free-form JSON params blob into the current typed
/// param map. v0 stored params as arbitrary JSON; the current
/// [`Widget`] constrains them to a flat map of scalar [`ParamValue`]s.
///
/// A non-object blob yields an empty map. Individual entries whose key
/// is not a valid [`ParamKey`] or whose value is not a scalar the new
/// schema can hold (nested objects/arrays, non-finite numbers) are
/// dropped with a `warn!` — the same "drop the unmappable, never fail
/// the whole migration" stance applied at the widget level.
fn params_from_value(value: Value) -> BTreeMap<ParamKey, ParamValue> {
    let Value::Object(entries) = value else {
        if !value.is_null() {
            warn!("legacy widget params were not a JSON object; dropping them");
        }
        return BTreeMap::new();
    };

    let mut out = BTreeMap::new();
    for (key, raw) in entries {
        let Ok(param_key) = ParamKey::try_new(key.clone()) else {
            warn!(key = %key, "legacy param key is not a valid ParamKey; dropping");
            continue;
        };
        match serde_json::from_value::<ParamValue>(raw) {
            Ok(param_value) => {
                out.insert(param_key, param_value);
            }
            Err(err) => {
                warn!(
                    key = %key,
                    error = %err,
                    "legacy param value is not a scalar the current schema can hold; dropping"
                );
            }
        }
    }
    out
}

/// Translate a legacy `clock` widget into the current unified Clock
/// widget ([`CLOCK_UID`]), which folds the old digital/analog styles
/// behind its `clock_style` param.
///
/// The shipped manifest (`widgets-wasm/clock/manifest.json`) keeps
/// v0's snake_case param names verbatim, so this is a value-level
/// migration, not a rename. We rebuild the param object from the
/// manifest's known keys — dropping any legacy key the manifest does
/// not declare, and leaving absent params out so the widget applies
/// its own manifest defaults rather than pinning them here. The one
/// real transformation is `numbers_font_style`, whose vocabulary
/// changed from `light`/`medium`/`bold` to `regular`/`semi-bold`/`bold`.
fn dispatch_clock(widget: &v0::Widget) -> (Uuid, Value) {
    let mut params = Map::new();

    // `clock_style` shares its vocabulary across v0 and the current
    // manifest (`digital` / `analog_round` / `analog_rect`), so it
    // passes through unchanged when present.
    if let Some(style) = widget.params.get("clock_style").and_then(Value::as_str) {
        params.insert("clock_style".to_owned(), json!(style));
    }

    // Booleans carry identical meaning on both sides; copy the ones
    // that are present and actually boolean-typed, so a malformed v0
    // value falls back to the manifest default instead of migrating a
    // wrong-typed param.
    for key in ["show_date", "show_seconds", "show_timezone"] {
        if let Some(flag) = widget.params.get(key).and_then(Value::as_bool) {
            params.insert(key.to_owned(), json!(flag));
        }
    }

    // Font-weight vocabulary changed, so a raw pass-through could emit
    // an enum value the manifest no longer accepts. Always remap,
    // falling back to the clock manifest's own default weight.
    let font_style = migrate_font_style(widget, "semi-bold");
    params.insert("numbers_font_style".to_owned(), json!(font_style));

    (CLOCK_UID, Value::Object(params))
}

/// Translate a legacy `block_height` widget into the current Block
/// Height widget ([`BLOCK_HEIGHT_UID`]). Like the clock, the shipped
/// manifest (`widgets-wasm/blockheight/manifest.json`) keeps v0's
/// param names, so the only real transformation is the
/// `numbers_font_style` vocabulary remap; `show_timestamp` passes
/// through when present, and an absent param defers to the manifest
/// default.
fn dispatch_block_height(widget: &v0::Widget) -> (Uuid, Value) {
    let mut params = Map::new();

    if let Some(flag) = widget.params.get("show_timestamp").and_then(Value::as_bool) {
        params.insert("show_timestamp".to_owned(), json!(flag));
    }

    // The block-height manifest defaults this weight to `bold`, unlike
    // the clock's `semi-bold`; only the vocabulary remap is shared.
    let font_style = migrate_font_style(widget, "bold");
    params.insert("numbers_font_style".to_owned(), json!(font_style));

    (BLOCK_HEIGHT_UID, Value::Object(params))
}

/// Read a v0 `numbers_font_style`, remap it to the current
/// `regular`/`semi-bold`/`bold` vocabulary, and fall back to
/// `default` when the param is absent or carries a weight outside the
/// v0 set. `default` is the target widget's own manifest default,
/// which differs per widget (clock `semi-bold`, block height `bold`).
fn migrate_font_style(widget: &v0::Widget, default: &'static str) -> &'static str {
    widget
        .params
        .get("numbers_font_style")
        .and_then(Value::as_str)
        .and_then(translate_font_style)
        .unwrap_or(default)
}

/// Map a v0 numeral font weight (`light`/`medium`/`bold`) to the
/// current shared vocabulary (`regular`/`semi-bold`/`bold`). `None`
/// for a weight outside the v0 set, so callers can substitute their
/// own per-widget manifest default.
fn translate_font_style(font_style: &str) -> Option<&'static str> {
    match font_style {
        "light" => Some("regular"),
        "medium" => Some("semi-bold"),
        "bold" => Some("bold"),
        _ => None,
    }
}

/// Translate a legacy `remote_image` widget into the current Image
/// widget ([`REMOTE_IMAGE_UID`], `widgets-wasm/image/manifest.json`).
///
/// Two params changed shape between v0 and the current manifest:
///
/// - `refresh_duration` (humantime string, e.g. `"1h"`) became
///   `refresh_seconds` (integer seconds).
/// - `image_scale_mode` (`fit` / `fill`) became `sizing`
///   (`contain` / `cover`).
///
/// `url` keeps its name and passes through. Absent, wrong-typed, or
/// unparseable params fall back to the manifest defaults.
fn dispatch_remote_image(widget: &v0::Widget) -> (Uuid, Value) {
    let mut params = Map::new();

    if let Some(url) = widget.params.get("url").and_then(Value::as_str) {
        params.insert("url".to_owned(), json!(url));
    }

    // v0 stored the refresh interval as a humantime string; the
    // current manifest wants integer seconds. Reparse through the same
    // `humantime_serde` machinery the rest of the config uses, falling
    // back to the manifest default when it is absent or unparseable.
    let refresh_seconds = widget
        .params
        .get("refresh_duration")
        .and_then(|v| humantime_serde::deserialize::<Duration, _>(v).ok())
        .map_or(3600, |d| d.as_secs());
    params.insert("refresh_seconds".to_owned(), json!(refresh_seconds));

    // `image_scale_mode` (`fit` / `fill`) was renamed to `sizing`
    // (`contain` / `cover`). Anything other than `fill` — v0 `fit`, an
    // unknown value, or an absent param — lands on the manifest default
    // `contain`.
    let sizing = match widget
        .params
        .get("image_scale_mode")
        .and_then(Value::as_str)
    {
        Some("fill") => "cover",
        _ => "contain",
    };
    params.insert("sizing".to_owned(), json!(sizing));

    (REMOTE_IMAGE_UID, Value::Object(params))
}

/// Map a legacy `remote_widget` to a reserved remote-widget UID via
/// its `widget_url` slug. URLs outside the Braiinsforge host, or
/// slugs not in [`REMOTE_WIDGET_SLUGS`], are dropped. The inner
/// `params` field of the legacy `RemoteWidget` becomes the new
/// widget's params verbatim; the now-redundant metadata (`name`,
/// `description`, `widget_url`, `icon_url`) is dropped because the
/// UID itself encodes widget identity.
fn dispatch_remote_widget(widget: &v0::Widget) -> Option<(Uuid, Value)> {
    let Some(url) = widget.params.get("widget_url").and_then(Value::as_str) else {
        warn!(
            id = %widget.id,
            "remote_widget missing widget_url; dropping"
        );
        return None;
    };

    let Some(slug) = remote_widget_slug(url) else {
        warn!(
            id = %widget.id,
            url = %url,
            "remote_widget URL not hosted on widgets.braiinsforge.com; dropping"
        );
        return None;
    };

    let Some(uid) = reserved_remote_widget_uid(slug) else {
        warn!(
            id = %widget.id,
            url = %url,
            slug = %slug,
            "remote_widget slug not in Braiinsforge catalog; dropping"
        );
        return None;
    };

    // Inner `params` — what the legacy remote widget actually ran
    // with — becomes the new widget's params. Future shipped
    // manifest for this slug is authoritative over the param schema.
    let inner = widget.params.get("params").cloned().unwrap_or(Value::Null);
    Some((uid, inner))
}

/// Extract the first path segment after the Braiinsforge widget
/// host. `None` for URLs not matching the prefix or with empty
/// paths. Also terminates the segment at `?` or `#` so that URLs
/// carrying a query string (`…/weather?lat=…`) or fragment
/// (`…/weather#foo`) round-tripped through the legacy config still
/// match the `weather` slug.
fn remote_widget_slug(widget_url: &str) -> Option<&str> {
    widget_url
        .strip_prefix(BRAIINSFORGE_URL_PREFIX)?
        .trim_end_matches('/')
        .split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())
}

/// UID for a known Braiinsforge remote-widget slug, or `None` if
/// the slug is not in the catalog.
fn reserved_remote_widget_uid(slug: &str) -> Option<Uuid> {
    REMOTE_WIDGET_SLUGS
        .iter()
        .find(|s| **s == slug)
        .map(|s| Uuid::new_v5(&BRAIINSFORGE_WIDGETS_NS, s.as_bytes()))
}

fn parse_size(size: &str) -> WidgetSize {
    match size {
        "small" => WidgetSize::Small,
        "medium" => WidgetSize::Medium,
        "large" => WidgetSize::Large,
        "full" => WidgetSize::Full,
        other => {
            warn!(
                size = %other,
                "legacy widget carried an unknown size; defaulting to full"
            );
            WidgetSize::Full
        }
    }
}

// --- Accounts pass-through ---------------------------------------------------

/// Re-parse the v0 `accounts` array through the current `Account`
/// type. The shape is identical on both sides so this is a validate
/// step, not a transformation; any malformed entry is logged and
/// dropped rather than failing the whole migration.
fn deserialize_accounts_passthrough(
    accounts: Vec<Value>,
) -> IndexMap<crate::data::AccountId, crate::data::Account> {
    use crate::data::{Account, AccountId};

    let mut out = IndexMap::<AccountId, Account>::new();
    for (idx, raw) in accounts.into_iter().enumerate() {
        match serde_json::from_value::<Account>(raw) {
            Ok(account) => {
                out.insert(account.id.clone(), account);
            }
            Err(err) => {
                warn!(
                    index = idx,
                    error = %err,
                    "legacy account dropped: failed to parse into current schema"
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_widget(kind: &str, params: Value) -> v0::Widget {
        v0::Widget {
            id: Uuid::nil(),
            row: 0,
            col: 0,
            size: "full".into(),
            kind: kind.into(),
            params,
        }
    }

    /// Build the current typed param map from `(key, value)` pairs so
    /// pass-through expectations can be stated without leaning on
    /// [`params_from_value`] (which is what we're checking against).
    fn param_map(pairs: &[(&str, ParamValue)]) -> BTreeMap<ParamKey, ParamValue> {
        pairs
            .iter()
            .map(|(k, v)| {
                (
                    ParamKey::try_new((*k).to_owned()).expect("BUG: test param key must be valid"),
                    v.clone(),
                )
            })
            .collect()
    }

    /// Upgrade a single widget of `kind` and unwrap it, panicking if
    /// it was dropped. For the many cases that must survive.
    fn upgrade(kind: &str, params: Value) -> Widget {
        upgrade_widget(&mk_widget(kind, params))
            .unwrap_or_else(|| panic!("BUG: {kind} widget must survive the upgrade"))
    }

    /// Shorthand for a `ParamValue::String`.
    fn str_param(value: &str) -> ParamValue {
        ParamValue::String(value.to_owned())
    }

    // --- clock ---------------------------------------------------------------

    #[test]
    fn clock_maps_to_clock_uid() {
        assert_eq!(upgrade("clock", json!({})).widget_type_id, CLOCK_UID);
    }

    #[test]
    fn clock_remaps_font_style_vocabulary() {
        for (v0_weight, expected) in [
            ("light", "regular"),
            ("medium", "semi-bold"),
            ("bold", "bold"),
        ] {
            let upgraded = upgrade("clock", json!({ "numbers_font_style": v0_weight }));
            assert_eq!(
                upgraded.params["numbers_font_style"],
                str_param(expected),
                "v0 `{v0_weight}` must map to `{expected}`"
            );
        }
    }

    #[test]
    fn clock_missing_or_unknown_font_style_defaults_to_semi_bold() {
        for params in [json!({}), json!({ "numbers_font_style": "gigantic" })] {
            let upgraded = upgrade("clock", params);
            assert_eq!(
                upgraded.params["numbers_font_style"],
                str_param("semi-bold")
            );
        }
    }

    #[test]
    fn clock_passes_through_style_and_booleans() {
        let upgraded = upgrade(
            "clock",
            json!({
                "clock_style": "analog_round",
                "show_date": false,
                "show_seconds": true,
                "show_timezone": true,
            }),
        );
        assert_eq!(upgraded.params["clock_style"], str_param("analog_round"));
        assert_eq!(upgraded.params["show_date"], ParamValue::Boolean(false));
        assert_eq!(upgraded.params["show_seconds"], ParamValue::Boolean(true));
        assert_eq!(upgraded.params["show_timezone"], ParamValue::Boolean(true));
    }

    #[test]
    fn clock_drops_keys_outside_the_manifest() {
        let upgraded = upgrade(
            "clock",
            json!({ "clock_style": "digital", "legacy_junk": "x" }),
        );
        assert!(!upgraded.params.contains_key("legacy_junk"));
        // Only `clock_style` plus the always-set `numbers_font_style`.
        assert_eq!(upgraded.params.len(), 2);
    }

    // --- block height --------------------------------------------------------

    #[test]
    fn block_height_maps_to_uid_and_defaults_font_to_bold() {
        let upgraded = upgrade("block_height", json!({}));
        assert_eq!(upgraded.widget_type_id, BLOCK_HEIGHT_UID);
        assert_eq!(upgraded.params["numbers_font_style"], str_param("bold"));
    }

    #[test]
    fn block_height_remaps_font_and_passes_timestamp() {
        let upgraded = upgrade(
            "block_height",
            json!({ "numbers_font_style": "medium", "show_timestamp": false }),
        );
        assert_eq!(
            upgraded.params["numbers_font_style"],
            str_param("semi-bold")
        );
        assert_eq!(
            upgraded.params["show_timestamp"],
            ParamValue::Boolean(false)
        );
    }

    // --- remote image --------------------------------------------------------

    #[test]
    fn remote_image_maps_to_uid() {
        assert_eq!(
            upgrade("remote_image", json!({})).widget_type_id,
            REMOTE_IMAGE_UID
        );
    }

    #[test]
    fn remote_image_parses_humantime_refresh_into_seconds() {
        for (human, secs) in [("30s", 30), ("5m", 300), ("1h", 3600)] {
            let upgraded = upgrade("remote_image", json!({ "refresh_duration": human }));
            assert_eq!(
                upgraded.params["refresh_seconds"],
                ParamValue::Integer(secs),
                "`{human}` must become {secs} seconds"
            );
        }
    }

    #[test]
    fn remote_image_missing_or_unparseable_refresh_defaults_to_3600() {
        for params in [json!({}), json!({ "refresh_duration": "not a duration" })] {
            let upgraded = upgrade("remote_image", params);
            assert_eq!(
                upgraded.params["refresh_seconds"],
                ParamValue::Integer(3600)
            );
        }
    }

    #[test]
    fn remote_image_renames_scale_mode_to_sizing() {
        for (mode, sizing) in [("fit", "contain"), ("fill", "cover")] {
            let upgraded = upgrade("remote_image", json!({ "image_scale_mode": mode }));
            assert_eq!(upgraded.params["sizing"], str_param(sizing));
        }
    }

    #[test]
    fn remote_image_missing_or_unknown_scale_mode_defaults_to_contain() {
        for params in [json!({}), json!({ "image_scale_mode": "warp" })] {
            let upgraded = upgrade("remote_image", params);
            assert_eq!(upgraded.params["sizing"], str_param("contain"));
        }
    }

    #[test]
    fn remote_image_passes_url_through() {
        let upgraded = upgrade(
            "remote_image",
            json!({ "url": "https://example.com/a.png" }),
        );
        assert_eq!(
            upgraded.params["url"],
            str_param("https://example.com/a.png")
        );
    }

    #[test]
    fn unknown_kind_drops() {
        let w = mk_widget("mystery_widget", json!({}));
        assert!(upgrade_widget(&w).is_none());
    }

    // --- scene dropping ------------------------------------------------------

    fn scene_with(widgets: Vec<v0::Widget>) -> v0::Scene {
        v0::Scene {
            id: Uuid::nil(),
            enabled: true,
            kind: v0::SceneKind::Fullscreen,
            widgets,
        }
    }

    #[test]
    fn scene_with_only_unmappable_widgets_is_dropped() {
        let scene = scene_with(vec![mk_widget("mystery_widget", json!({}))]);
        let (_current, report) = upgrade_with_report(v0::Config {
            scenes: vec![scene],
            accounts: vec![],
        });
        assert_eq!(report.scenes, 0, "the empty scene must not be kept");
        assert_eq!(report.dropped_scenes, 1);
        assert_eq!(report.dropped_widgets, 1);
    }

    #[test]
    fn scene_with_a_survivor_is_kept() {
        let scene = scene_with(vec![mk_widget("clock", json!({}))]);
        let (_current, report) = upgrade_with_report(v0::Config {
            scenes: vec![scene],
            accounts: vec![],
        });
        assert_eq!(report.scenes, 1);
        assert_eq!(report.dropped_scenes, 0);
        assert_eq!(report.translated_widgets, 1);
    }

    #[test]
    fn remote_widget_known_slug_is_translated() {
        let inner = json!({ "city": "Brno", "units": "c" });
        let w = mk_widget(
            "remote_widget",
            json!({
                "name": "Weather",
                "description": "",
                "widget_url": "https://widgets.braiinsforge.com/weather",
                "icon_url": "",
                "params": inner.clone(),
            }),
        );
        let upgraded =
            upgrade_widget(&w).expect("BUG: weather slug must resolve to a reserved UID");
        assert_ne!(upgraded.widget_type_id, Uuid::nil());
        // Inner params survive verbatim; legacy metadata (name, URLs) drops.
        assert_eq!(
            upgraded.params,
            param_map(&[
                ("city", ParamValue::String("Brno".to_owned())),
                ("units", ParamValue::String("c".to_owned())),
            ])
        );
    }

    #[test]
    fn remote_widget_uid_is_deterministic_for_weather() {
        // Pinned literal, computed offline once from
        // `Uuid::new_v5(BRAIINSFORGE_WIDGETS_NS, b"weather")`.
        // Deriving "expected" via `new_v5(NS, ...)` inline would
        // move in lockstep with any accidental change to the
        // namespace constant, defeating the point of the test.
        // A literal pins the contract: if either the namespace
        // constant or the v5 implementation shifts, this test
        // fails.
        let expected = Uuid::from_u128(0x1042_a953_d87a_57ca_aba2_0a3b_2f99_af50);
        assert_eq!(reserved_remote_widget_uid("weather"), Some(expected));
    }

    #[test]
    fn remote_widget_uid_is_distinct_per_slug() {
        let weather = reserved_remote_widget_uid("weather").expect("BUG: weather must be known");
        let iss = reserved_remote_widget_uid("iss-position").expect("BUG: iss must be known");
        assert_ne!(weather, iss);
    }

    #[test]
    fn remote_widget_unknown_host_drops() {
        let w = mk_widget(
            "remote_widget",
            json!({
                "widget_url": "https://example.com/weather",
                "params": {},
            }),
        );
        assert!(upgrade_widget(&w).is_none());
    }

    #[test]
    fn remote_widget_unknown_slug_drops() {
        let w = mk_widget(
            "remote_widget",
            json!({
                "widget_url": "https://widgets.braiinsforge.com/not-a-real-widget",
                "params": {},
            }),
        );
        assert!(upgrade_widget(&w).is_none());
    }

    #[test]
    fn remote_widget_missing_url_drops() {
        let w = mk_widget("remote_widget", json!({ "params": {} }));
        assert!(upgrade_widget(&w).is_none());
    }

    #[test]
    fn remote_widget_url_with_query_and_fragment_still_matches_slug() {
        // Legacy configs that round-tripped through a browser (or
        // a debug probe) might carry `?query` or `#fragment`
        // suffixes on the widget URL. The slug extractor must
        // terminate at those delimiters so `weather?lat=50.1`
        // still resolves to the `weather` slug.
        for url in [
            "https://widgets.braiinsforge.com/weather?lat=50.1&lon=14.4",
            "https://widgets.braiinsforge.com/weather#details",
            "https://widgets.braiinsforge.com/weather/?foo",
        ] {
            let w = mk_widget("remote_widget", json!({ "widget_url": url, "params": {} }));
            assert!(
                upgrade_widget(&w).is_some(),
                "URL `{url}` must resolve to the weather slug"
            );
        }
    }

    #[test]
    fn every_catalog_slug_resolves() {
        for slug in REMOTE_WIDGET_SLUGS {
            assert!(
                reserved_remote_widget_uid(slug).is_some(),
                "BUG: slug {slug} declared in REMOTE_WIDGET_SLUGS must resolve"
            );
        }
    }

    #[test]
    fn upgraded_config_carries_current_version() {
        let v0 = v0::Config {
            scenes: vec![],
            accounts: vec![],
        };
        let upgraded = v0.upgrade_to_next_version();
        assert_eq!(upgraded.version, Config::VERSION);
    }
}
