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
//! - **Each known widget class has a reserved UID.** Native kinds
//!   (`clock`, `ticker_btc`, …) use sequential UIDs matching the
//!   manifest convention (`550e8400-e29b-41d4-a716-44665544000N`).
//!   Braiinsforge-hosted remote widgets use deterministic UUID v5
//!   derived from their URL slug, so adding a new remote widget is
//!   one line in [`REMOTE_WIDGET_SLUGS`] without any UUID bookkeeping.
//! - **Deep translators only where the target manifest already
//!   ships.** Today that is just the digital-clock variant of the
//!   legacy `clock` widget. Everything else passes `params` through
//!   untouched; the future widget's manifest is authoritative over
//!   its own param schema and can migrate internally when it loads.
//! - **Unknown v0 kinds or unrecognised remote-widget URLs drop.**
//!   Per review, users migrate all widgets at once; an unmappable
//!   widget is an edge case, not an inter-state we need to preserve.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use super::{Report, Upgrade, Version, v0};
use crate::config::Config;
use crate::scene::{Scene, SceneId, SceneKind, Widget, WidgetId, WidgetPosition, WidgetSize};

// --- Reserved UIDs for native v0 widget kinds -------------------------------
//
// The digital-clock and flip-clock UIDs are the real values declared in
// the widgets' `manifest.json` files. The remaining IDs are dummies
// reserved in advance so that migrated configs can reference the
// eventual manifest widget without re-migration when it ships. Order
// matches the `WidgetKind` enum in `bmc-display/src/data.rs`.

const DIGITAL_CLOCK_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0001);
const ANALOG_ROUND_CLOCK_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0003);
const ANALOG_RECT_CLOCK_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0004);
const TICKER_BTC_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0005);
const BLOCK_HEIGHT_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0006);
const BRAIINS_POOL_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0007);
const REMOTE_IMAGE_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0008);
const BLOCKCHAIN_DATA_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0009);
const HALVING_COUNTDOWN_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_000a);

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
    let mut report = Report {
        scenes: v0.scenes.len(),
        ..Report::default()
    };

    let scenes: IndexMap<SceneId, Scene> = v0
        .scenes
        .iter()
        .map(|scene| {
            let converted = upgrade_scene(scene, &mut report);
            (converted.id, converted)
        })
        .collect();

    // The current `Config` has more fields than v0 ever knew about
    // (localization, night mode, alarms, autoupgrade, …). Start from
    // `Default::default()` and overwrite only what v0 can derive.
    // Everything else inherits sensible defaults from the current
    // schema.
    let mut current = Config::default();
    current.scenes = scenes;
    current.accounts = deserialize_accounts_passthrough(v0.accounts);
    (current, report)
}

/// Sanity assertions — the chain terminates at the current schema,
/// and version numbers are adjacent.
const _: () = {
    const _V0_IS_ZERO: () = assert!(v0::Config::VERSION == 0);
    const _CURRENT_IS_ONE_ABOVE_V0: () = assert!(Config::VERSION == v0::Config::VERSION + 1);
};

// --- Per-widget dispatch -----------------------------------------------------

fn upgrade_scene(scene: &v0::Scene, report: &mut Report) -> Scene {
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

    Scene {
        id: SceneId::from(scene.id),
        enabled: scene.enabled,
        cycle_duration: None,
        kind: match scene.kind {
            v0::SceneKind::Fullscreen => SceneKind::Fullscreen,
            v0::SceneKind::Combined => SceneKind::Combined,
        },
        widgets,
    }
}

/// Map a v0 widget to a current-schema [`Widget`], or drop it.
///
/// A `Some` return always carries a non-nil `widget_type_id`; there
/// is no placeholder bucket. Callers treat `None` as "this widget
/// does not survive the upgrade" and count it accordingly.
fn upgrade_widget(widget: &v0::Widget) -> Option<Widget> {
    let (widget_type_id, params) = match widget.kind.as_str() {
        "clock" => dispatch_clock(widget)?,
        "ticker_btc" => (TICKER_BTC_UID, widget.params.clone()),
        "block_height" => (BLOCK_HEIGHT_UID, widget.params.clone()),
        "braiins_pool" => (BRAIINS_POOL_UID, widget.params.clone()),
        "remote_image" => (REMOTE_IMAGE_UID, widget.params.clone()),
        "blockchain_data" => (BLOCKCHAIN_DATA_UID, widget.params.clone()),
        "halving_countdown" => (HALVING_COUNTDOWN_UID, widget.params.clone()),
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
        size: parse_size(&widget.size),
        widget_type_id,
        params,
    })
}

/// Legacy `clock` has three styles: `digital`, `analog_round`,
/// `analog_rect`. Only `digital` has a shipped target manifest
/// today, so only that style gets a deep param translation; the
/// analog variants use reserved UIDs with their original params
/// passed through. A missing `clock_style` is treated as `digital`
/// for backwards compatibility with very old configs.
fn dispatch_clock(widget: &v0::Widget) -> Option<(Uuid, Value)> {
    let style = widget.params.get("clock_style").and_then(Value::as_str);
    match style {
        Some("digital") | None => Some(translate_clock_digital(widget)),
        Some("analog_round") => Some((ANALOG_ROUND_CLOCK_UID, widget.params.clone())),
        Some("analog_rect") => Some((ANALOG_RECT_CLOCK_UID, widget.params.clone())),
        Some(other) => {
            warn!(
                clock_style = %other,
                id = %widget.id,
                "unknown clock_style; dropping"
            );
            None
        }
    }
}

/// Deep translator for the digital-clock variant. Maps v0 param
/// names to the names used by the shipped `digital-clock` manifest
/// (`showSeconds`, `showTimezone`, `fontStyle`) and drops
/// `show_date` with a warning (no equivalent on the new manifest).
fn translate_clock_digital(widget: &v0::Widget) -> (Uuid, Value) {
    let show_seconds = widget
        .params
        .get("show_seconds")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let show_timezone = widget
        .params
        .get("show_timezone")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let font_style = widget
        .params
        .get("numbers_font_style")
        .and_then(Value::as_str)
        .filter(|s| matches!(*s, "light" | "medium" | "bold"))
        .unwrap_or("medium");

    if widget.params.get("show_date").and_then(Value::as_bool) == Some(true) {
        warn!(
            id = %widget.id,
            "legacy clock had `show_date: true`; dropped (digital-clock manifest has no date param)"
        );
    }

    (
        DIGITAL_CLOCK_UID,
        json!({
            "showSeconds": show_seconds,
            "showTimezone": show_timezone,
            "fontStyle": font_style,
        }),
    )
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
) -> IndexMap<bmc_display::data::AccountId, bmc_display::data::Account> {
    use bmc_display::data::{Account, AccountId};

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

    #[test]
    fn digital_clock_deep_translated() {
        let w = mk_widget(
            "clock",
            json!({
                "clock_style": "digital",
                "numbers_font_style": "bold",
                "show_seconds": false,
                "show_timezone": true,
                "show_date": true,
            }),
        );
        let upgraded = upgrade_widget(&w).expect("BUG: digital clock must survive the upgrade");
        assert_eq!(upgraded.widget_type_id, DIGITAL_CLOCK_UID);
        assert_eq!(upgraded.params["showSeconds"], false);
        assert_eq!(upgraded.params["showTimezone"], true);
        assert_eq!(upgraded.params["fontStyle"], "bold");
        assert!(
            upgraded.params.get("show_date").is_none(),
            "show_date must be dropped by the deep translator"
        );
    }

    #[test]
    fn analog_clocks_get_reserved_uids_and_keep_params() {
        for (style, expected_uid) in [
            ("analog_round", ANALOG_ROUND_CLOCK_UID),
            ("analog_rect", ANALOG_RECT_CLOCK_UID),
        ] {
            let params = json!({ "clock_style": style, "numbers_font_style": "medium" });
            let w = mk_widget("clock", params.clone());
            let upgraded = upgrade_widget(&w)
                .unwrap_or_else(|| panic!("BUG: analog_{style} must survive the upgrade"));
            assert_eq!(upgraded.widget_type_id, expected_uid);
            // Shallow pass-through: the params blob is preserved unchanged.
            assert_eq!(upgraded.params, params);
        }
    }

    #[test]
    fn unknown_clock_style_drops() {
        let w = mk_widget(
            "clock",
            json!({ "clock_style": "gigantic", "numbers_font_style": "medium" }),
        );
        assert!(upgrade_widget(&w).is_none());
    }

    #[test]
    fn missing_clock_style_defaults_to_digital() {
        let w = mk_widget("clock", json!({ "show_seconds": true }));
        let upgraded =
            upgrade_widget(&w).expect("BUG: missing clock_style must default to digital");
        assert_eq!(upgraded.widget_type_id, DIGITAL_CLOCK_UID);
    }

    #[test]
    fn native_kinds_get_reserved_uids_and_pass_params_through() {
        for (kind, expected) in [
            ("ticker_btc", TICKER_BTC_UID),
            ("block_height", BLOCK_HEIGHT_UID),
            ("braiins_pool", BRAIINS_POOL_UID),
            ("remote_image", REMOTE_IMAGE_UID),
            ("blockchain_data", BLOCKCHAIN_DATA_UID),
            ("halving_countdown", HALVING_COUNTDOWN_UID),
        ] {
            let params = json!({ "some_param": "abc", "another": 42 });
            let w = mk_widget(kind, params.clone());
            let upgraded = upgrade_widget(&w)
                .unwrap_or_else(|| panic!("BUG: {kind} must survive the upgrade"));
            assert_eq!(upgraded.widget_type_id, expected);
            assert_eq!(upgraded.params, params);
        }
    }

    #[test]
    fn unknown_kind_drops() {
        let w = mk_widget("mystery_widget", json!({}));
        assert!(upgrade_widget(&w).is_none());
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
        assert_eq!(upgraded.params, inner);
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
