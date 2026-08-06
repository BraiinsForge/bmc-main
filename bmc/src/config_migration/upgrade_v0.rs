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

//! v0 → current schema upgrade.
//!
//! [`upgrade_with_report`] takes a parsed [`v0::Config`] and produces a
//! typed [`crate::config::Config`] directly, along with a [`Report`] of
//! what was translated and dropped.
//!
//! Policy — aligned with review feedback:
//!
//! - **No intermediate format for unsupported widgets.** Every v0
//!   widget either maps to a shipped manifest's `widget_type_id` in
//!   the current schema or is dropped outright with a `warn!`. There
//!   is no `_legacy` / `_legacy_remote` placeholder in the output.
//! - **Each mapped widget targets a real manifest UID.** Native
//!   kinds (`clock`, `block_height`, `halving_countdown`,
//!   `remote_image`, `braiins_pool`) map to the `uid`
//!   declared in their `widgets-wasm/*/manifest.json`.
//!   Legacy remote widgets with a native equivalent (`weather`,
//!   `nameday`, `iss-position`, `random-facts`, `spacex-launch`)
//!   map to their native manifest UID; every other legacy remote
//!   widget drops until it gains a native counterpart.
//! - **Deep translation where param shape changed.** `clock`,
//!   `block_height`, `remote_image`, `weather`, and `nameday` get
//!   value-level translation (font-weight vocabulary, humantime →
//!   seconds, enum renames, camelCase → snake_case) into their
//!   shipped manifest's param names.
//! - **Unknown v0 kinds or unrecognised remote-widget URLs drop.**
//!   Per review, users migrate all widgets at once; an unmappable
//!   widget is an edge case, not an inter-state we need to preserve.

use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;

use bmc_shared_time::time::Timezone;
use bmc_widget_manifest::{CredentialKey, ParamKey, ParamValue, ViewportShape};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use tracing::warn;
use uuid::Uuid;

use super::{Report, v0};
use crate::config::widget_uuids::{
    BLOCK_HEIGHT_UID, BRAIINS_POOL_UID, CLOCK_UID, HALVING_COUNTDOWN_UID, ISS_POSITION_UID,
    NAMEDAY_UID, RANDOM_FACTS_UID, REMOTE_IMAGE_UID, SPACEX_LAUNCH_UID, TICKER_LIST_UID,
    TICKER_SINGLE_UID, WEATHER_UID,
};
use crate::config::{CONFIG_VERSION, Config, MigratedSettings};
use crate::data::{AccountId, SceneCycling};
use crate::scene::{
    Scene, SceneId, SceneKind, Widget, WidgetId, WidgetPlacement, WidgetPosition, WidgetSize,
};

// --- Braiinsforge remote-widget host ----------------------------------------

/// Prefix that identifies a Braiinsforge-hosted remote widget URL.
/// We don't use the `url` crate here — the match is a plain prefix
/// strip, kept deliberately tight so unrelated URLs drop out.
const BRAIINSFORGE_URL_PREFIX: &str = "https://widgets.braiinsforge.com/";

// --- Manifest-default fallbacks ---------------------------------------------
//
// The boot-load path hands stored params to the widget without injecting
// manifest defaults, so the migration must fill every required param with
// a value the widget accepts. Each constant below re-states the
// `default_value` (or `enum_values`) its widget's own `manifest.json`
// already declares. Cross-crate tests keep these static fallbacks aligned
// with the shipped manifests.

/// Clock `numbers_font_style` fallback — clock manifest `default_value`.
const CLOCK_FONT_DEFAULT: &str = "semi-bold";
/// Block-height `numbers_font_style` fallback — blockheight manifest default.
const BLOCK_HEIGHT_FONT_DEFAULT: &str = "bold";
/// Halving-countdown `numbers_font_style` fallback — manifest default.
const HALVING_COUNTDOWN_FONT_DEFAULT: &str = "bold";
/// Weather `location` fallback — weather manifest `default_value`.
const DEFAULT_WEATHER_LOCATION: &str = "Prague";
/// Weather `time_zone` fallback — weather manifest `default_value`.
const DEFAULT_WEATHER_TIME_ZONE: &str = "location";
/// Image `refresh_seconds` fallback — image manifest `default_value`.
const DEFAULT_REFRESH_SECONDS: i32 = 3600;
/// Image `refresh_seconds` floor — image manifest `min`.
const MIN_REFRESH_SECONDS: i32 = 300;
/// Nameday `country` fallback — nameday manifest `default_value`.
const DEFAULT_NAMEDAY_COUNTRY: &str = "cz";
/// Pool `style` fallback — braiins-pool manifest `default_value`.
const POOL_STYLE_DEFAULT: &str = "overview";
/// Pool `chart_frame` fallback — braiins-pool manifest `default_value`.
const POOL_CHART_FRAME_DEFAULT: &str = "hours_12";
/// Pool `worker_states` fallback — braiins-pool manifest `default_value`.
const POOL_WORKER_STATES_DEFAULT: bool = true;
/// The credential slot the braiins-pool manifest declares.
const POOL_CREDENTIAL_SLOT: &str = "pool";
/// Nameday `country` vocabulary — nameday manifest `enum_values`.
const NAMEDAY_COUNTRIES: &[&str] = &[
    "at", "cz", "de", "dk", "ee", "es", "fi", "fr", "hr", "hu", "it", "lt", "lv", "pl", "se", "sk",
    "us",
];
/// Ticker-single `pair` fallback — ticker-single manifest `default_value`.
const DEFAULT_TICKER_PAIR: &str = "BTC-USD";
/// Ticker `period` fallback — both ticker manifests' `default_value`.
const DEFAULT_TICKER_PERIOD: &str = "7d";
/// Ticker-list `symbol_1`..`symbol_8` fallbacks — ticker-list manifest
/// `default_value`s, in slot order.
const TICKER_LIST_SYMBOL_DEFAULTS: [&str; 8] =
    ["NVDA", "AAPL", "TSLA", "MSTR", "JPM", "META", "SPY", "NFLX"];
/// Exchange-rate `base`/`quote` fallbacks. These have no counterpart in a
/// current manifest — they restate the legacy exchange-rate widget's own
/// meta defaults, so a param-less legacy widget keeps showing the same pair.
const DEFAULT_EXCHANGE_BASE: &str = "EUR";
const DEFAULT_EXCHANGE_QUOTE: &str = "USD";

#[cfg(feature = "manifest-tests")]
pub(crate) fn manifest_test_expectations()
-> crate::manifest_test_support::MigrationManifestExpectations {
    crate::manifest_test_support::MigrationManifestExpectations {
        clock_font: CLOCK_FONT_DEFAULT,
        block_height_font: BLOCK_HEIGHT_FONT_DEFAULT,
        halving_countdown_font: HALVING_COUNTDOWN_FONT_DEFAULT,
        weather_location: DEFAULT_WEATHER_LOCATION,
        weather_time_zone: DEFAULT_WEATHER_TIME_ZONE,
        image_refresh_seconds: DEFAULT_REFRESH_SECONDS,
        nameday_country: DEFAULT_NAMEDAY_COUNTRY,
        nameday_countries: NAMEDAY_COUNTRIES,
        translated_font_styles: ["light", "medium", "bold"].map(|style| {
            translate_font_style(style).expect("BUG: known v0 font style must translate")
        }),
        pool_style: POOL_STYLE_DEFAULT,
        pool_chart_frame: POOL_CHART_FRAME_DEFAULT,
        pool_worker_states: POOL_WORKER_STATES_DEFAULT,
        pool_styles: ["overview", "big_chart"],
        translated_pool_chart_frames: ["hours4", "hours12", "hours24", "days7"].map(|frame| {
            translate_chart_frame(frame).expect("BUG: known v0 chart frame must translate")
        }),
        pool_credential_slot: POOL_CREDENTIAL_SLOT,
        ticker_pair: DEFAULT_TICKER_PAIR,
        ticker_period: DEFAULT_TICKER_PERIOD,
        ticker_list_symbols: TICKER_LIST_SYMBOL_DEFAULTS,
        translated_ticker_periods: ["1h", "24h", "1d", "7d", "30d", "bogus"]
            .map(|period| translate_ticker_period(Some(period))),
        translated_btc_time_frames: [
            "day1", "week1", "week2", "month1", "month3", "month6", "year1", "year2", "year5",
            "all",
        ]
        .map(|time_frame| {
            translate_btc_time_frame(time_frame).expect("BUG: known v0 time frame must translate")
        }),
        ticker_views: ["sparkline", "candlestick"],
    }
}

// --- Upgrade entry point -----------------------------------------------------

/// The v0 hop. Accounts come back raw and untouched —
/// reshaping them is the next hop's transform, sequenced one level up.
pub(super) fn upgrade_with_report(v0: v0::Config) -> (Config, Report, Vec<Value>) {
    let mut report = Report::default();

    // Insert explicitly rather than `collect()` so a duplicate scene
    // id (hand-edited or corrupt config) is dropped with a `warn!`
    // instead of silently overwriting an earlier scene. `upgrade_scene`
    // has already counted the displaced scene and its widgets, so undo
    // those counts to keep the report matching the on-disk result.
    let mut scenes: IndexMap<SceneId, Scene> = IndexMap::new();
    for scene in &v0.scenes {
        let Some(scene) = upgrade_scene(scene, &mut report) else {
            continue;
        };
        if scenes.contains_key(&scene.id) {
            warn!(id = %scene.id, "duplicate scene id; dropping the duplicate scene");
            report.scenes = report.scenes.saturating_sub(1);
            report.translated_widgets = report
                .translated_widgets
                .saturating_sub(scene.widgets.len());
            continue;
        }
        scenes.insert(scene.id, scene);
    }

    // Top-level settings (night mode, alarms, brightness, …) kept
    // their shape across the schema change, so each passes through a
    // lenient re-parse into its current type. A malformed value drops
    // that single field — never the migration.
    let settings = MigratedSettings {
        scene_cycling: migrate_scene_cycling(v0.scene_cycling),
        localization: passthrough_setting("localization", v0.localization),
        data_collection: passthrough_setting("data_collection", v0.data_collection),
        brightness_pct: passthrough_setting("brightness_pct", v0.brightness_pct),
        night_mode: passthrough_setting("night_mode", v0.night_mode),
        sound_volume_pct: passthrough_setting("sound_volume_pct", v0.sound_volume_pct),
        alarms: passthrough_setting("alarms", v0.alarms),
        led_enabled: passthrough_setting("led_enabled", v0.led_enabled),
        boot_sound_enabled: passthrough_setting("boot_sound_enabled", v0.boot_sound_enabled),
        autoupgrade: passthrough_setting("autoupgrade", v0.autoupgrade),
    };
    let current = Config::from_migrated_parts(scenes, settings);
    (current, report, v0.accounts)
}

// This hop lands scenes and settings on the current schema directly;
// a bump past the current version adds an axis it wouldn't apply,
// so revisit when the assert fires.
const _: () = assert!(CONFIG_VERSION == 2);

// --- Per-widget dispatch -----------------------------------------------------

fn upgrade_scene(scene: &v0::Scene, report: &mut Report) -> Option<Scene> {
    let kind = match scene.kind {
        v0::SceneKind::Fullscreen => SceneKind::Fullscreen,
        v0::SceneKind::Combined => SceneKind::Combined,
    };

    // Insert explicitly rather than `collect()` so a duplicate widget
    // id (only reachable from a hand-edited or corrupt config) is
    // dropped with a `warn!` and counted, instead of silently
    // overwriting an earlier widget and leaving the report overstating
    // the on-disk result.
    let mut widgets: IndexMap<WidgetId, Widget> = IndexMap::new();
    for widget in &scene.widgets {
        let Some(w) = upgrade_widget(widget) else {
            report.dropped_widgets += 1;
            continue;
        };
        if widgets.contains_key(&w.id) {
            warn!(id = %w.id, "duplicate widget id within scene; dropping the duplicate");
            report.dropped_widgets += 1;
            continue;
        }
        report.translated_widgets += 1;
        widgets.insert(w.id, w);
    }

    if widgets.is_empty() {
        report.dropped_scenes += 1;
        return None;
    }

    report.scenes += 1;
    Some(Scene {
        id: SceneId::from(scene.id),
        enabled: scene.enabled,
        cycle_duration: scene.cycle_duration,
        kind,
        widgets,
    })
}

/// Map a v0 widget to a current-schema [`Widget`], or drop it.
///
/// A `Some` return always carries a non-nil `widget_type_id`; there
/// is no placeholder bucket. Callers treat `None` as "this widget
/// does not survive the upgrade" and count it accordingly.
fn upgrade_widget(widget: &v0::Widget) -> Option<Widget> {
    let (widget_type_id, params, credential_bindings) = match widget.kind.as_str() {
        "clock" => with_no_bindings(dispatch_clock(widget)),
        "ticker_btc" => with_no_bindings(dispatch_ticker_btc(widget)),
        "block_height" => with_no_bindings(dispatch_block_height(widget)),
        "halving_countdown" => with_no_bindings(dispatch_halving_countdown()),
        "remote_image" => with_no_bindings(dispatch_remote_image(widget)),
        "remote_widget" => with_no_bindings(dispatch_remote_widget(widget)?),
        "braiins_pool" => dispatch_braiins_pool(widget),
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
        credential_bindings,
    })
}

fn with_no_bindings(
    (uid, params): (Uuid, Value),
) -> (Uuid, Value, BTreeMap<CredentialKey, AccountId>) {
    (uid, params, BTreeMap::new())
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
/// migration, not a rename. Every *required* manifest param is filled:
/// a present, valid v0 value passes through; anything absent or
/// malformed falls back to that param's manifest default. This is
/// deliberate — the boot-load path hands stored params to the widget
/// without injecting manifest defaults, so a required key left unset
/// would panic the widget when it reads it. Keys the manifest does not
/// declare are dropped. Two transformations: `numbers_font_style`'s
/// vocabulary changed from `light`/`medium`/`bold` to
/// `regular`/`semi-bold`/`bold`, and the optional v0 `timezone`
/// (IANA string) was renamed to `timezone_override`.
fn dispatch_clock(widget: &v0::Widget) -> (Uuid, Value) {
    let mut params = Map::new();

    // `clock_style` shares its vocabulary across v0 and the current
    // manifest, so a known value passes through unchanged. An absent
    // or out-of-enum value lands on the manifest default `digital` —
    // never pass a value the manifest enum does not accept, or the
    // widget's typed read would panic.
    let clock_style = widget
        .params
        .get("clock_style")
        .and_then(Value::as_str)
        .filter(|s| matches!(*s, "digital" | "analog_round" | "analog_rect"))
        .unwrap_or("digital");
    params.insert("clock_style".to_owned(), json!(clock_style));

    // Booleans carry identical meaning on both sides; copy the ones
    // that are present and actually boolean-typed. A missing or
    // malformed value falls back to the manifest default `true`.
    for key in ["show_date", "show_seconds", "show_timezone"] {
        let flag = widget
            .params
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(true);
        params.insert(key.to_owned(), json!(flag));
    }

    // Font-weight vocabulary changed, so a raw pass-through could emit
    // an enum value the manifest no longer accepts. Always remap,
    // falling back to the clock manifest's own default weight.
    let font_style = migrate_font_style(widget, CLOCK_FONT_DEFAULT);
    params.insert("numbers_font_style".to_owned(), json!(font_style));

    // v0's optional `timezone` (IANA string) became the manifest's
    // optional `timezone_override`. Optional means it may stay unset,
    // so it is only carried over when the value is a timezone the
    // current firmware recognises — an unknown string drops rather
    // than migrating blind.
    if let Some(tz) = widget
        .params
        .get("timezone")
        .and_then(Value::as_str)
        .filter(|s| Timezone::lookup(s).is_some())
    {
        params.insert("timezone_override".to_owned(), json!(tz));
    }

    (CLOCK_UID, Value::Object(params))
}

/// Translate a legacy `block_height` widget into the current Block
/// Height widget ([`BLOCK_HEIGHT_UID`]). Like the clock, the shipped
/// manifest (`widgets-wasm/blockheight/manifest.json`) keeps v0's
/// param names. Both required params are always set — the boot-load
/// path injects no manifest defaults: `show_timestamp` passes through
/// when present and boolean-typed, else the manifest default `true`;
/// `numbers_font_style` is remapped to the new vocabulary, defaulting
/// to `bold`.
fn dispatch_block_height(widget: &v0::Widget) -> (Uuid, Value) {
    let mut params = Map::new();

    let show_timestamp = widget
        .params
        .get("show_timestamp")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    params.insert("show_timestamp".to_owned(), json!(show_timestamp));

    // The block-height manifest defaults this weight to `bold`, unlike
    // the clock's `semi-bold`; only the vocabulary remap is shared.
    let font_style = migrate_font_style(widget, BLOCK_HEIGHT_FONT_DEFAULT);
    params.insert("numbers_font_style".to_owned(), json!(font_style));

    (BLOCK_HEIGHT_UID, Value::Object(params))
}

/// v0 `halving_countdown` had no supported settings.
/// Hand-edited legacy params are therefore discarded.
fn dispatch_halving_countdown() -> (Uuid, Value) {
    (
        HALVING_COUNTDOWN_UID,
        json!({ "numbers_font_style": HALVING_COUNTDOWN_FONT_DEFAULT }),
    )
}

/// v0's `braiins_pool` widget maps onto the WASM one ([`BRAIINS_POOL_UID`]).
/// `pool_style` and the current `style` share a vocabulary, so a known value
/// passes through; the chart windows are respelled (`hours24` → `hours_24`).
/// v0 never persisted `worker_states`, so it takes the manifest default.
///
/// The legacy `account_id` becomes the `pool` slot's credential binding.
/// Accounts migrate in the next hop keeping their ids, so the binding lands
/// on the same account the operator had chosen; a widget that names none
/// migrates unbound and renders its bind prompt.
fn dispatch_braiins_pool(widget: &v0::Widget) -> (Uuid, Value, BTreeMap<CredentialKey, AccountId>) {
    let mut params = Map::new();

    let style = widget
        .params
        .get("pool_style")
        .and_then(Value::as_str)
        .filter(|s| matches!(*s, "overview" | "big_chart"))
        .unwrap_or(POOL_STYLE_DEFAULT);
    params.insert("style".to_owned(), json!(style));

    let chart_frame = widget
        .params
        .get("chart_frame")
        .and_then(Value::as_str)
        .and_then(translate_chart_frame)
        .unwrap_or(POOL_CHART_FRAME_DEFAULT);
    params.insert("chart_frame".to_owned(), json!(chart_frame));

    params.insert(
        "worker_states".to_owned(),
        json!(POOL_WORKER_STATES_DEFAULT),
    );

    let mut bindings = BTreeMap::new();
    if let Some((slot, account)) = pool_account(widget) {
        bindings.insert(slot, account);
    } else {
        warn!(
            id = %widget.id,
            "legacy pool widget names no usable account; migrating it unbound"
        );
    }

    (BRAIINS_POOL_UID, Value::Object(params), bindings)
}

/// The `pool` slot and the account a legacy pool widget names, if it names
/// one this schema can hold.
fn pool_account(widget: &v0::Widget) -> Option<(CredentialKey, AccountId)> {
    let account = widget.params.get("account_id").and_then(Value::as_str)?;
    let account = AccountId::from_str(account).ok()?;
    let slot = CredentialKey::try_new(POOL_CREDENTIAL_SLOT.to_owned())
        .expect("BUG: the pool slot key is a fixed literal the manifest declares");
    Some((slot, account))
}

/// Map a v0 chart window onto the current manifest's spelling. `None` for a
/// window outside the v0 set, so the caller substitutes the manifest default.
fn translate_chart_frame(frame: &str) -> Option<&'static str> {
    match frame {
        "hours4" => Some("hours_4"),
        "hours12" => Some("hours_12"),
        "hours24" => Some("hours_24"),
        "days7" => Some("days_7"),
        _ => None,
    }
}

/// Map a v0 native `ticker_btc` widget to the current Ticker (Single)
/// widget ([`TICKER_SINGLE_UID`]). The legacy widget always charted
/// Bitcoin as a line graph, so the symbol uses the manifest default and
/// the view is `sparkline`; only the time frame carries over.
fn dispatch_ticker_btc(widget: &v0::Widget) -> (Uuid, Value) {
    let period = widget
        .params
        .get("time_frame")
        .and_then(Value::as_str)
        .and_then(translate_btc_time_frame)
        .unwrap_or(DEFAULT_TICKER_PERIOD);
    (
        TICKER_SINGLE_UID,
        json!({
            "pair": DEFAULT_TICKER_PAIR,
            "period": period,
            "view": "sparkline",
        }),
    )
}

/// Map a v0 `time_frame` token to the ticker manifests' period enum.
/// `None` for a token outside the v0 set, so the caller can fall back
/// to the manifest default.
fn translate_btc_time_frame(time_frame: &str) -> Option<&'static str> {
    match time_frame {
        "day1" => Some("1d"),
        "week1" => Some("7d"),
        "week2" => Some("14d"),
        "month1" => Some("1mo"),
        "month3" => Some("3mo"),
        "month6" => Some("6mo"),
        "year1" => Some("1Y"),
        "year2" => Some("2Y"),
        "year5" => Some("5Y"),
        "all" => Some("full"),
        _ => None,
    }
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
/// `url` keeps its name and passes through. Every required param is
/// always set — the boot-load path injects no manifest defaults — so
/// absent, wrong-typed, or unparseable params fall back to the
/// manifest defaults here.
fn dispatch_remote_image(widget: &v0::Widget) -> (Uuid, Value) {
    let mut params = Map::new();

    // `url` is a free-form string; the manifest default is empty. Fill
    // it when absent so the required param is always present.
    let url = widget
        .params
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("");
    params.insert("url".to_owned(), json!(url));

    // v0 stored the refresh interval as a humantime string; the
    // current manifest wants integer seconds. Reparse through the same
    // `humantime_serde` machinery the rest of the config uses. The
    // manifest types `refresh_seconds` as an integer (i32); a humantime
    // value above i32::MAX seconds would widen to a JSON double and the
    // widget's required-i32 read would then panic on boot, so clamp into
    // the manifest's representable range. Absent or unparseable input
    // falls back to the manifest default.
    //
    // v0 had no lower bound, so a carried-over interval is raised to the floor.
    // Migration is the only point a v0 value enters; values written since then
    // are bounded where they are set.
    let refresh_seconds = widget
        .params
        .get("refresh_duration")
        .and_then(|v| humantime_serde::deserialize::<Duration, _>(v).ok())
        .map_or(DEFAULT_REFRESH_SECONDS, |d| {
            i32::try_from(d.as_secs()).unwrap_or(i32::MAX)
        })
        .max(MIN_REFRESH_SECONDS);
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

/// Translate the inner `params` of a legacy `weather` remote widget
/// into the current native Weather widget ([`WEATHER_UID`],
/// `widgets-wasm/weather/manifest.json`).
///
/// v0 stored only a `location` string. The current manifest requires
/// both `location` and `time_zone`; since the widget applies no
/// defaults of its own on the boot-load path, this fills both. An
/// absent, non-string, or empty `location` falls back to the manifest
/// default `Prague`, and `time_zone` — which v0 had no concept of —
/// is pinned to the manifest default `location`.
fn dispatch_weather_widget_params(params: &Value) -> Value {
    let location = params
        .get("location")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_WEATHER_LOCATION);

    json!({
        "location": location,
        "time_zone": DEFAULT_WEATHER_TIME_ZONE
    })
}

/// Translate the inner `params` of a legacy `nameday` remote widget
/// into the current native Nameday widget ([`NAMEDAY_UID`],
/// `widgets-wasm/nameday/manifest.json`).
///
/// v0 stored `country` (same two-letter vocabulary as the current
/// manifest's enum) and camelCase `showDate`, which the manifest
/// renamed to `show_date`. Both current params are required and the
/// boot-load path injects no manifest defaults, so this fills both:
/// a `country` outside the manifest enum — the widget's typed read
/// would panic on it — and an absent or wrong-typed value land on the
/// manifest default `cz`; `showDate` defaults to `true`.
fn dispatch_nameday_widget_params(params: &Value) -> Value {
    let country = params
        .get("country")
        .and_then(Value::as_str)
        .filter(|c| NAMEDAY_COUNTRIES.contains(c))
        .unwrap_or(DEFAULT_NAMEDAY_COUNTRY);

    let show_date = params
        .get("showDate")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    json!({
        "country": country,
        "show_date": show_date
    })
}

fn translate_ticker_period(period: Option<&str>) -> &'static str {
    match period {
        Some("1h") => "1h",
        Some("24h" | "1d") => "1d",
        Some("7d") => "7d",
        Some("30d") => "1mo",
        Some(_) | None => DEFAULT_TICKER_PERIOD,
    }
}

/// Translate the inner `params` of a legacy `exchange-rate` remote widget
/// into the current Ticker (Single) widget ([`TICKER_SINGLE_UID`]).
///
/// `base`/`quote` collapse into the widget's `pair` symbol (`EUR-USD`),
/// which it maps back onto the same `prices/<window>/<candle>/EUR/USD`
/// resource the legacy widget fetched. The legacy line chart carries over
/// as the `sparkline` view.
fn dispatch_exchange_rate_params(params: &Value) -> Value {
    let base = params
        .get("base")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_EXCHANGE_BASE);
    let quote = params
        .get("quote")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_EXCHANGE_QUOTE);
    json!({
        "pair": format!("{base}-{quote}"),
        "period": translate_ticker_period(params.get("period").and_then(Value::as_str)),
        "view": "sparkline",
    })
}

/// Translate the inner `params` of a legacy `ticker-single-sparkline` or
/// `ticker-single-candlestick` remote widget into the current Ticker
/// (Single) widget ([`TICKER_SINGLE_UID`]). The two legacy widgets differ
/// only in chart style, which the merged widget folds behind its `view`
/// param — the slug picks the value. `pair` keeps its name and passes
/// through; an absent or empty one falls back to the manifest default.
fn dispatch_ticker_single_params(params: &Value, view: &'static str) -> Value {
    let pair = params
        .get("pair")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_TICKER_PAIR);
    json!({
        "pair": pair,
        "period": translate_ticker_period(params.get("period").and_then(Value::as_str)),
        "view": view,
    })
}

/// Translate the inner `params` of a legacy `ticker-list` remote widget
/// into the current Financial Ticker List widget ([`TICKER_LIST_UID`]).
///
/// v0 stored the rows as a JSON `symbols` array; the current manifest has
/// no array type, so each row is its own `symbol_N` param. Usable entries
/// (strings with non-blank content) fill the eight slots in order, and the
/// remaining optional slots stay absent, which the widget reads as skipped
/// rows, not defaulted ones. A `symbols` that is
/// absent, not an array, or yields no usable entry falls back to the
/// manifest defaults, the same stance every other dispatch takes.
fn dispatch_ticker_list_params(params: &Value) -> Value {
    let configured: Vec<&str> = params
        .get("symbols")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(TICKER_LIST_SYMBOL_DEFAULTS.len())
                .collect()
        })
        .unwrap_or_default();
    let slots: &[&str] = if configured.is_empty() {
        &TICKER_LIST_SYMBOL_DEFAULTS
    } else {
        &configured
    };

    let mut out = Map::new();
    for index in 0..TICKER_LIST_SYMBOL_DEFAULTS.len() {
        let value = slots.get(index).copied().map_or(Value::Null, Value::from);
        out.insert(format!("symbol_{}", index + 1), value);
    }
    out.insert(
        "period".to_owned(),
        json!(translate_ticker_period(
            params.get("period").and_then(Value::as_str)
        )),
    );
    Value::Object(out)
}

/// Map a legacy `remote_widget` to a native widget via its
/// `widget_url` slug. Slugs with a native equivalent map to that
/// widget's manifest UID with their inner `params` translated into
/// the manifest's schema; `iss-position`, `random-facts`, and
/// `spacex-launch` take no params in their current manifests, so any
/// legacy params (e.g. spacex `showSeconds`) drop with the mapping.
/// URLs outside the Braiinsforge host, and slugs without a native
/// counterpart, are dropped — the now-redundant metadata (`name`,
/// `description`, `widget_url`, `icon_url`) goes with them because
/// the UID itself encodes widget identity.
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

    // Inner `params` — what the legacy remote widget actually ran
    // with — is translated into the native widget's manifest schema.
    let inner_params = widget.params.get("params").cloned().unwrap_or(Value::Null);

    match slug {
        "weather" => Some((WEATHER_UID, dispatch_weather_widget_params(&inner_params))),
        "nameday" => Some((NAMEDAY_UID, dispatch_nameday_widget_params(&inner_params))),
        "iss-position" => Some((ISS_POSITION_UID, json!({}))),
        "random-facts" => Some((RANDOM_FACTS_UID, json!({}))),
        "spacex-launch" => Some((SPACEX_LAUNCH_UID, json!({}))),
        "exchange-rate" => Some((
            TICKER_SINGLE_UID,
            dispatch_exchange_rate_params(&inner_params),
        )),
        "ticker-single-sparkline" => Some((
            TICKER_SINGLE_UID,
            dispatch_ticker_single_params(&inner_params, "sparkline"),
        )),
        "ticker-single-candlestick" => Some((
            TICKER_SINGLE_UID,
            dispatch_ticker_single_params(&inner_params, "candlestick"),
        )),
        "ticker-list" => Some((TICKER_LIST_UID, dispatch_ticker_list_params(&inner_params))),
        _ => {
            warn!(
                id = %widget.id,
                url = %url,
                slug = %slug,
                "remote_widget slug has no native equivalent; dropping"
            );
            None
        }
    }
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

fn parse_size(size: &str) -> WidgetSize {
    // Reuse `WidgetSize`'s own serde vocabulary rather than re-listing
    // the size strings here; an unknown size falls back to full.
    serde_json::from_value(Value::String(size.to_owned())).unwrap_or_else(|_| {
        warn!(
            size = %size,
            "legacy widget carried an unknown size; defaulting to full"
        );
        WidgetSize::Full
    })
}

// --- Settings migration ------------------------------------------------------

/// Pass `scene_cycling` through with `transition` pinned to Slide.
/// Every v0 release persisted the field without exposing it in the UI
/// (25.11-26.02 even serialized a Fade default), so the stored value
/// is not a user choice and must not survive the upgrade.
fn migrate_scene_cycling(raw: Option<Value>) -> Option<SceneCycling> {
    let mut raw = raw?;
    if let Some(section) = raw.as_object_mut() {
        section.insert("transition".to_owned(), Value::from("slide"));
    }
    passthrough_setting("scene_cycling", Some(raw))
}

/// Re-parse one top-level v0 setting into its current typed form.
/// The shape is identical on both sides so this is a validate step,
/// not a transformation; a malformed value is logged and dropped
/// rather than failing the whole migration. An explicit `null`
/// counts as absent.
fn passthrough_setting<T: serde::de::DeserializeOwned>(
    field: &'static str,
    raw: Option<Value>,
) -> Option<T> {
    let raw = raw?;
    if raw.is_null() {
        return None;
    }
    match serde_json::from_value(raw) {
        Ok(value) => Some(value),
        Err(err) => {
            warn!(
                field,
                error = %err,
                "legacy setting dropped: failed to parse into current schema"
            );
            None
        }
    }
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

    /// Build the current typed param map from `(key, value)` pairs
    /// so pass-through expectations can be stated without leaning
    /// on [`params_from_value`] (which is what we're checking against).
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
        // The five required manifest params are always set; the
        // optional `timezone_override` is not, and `legacy_junk` drops.
        assert_eq!(upgraded.params.len(), 5);
    }

    #[test]
    fn clock_fills_required_defaults_when_absent() {
        // The boot-load path injects no manifest defaults, so an empty
        // v0 clock must still carry every required param with its
        // manifest default and no optional param.
        let upgraded = upgrade("clock", json!({}));
        assert_eq!(upgraded.params["clock_style"], str_param("digital"));
        assert_eq!(upgraded.params["show_date"], ParamValue::Boolean(true));
        assert_eq!(upgraded.params["show_seconds"], ParamValue::Boolean(true));
        assert_eq!(upgraded.params["show_timezone"], ParamValue::Boolean(true));
        assert_eq!(
            upgraded.params["numbers_font_style"],
            str_param("semi-bold")
        );
        assert!(!upgraded.params.contains_key("timezone_override"));
    }

    #[test]
    fn clock_invalid_style_defaults_to_digital() {
        // An out-of-enum style must not reach the widget verbatim — it
        // would panic the typed read — so it falls back to `digital`.
        let upgraded = upgrade("clock", json!({ "clock_style": "sundial" }));
        assert_eq!(upgraded.params["clock_style"], str_param("digital"));
    }

    #[test]
    fn clock_renames_timezone_to_timezone_override() {
        let upgraded = upgrade("clock", json!({ "timezone": "Europe/Prague" }));
        assert_eq!(
            upgraded.params["timezone_override"],
            str_param("Europe/Prague")
        );
    }

    #[test]
    fn clock_unknown_timezone_is_dropped_not_migrated() {
        // `timezone_override` is optional, so a v0 value outside the
        // current firmware's timezone list drops instead of migrating
        // a string the widget cannot resolve.
        let upgraded = upgrade("clock", json!({ "timezone": "Mars/Olympus_Mons" }));
        assert!(!upgraded.params.contains_key("timezone_override"));
    }

    // --- block height --------------------------------------------------------

    #[test]
    fn block_height_maps_to_uid_and_defaults_font_to_bold() {
        let upgraded = upgrade("block_height", json!({}));
        assert_eq!(upgraded.widget_type_id, BLOCK_HEIGHT_UID);
        assert_eq!(upgraded.params["numbers_font_style"], str_param("bold"));
        // `show_timestamp` is required; an empty v0 widget defaults it.
        assert_eq!(upgraded.params["show_timestamp"], ParamValue::Boolean(true));
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

    // --- halving countdown --------------------------------------------------

    #[test]
    fn halving_countdown_ignores_legacy_params_and_fills_required_font_default() {
        let upgraded = upgrade(
            "halving_countdown",
            json!({ "numbers_font_style": "light", "junk": 1 }),
        );
        assert_eq!(upgraded.widget_type_id, HALVING_COUNTDOWN_UID);
        assert_eq!(
            upgraded.params,
            param_map(&[(
                "numbers_font_style",
                str_param(HALVING_COUNTDOWN_FONT_DEFAULT),
            )])
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
        for (human, secs) in [("5m", 300), ("1h", 3600), ("2h30m", 9000)] {
            let upgraded = upgrade("remote_image", json!({ "refresh_duration": human }));
            assert_eq!(
                upgraded.params["refresh_seconds"],
                ParamValue::Integer(secs),
                "`{human}` must become {secs} seconds"
            );
        }
    }

    #[test]
    fn remote_image_refresh_under_the_floor_is_raised_to_it() {
        let upgraded = upgrade("remote_image", json!({ "refresh_duration": "30s" }));
        assert_eq!(
            upgraded.params["refresh_seconds"],
            ParamValue::Integer(300),
            "a v0 interval under the manifest floor must be raised, not carried over"
        );
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
    fn remote_image_huge_refresh_clamps_to_i32_max_and_stays_integer() {
        // A duration above i32::MAX seconds must not widen to a double
        // (which would panic the widget's required-i32 read on boot);
        // it clamps to the manifest's integer range instead.
        let upgraded = upgrade("remote_image", json!({ "refresh_duration": "100years" }));
        assert_eq!(
            upgraded.params["refresh_seconds"],
            ParamValue::Integer(i32::MAX),
            "an out-of-range refresh must clamp to i32::MAX, not become a double"
        );
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
    fn remote_image_defaults_url_when_absent() {
        // `url` is required; an empty v0 widget must still carry it as
        // the manifest default (empty string) rather than omitting it.
        let upgraded = upgrade("remote_image", json!({}));
        assert_eq!(upgraded.params["url"], str_param(""));
    }

    #[test]
    fn ticker_btc_maps_to_the_default_btc_pair_as_a_sparkline() {
        let upgraded = upgrade("ticker_btc", json!({ "time_frame": "month1" }));
        assert_eq!(upgraded.widget_type_id, TICKER_SINGLE_UID);
        assert_eq!(
            upgraded.params,
            param_map(&[
                ("pair", str_param(DEFAULT_TICKER_PAIR)),
                ("period", str_param("1mo")),
                ("view", str_param("sparkline")),
            ])
        );
    }

    #[test]
    fn ticker_btc_defaults_the_period_for_unknown_time_frames() {
        for inner in [json!(null), json!({ "time_frame": "fortnight" })] {
            let upgraded = upgrade("ticker_btc", inner);
            assert_eq!(upgraded.params["period"], str_param("7d"));
        }
    }

    #[test]
    fn unknown_kind_drops() {
        let w = mk_widget("mystery_widget", json!({}));
        assert!(upgrade_widget(&w).is_none());
    }

    // --- braiins pool --------------------------------------------------------

    #[test]
    fn pool_binds_the_account_it_named() {
        let account = "9fbe91d6-c391-4598-a8c2-b0e54eb5290c";
        let upgraded = upgrade("braiins_pool", json!({ "account_id": account }));
        let slot = CredentialKey::try_new(POOL_CREDENTIAL_SLOT.to_owned())
            .expect("BUG: slot key is valid");
        assert_eq!(
            upgraded
                .credential_bindings
                .get(&slot)
                .map(ToString::to_string),
            Some(account.to_owned()),
        );
    }

    /// A missing or unparseable `account_id` leaves the widget unbound
    /// rather than dropping it; the operator rebinds from its own prompt.
    #[test]
    fn pool_without_a_usable_account_migrates_unbound() {
        for params in [json!({}), json!({ "account_id": "" })] {
            let upgraded = upgrade("braiins_pool", params.clone());
            assert!(
                upgraded.credential_bindings.is_empty(),
                "{params} must not produce a binding",
            );
        }
    }

    #[test]
    fn pool_respells_the_chart_window_and_defaults_an_unknown_one() {
        for (v0_frame, expected) in [
            ("hours4", "hours_4"),
            ("hours12", "hours_12"),
            ("hours24", "hours_24"),
            ("days7", "days_7"),
        ] {
            let upgraded = upgrade("braiins_pool", json!({ "chart_frame": v0_frame }));
            assert_eq!(
                upgraded.params["chart_frame"],
                str_param(expected),
                "v0 `{v0_frame}` must map to `{expected}`",
            );
        }

        let upgraded = upgrade("braiins_pool", json!({ "chart_frame": "fortnight" }));
        assert_eq!(
            upgraded.params["chart_frame"],
            str_param(POOL_CHART_FRAME_DEFAULT),
            "a window outside the v0 set must land on the manifest default",
        );
    }

    #[test]
    fn pool_passes_a_known_style_and_defaults_the_rest() {
        for style in ["overview", "big_chart"] {
            let upgraded = upgrade("braiins_pool", json!({ "pool_style": style }));
            assert_eq!(upgraded.params["style"], str_param(style));
        }

        let upgraded = upgrade("braiins_pool", json!({ "pool_style": "hologram" }));
        assert_eq!(
            upgraded.params["style"],
            str_param(POOL_STYLE_DEFAULT),
            "a style outside the v0 set must land on the manifest default",
        );
    }

    // --- scene dropping ------------------------------------------------------

    fn scene_with(widgets: Vec<v0::Widget>) -> v0::Scene {
        v0::Scene {
            id: Uuid::nil(),
            enabled: true,
            cycle_duration: None,
            kind: v0::SceneKind::Fullscreen,
            widgets,
        }
    }

    #[test]
    fn scene_with_only_unmappable_widgets_is_dropped() {
        let scene = scene_with(vec![mk_widget("mystery_widget", json!({}))]);
        let (_current, report, _) = upgrade_with_report(v0::Config {
            scenes: vec![scene],
            ..Default::default()
        });
        assert_eq!(report.scenes, 0, "the empty scene must not be kept");
        assert_eq!(report.dropped_scenes, 1);
        assert_eq!(report.dropped_widgets, 1);
    }

    #[test]
    fn scene_with_a_survivor_is_kept() {
        let scene = scene_with(vec![mk_widget("clock", json!({}))]);
        let (_current, report, _) = upgrade_with_report(v0::Config {
            scenes: vec![scene],
            ..Default::default()
        });
        assert_eq!(report.scenes, 1);
        assert_eq!(report.dropped_scenes, 0);
        assert_eq!(report.translated_widgets, 1);
    }

    // --- scene parameters migration ------------------------------------------
    #[test]
    fn scene_supported_parameters_migrates() {
        let scene_uuid = Uuid::from_u128(67);

        let scene = v0::Scene {
            id: scene_uuid,
            enabled: true,
            cycle_duration: Some(Duration::from_mins(1)),
            kind: v0::SceneKind::Fullscreen,
            widgets: vec![mk_widget("clock", json!({}))],
        };
        let (current, _, _) = upgrade_with_report(v0::Config {
            scenes: vec![scene.clone()],
            ..Default::default()
        });
        let migrated_scene = current
            .scenes()
            .get(&SceneId::from(scene_uuid))
            .expect("BUG: valid scene must be migrated");

        assert_eq!(
            scene.id,
            migrated_scene.id.as_uuid(),
            "Migrated scene 'ID' parameter must match!"
        );
        assert_eq!(
            scene.enabled, migrated_scene.enabled,
            "Migrated scene 'enabled' parameter must match!"
        );
        assert_eq!(
            scene.cycle_duration, migrated_scene.cycle_duration,
            "Migrated scene 'cycle_duration' parameter must match!"
        );
        assert_eq!(
            migrated_scene.kind,
            SceneKind::Fullscreen,
            "Migrated scene 'kind' parameter must match!"
        );

        current
            .validate()
            .expect("BUG: migrated valid scene must validate");
    }

    // --- malformed scenes ----------------------------------------------------

    #[test]
    fn duplicate_scene_id_drops_one_and_keeps_the_report_accurate() {
        // Two scenes sharing an id can only come from a hand-edited or
        // corrupt config. One must drop (not silently overwrite), and
        // the report must still match the on-disk result.
        let id = Uuid::from_u128(0x5ce7e);
        let scene = |kind| v0::Scene {
            id,
            enabled: true,
            cycle_duration: None,
            kind,
            widgets: vec![mk_widget("clock", json!({}))],
        };
        let (current, report, _) = upgrade_with_report(v0::Config {
            scenes: vec![
                scene(v0::SceneKind::Fullscreen),
                scene(v0::SceneKind::Fullscreen),
            ],
            ..Default::default()
        });
        assert_eq!(current.scenes().len(), 1, "the duplicate scene must drop");
        assert_eq!(report.scenes, 1, "report must count only the survivor");
        assert_eq!(
            report.translated_widgets, 1,
            "report must not count the dropped duplicate's widget"
        );
    }

    #[test]
    fn duplicate_widget_id_within_scene_drops_the_duplicate() {
        // Two widgets in one scene sharing an id: the duplicate drops
        // and is counted as dropped, rather than overwriting the first.
        // A combined scene rejects fullscreen-sized widgets, so use
        // a slot size to keep the deduplicated scene valid — the point
        // here is id dedup.
        let mut first = mk_widget("clock", json!({}));
        let mut duplicate = mk_widget("clock", json!({}));
        first.size = "small".into();
        duplicate.size = "small".into();
        let scene = v0::Scene {
            id: Uuid::from_u128(0x5ce7f),
            enabled: true,
            cycle_duration: None,
            kind: v0::SceneKind::Combined,
            widgets: vec![first, duplicate],
        };
        let (current, report, _) = upgrade_with_report(v0::Config {
            scenes: vec![scene],
            ..Default::default()
        });
        current
            .validate()
            .expect("BUG: deduplicated combined scene must validate");
        assert_eq!(report.translated_widgets, 1);
        assert_eq!(report.dropped_widgets, 1);
        assert_eq!(
            current
                .scenes()
                .values()
                .next()
                .expect("BUG: one scene")
                .widgets
                .len(),
            1,
            "only one widget must survive on disk"
        );
    }

    #[test]
    fn remote_widget_weather_maps_to_native_uid_and_carries_location() {
        let w = mk_widget(
            "remote_widget",
            json!({
                "name": "Weather",
                "description": "",
                "widget_url": "https://widgets.braiinsforge.com/weather",
                "icon_url": "",
                "params": { "location": "Helsinki" },
            }),
        );
        let upgraded =
            upgrade_widget(&w).expect("BUG: weather slug must resolve to the native UID");
        assert_eq!(upgraded.widget_type_id, WEATHER_UID);
        // Legacy metadata (name, URLs) drops; `location` carries over and
        // `time_zone` defaults to the manifest's `location` value.
        assert_eq!(
            upgraded.params,
            param_map(&[
                ("location", str_param("Helsinki")),
                ("time_zone", str_param("location")),
            ])
        );
    }

    #[test]
    fn remote_widget_weather_defaults_location_when_absent_or_empty() {
        for inner in [json!({}), json!({ "location": "" })] {
            let w = mk_widget(
                "remote_widget",
                json!({
                    "widget_url": "https://widgets.braiinsforge.com/weather",
                    "params": inner,
                }),
            );
            let upgraded = upgrade_widget(&w).expect("BUG: weather slug must survive the upgrade");
            assert_eq!(upgraded.params["location"], str_param("Prague"));
            assert_eq!(upgraded.params["time_zone"], str_param("location"));
        }
    }

    #[test]
    fn remote_widget_slug_without_native_equivalent_drops() {
        // Braiinsforge-hosted remote widgets without a native
        // counterpart drop until they gain one.
        for slug in ["formula-1", "nasa-picture-of-the-day"] {
            let w = mk_widget(
                "remote_widget",
                json!({
                    "widget_url": format!("https://widgets.braiinsforge.com/{slug}"),
                    "params": {},
                }),
            );
            assert!(
                upgrade_widget(&w).is_none(),
                "slug `{slug}` has no native equivalent and must drop"
            );
        }
    }

    #[test]
    fn remote_widget_nameday_translates_country_and_renames_show_date() {
        let w = mk_widget(
            "remote_widget",
            json!({
                "widget_url": "https://widgets.braiinsforge.com/nameday",
                "params": { "country": "sk", "showDate": false },
            }),
        );
        let upgraded = upgrade_widget(&w).expect("BUG: nameday must survive the upgrade");
        assert_eq!(upgraded.widget_type_id, NAMEDAY_UID);
        // `country` carries over; camelCase `showDate` becomes the
        // manifest's `show_date`.
        assert_eq!(
            upgraded.params,
            param_map(&[
                ("country", str_param("sk")),
                ("show_date", ParamValue::Boolean(false)),
            ])
        );
    }

    #[test]
    fn remote_widget_nameday_defaults_on_null_or_unknown_country() {
        // Real v0 configs carry `params: null` for nameday; an
        // out-of-enum country must not reach the widget verbatim.
        for inner in [json!(null), json!({ "country": "xx" })] {
            let w = mk_widget(
                "remote_widget",
                json!({
                    "widget_url": "https://widgets.braiinsforge.com/nameday",
                    "params": inner,
                }),
            );
            let upgraded = upgrade_widget(&w).expect("BUG: nameday must survive the upgrade");
            assert_eq!(upgraded.params["country"], str_param("cz"));
            assert_eq!(upgraded.params["show_date"], ParamValue::Boolean(true));
        }
    }

    /// Upgrade a Braiinsforge remote widget of `slug` with `inner` params.
    fn upgrade_remote(slug: &str, inner: &Value) -> Widget {
        let w = mk_widget(
            "remote_widget",
            json!({
                "widget_url": format!("https://widgets.braiinsforge.com/{slug}"),
                "params": inner,
            }),
        );
        upgrade_widget(&w).unwrap_or_else(|| panic!("BUG: {slug} must survive the upgrade"))
    }

    #[test]
    fn remote_widget_exchange_rate_collapses_base_quote_into_pair() {
        let upgraded = upgrade_remote(
            "exchange-rate",
            &json!({ "base": "CZK", "quote": "USD", "period": "24h" }),
        );
        assert_eq!(upgraded.widget_type_id, TICKER_SINGLE_UID);
        // `CZK-USD` maps onto the same `prices/<window>/<candle>/CZK/USD`
        // resource the legacy widget fetched; v0 `24h` is the manifests'
        // `1d`; the legacy line chart carries over as `sparkline`.
        assert_eq!(
            upgraded.params,
            param_map(&[
                ("pair", str_param("CZK-USD")),
                ("period", str_param("1d")),
                ("view", str_param("sparkline")),
            ])
        );
    }

    #[test]
    fn remote_widget_exchange_rate_defaults_when_params_are_absent() {
        // EUR/USD restates the legacy widget's own meta defaults; the
        // period lands on the manifest default. Whitespace-only values
        // count as absent, or the pair would render as e.g. `-USD`.
        for inner in [json!(null), json!({ "base": "   ", "quote": " " })] {
            let upgraded = upgrade_remote("exchange-rate", &inner);
            assert_eq!(upgraded.params["pair"], str_param("EUR-USD"));
            assert_eq!(upgraded.params["period"], str_param("7d"));
        }
    }

    #[test]
    fn legacy_ticker_periods_preserve_their_windows() {
        for (legacy, current) in [
            ("1h", "1h"),
            ("24h", "1d"),
            ("1d", "1d"),
            ("7d", "7d"),
            ("30d", "1mo"),
        ] {
            assert_eq!(translate_ticker_period(Some(legacy)), current);
        }
    }

    #[test]
    fn remote_widget_ticker_singles_fold_the_slug_into_the_view_param() {
        for (slug, view) in [
            ("ticker-single-sparkline", "sparkline"),
            ("ticker-single-candlestick", "candlestick"),
        ] {
            let upgraded = upgrade_remote(slug, &json!({ "pair": "ETH-EUR", "period": "30d" }));
            assert_eq!(upgraded.widget_type_id, TICKER_SINGLE_UID);
            assert_eq!(
                upgraded.params,
                param_map(&[
                    ("pair", str_param("ETH-EUR")),
                    ("period", str_param("1mo")),
                    ("view", str_param(view)),
                ]),
                "wrong params for `{slug}`"
            );
        }
    }

    #[test]
    fn remote_widget_ticker_single_defaults_pair_and_unknown_period() {
        for empty_pair in ["", "   "] {
            let upgraded = upgrade_remote(
                "ticker-single-sparkline",
                &json!({ "pair": empty_pair, "period": "45d" }),
            );
            assert_eq!(upgraded.params["pair"], str_param("BTC-USD"));
            assert_eq!(upgraded.params["period"], str_param("7d"));
        }
    }

    #[test]
    fn remote_widget_ticker_list_fills_slots_in_order_and_nulls_the_rest() {
        let upgraded = upgrade_remote(
            "ticker-list",
            &json!({ "symbols": ["AAPL", "  MSFT ", "", 42, "^GSPC"], "period": "1d" }),
        );
        assert_eq!(upgraded.widget_type_id, TICKER_LIST_UID);
        // Usable entries compact into the leading slots; null optional slots
        // do not grow default rows the user never configured.
        assert_eq!(
            upgraded.params,
            param_map(&[
                ("symbol_1", str_param("AAPL")),
                ("symbol_2", str_param("MSFT")),
                ("symbol_3", str_param("^GSPC")),
                ("symbol_4", ParamValue::Null),
                ("symbol_5", ParamValue::Null),
                ("symbol_6", ParamValue::Null),
                ("symbol_7", ParamValue::Null),
                ("symbol_8", ParamValue::Null),
                ("period", str_param("1d")),
            ])
        );
    }

    #[test]
    fn remote_widget_ticker_list_caps_at_eight_slots() {
        let upgraded = upgrade_remote(
            "ticker-list",
            &json!({ "symbols": ["A", "B", "C", "D", "E", "F", "G", "H", "I"] }),
        );
        assert_eq!(upgraded.params["symbol_8"], str_param("H"));
        assert!(!upgraded.params.contains_key("symbol_9"));
    }

    #[test]
    fn remote_widget_ticker_list_defaults_when_symbols_are_unusable() {
        for inner in [
            json!(null),
            json!({ "symbols": "AAPL" }),
            json!({ "symbols": [""] }),
        ] {
            let upgraded = upgrade_remote("ticker-list", &inner);
            assert_eq!(upgraded.params["symbol_1"], str_param("NVDA"));
            assert_eq!(upgraded.params["symbol_8"], str_param("NFLX"));
            assert_eq!(upgraded.params["period"], str_param("7d"));
        }
    }

    #[test]
    fn remote_widget_paramless_slugs_map_to_native_uids_with_empty_params() {
        // These widgets take no params in their current manifests, so
        // any legacy params (e.g. spacex `showSeconds`) drop.
        for (slug, inner, uid) in [
            ("iss-position", json!({}), ISS_POSITION_UID),
            ("random-facts", json!({}), RANDOM_FACTS_UID),
            (
                "spacex-launch",
                json!({ "showSeconds": true }),
                SPACEX_LAUNCH_UID,
            ),
        ] {
            let w = mk_widget(
                "remote_widget",
                json!({
                    "widget_url": format!("https://widgets.braiinsforge.com/{slug}"),
                    "params": inner,
                }),
            );
            let upgraded = upgrade_widget(&w)
                .unwrap_or_else(|| panic!("BUG: {slug} must survive the upgrade"));
            assert_eq!(upgraded.widget_type_id, uid, "wrong UID for `{slug}`");
            assert!(
                upgraded.params.is_empty(),
                "`{slug}` params must be empty, got {:?}",
                upgraded.params
            );
        }
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
    fn upgraded_config_carries_current_version() {
        let v0 = v0::Config::default();
        let (upgraded, _, _) = upgrade_with_report(v0);
        assert_eq!(upgraded.version, CONFIG_VERSION);
    }

    #[test]
    fn settings_pass_through_to_current_config() {
        let v0: v0::Config = serde_json::from_value(json!({
            "scenes": [],
            "brightness_pct": 30,
            "sound_volume_pct": 45,
            "led_enabled": false,
            "boot_sound_enabled": true,
            "data_collection": false,
            "night_mode": { "enabled": true, "from": "21:00:00", "to": "06:30:00" },
            "alarms": [{
                "id": "wake-up",
                "enabled": true,
                "name": "Wake up",
                "time": "07:00:00",
                "repeat": []
            }]
        }))
        .expect("BUG: v0 settings fixture must parse");
        let (current, _, _) = upgrade_with_report(v0);
        let out = serde_json::to_value(&current).expect("BUG: config must serialize");
        assert_eq!(out["brightness_pct"], 30);
        assert_eq!(out["sound_volume_pct"], 45);
        assert_eq!(out["led_enabled"], false);
        assert_eq!(out["boot_sound_enabled"], true);
        assert_eq!(out["data_collection"], false);
        assert_eq!(out["night_mode"]["from"], "21:00:00");
        assert_eq!(out["alarms"][0]["time"], "07:00:00");
    }

    #[test]
    fn scene_cycling_migration_forces_slide_and_keeps_the_rest() {
        // 25.11-26.02 shape: `transition` carries the serialized Fade default
        // the user never chose => forced to Slide.
        let v0: v0::Config = serde_json::from_value(json!({
            "scenes": [],
            "scene_cycling": {
                "automatic_cycling_enabled": false,
                "automatic_cycling_default_duration": "45s",
                "transition": "fade"
            }
        }))
        .expect("BUG: v0 fixture must parse");
        let (current, _, _) = upgrade_with_report(v0);
        let out = serde_json::to_value(&current).expect("BUG: config must serialize");
        assert_eq!(out["scene_cycling"]["automatic_cycling_enabled"], false);
        assert_eq!(
            out["scene_cycling"]["automatic_cycling_default_duration"],
            "45s"
        );
        assert_eq!(out["scene_cycling"]["transition"], "slide");
    }

    #[test]
    fn malformed_setting_drops_only_that_field() {
        let v0: v0::Config = serde_json::from_value(json!({
            "scenes": [],
            "brightness_pct": 30,
            "night_mode": "not-an-object"
        }))
        .expect("BUG: v0 fixture must parse");
        let (current, _, _) = upgrade_with_report(v0);
        let out = serde_json::to_value(&current).expect("BUG: config must serialize");
        assert_eq!(out["brightness_pct"], 30);
        assert!(
            out.get("night_mode").is_none(),
            "malformed night_mode must be dropped, not fail the migration"
        );
    }

    #[test]
    fn explicit_null_setting_stays_unset() {
        let v0: v0::Config = serde_json::from_value(json!({
            "scenes": [],
            "brightness_pct": null
        }))
        .expect("BUG: v0 fixture must parse");
        let (current, _, _) = upgrade_with_report(v0);
        let out = serde_json::to_value(&current).expect("BUG: config must serialize");
        assert!(out.get("brightness_pct").is_none());
    }
}
