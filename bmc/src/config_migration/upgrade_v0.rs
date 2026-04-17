// Copyright (C) 2025  Braiins Systems s.r.o.

//! v0 → current schema upgrade.
//!
//! Implements [`Upgrade`] for [`v0::Config`], producing a typed
//! [`crate::config::Config`] directly. No intermediate
//! `serde_json::Value` juggling: the result is built with Rust
//! struct literals and only hits serde when the caller persists.
//!
//! Placeholder widgets (`widget_type_id == Uuid::nil()` with a
//! `_legacy` or `_legacy_remote` payload in `params`) preserve the
//! original data so a future firmware can promote them. The
//! placeholder shape is a deliberate on-disk convention; see
//! `docs/stories/config-migration.md`.

use indexmap::IndexMap;
use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use super::{Upgrade, Version, v0};
use crate::config::Config;
use crate::scene::{Scene, SceneId, SceneKind, Widget, WidgetId, WidgetPosition, WidgetSize};

/// Manifest UID of the digital-clock widget.
/// Mirrors `widgets/digital-clock/manifest.json`.
const DIGITAL_CLOCK_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0001);

impl Upgrade for v0::Config {
    type NextVersion = Config;

    fn upgrade_to_next_version(self) -> Config {
        let scenes: IndexMap<SceneId, Scene> = self
            .scenes
            .iter()
            .map(|scene| {
                let converted = upgrade_scene(scene);
                (converted.id, converted)
            })
            .collect();

        // The current [`Config`] has more fields than v0 ever knew
        // about (localization, night mode, alarms, autoupgrade, …).
        // Start from `Default::default()` and overwrite only what
        // we can derive from v0 data. Everything else inherits
        // sensible defaults from the current schema.
        let mut current = Config::default();
        current.scenes = scenes;
        // Accounts are pass-through: the JSON shape matches between
        // versions. Re-deserialize through serde_json once so the
        // current [`Account`] type validates the legacy payload.
        current.accounts = deserialize_accounts_passthrough(self.accounts);
        current
    }
}

/// Sanity assertion — the upgrade chain terminates at the current
/// schema, and the two version numbers are adjacent.
const _: () = {
    const _V0_IS_ZERO: () = assert!(v0::Config::VERSION == 0);
    const _CURRENT_IS_ONE_ABOVE_V0: () = assert!(Config::VERSION == v0::Config::VERSION + 1);
};

fn upgrade_scene(scene: &v0::Scene) -> Scene {
    let widgets: IndexMap<WidgetId, Widget> = scene
        .widgets
        .iter()
        .map(|widget| {
            let upgraded = upgrade_widget(widget);
            (upgraded.id, upgraded)
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

/// Construct a current-schema widget from a v0 widget. Widgets
/// without a current manifest land as placeholders with
/// `widget_type_id == Uuid::nil()` and the original data stashed
/// under a reserved key in `params`.
fn upgrade_widget(widget: &v0::Widget) -> Widget {
    let id = WidgetId::from(widget.id);
    let position = WidgetPosition {
        row: widget.row,
        col: widget.col,
    };
    let size = parse_size(&widget.size);

    let (widget_type_id, params) = match widget.kind.as_str() {
        "clock" => translate_clock(widget),
        "remote_widget" => translate_remote_widget(widget),
        // ticker_btc, block_height, and friends have no manifest yet.
        _ => unavailable_placeholder(widget),
    };

    Widget {
        id,
        position,
        size,
        widget_type_id,
        params,
    }
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

/// `clock` + `clock_style: "digital"` maps to the `digital-clock`
/// manifest. The analog styles have no current target. `show_date`
/// has no equivalent on the new manifest and is silently dropped
/// (a warning is logged).
fn translate_clock(widget: &v0::Widget) -> (Uuid, Value) {
    let style = widget
        .params
        .get("clock_style")
        .and_then(Value::as_str)
        .unwrap_or("digital");

    if style != "digital" {
        return unavailable_placeholder(widget);
    }

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
            "legacy clock had `show_date: true`; dropped (digital-clock manifest has no date param)",
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

/// `remote_widget` preserves the full legacy metadata under
/// `_legacy_remote` so a future WASM remote-widget host can adopt
/// the placeholders without user re-entry.
fn translate_remote_widget(widget: &v0::Widget) -> (Uuid, Value) {
    let pick_string = |key: &str| -> String {
        widget
            .params
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let name = pick_string("name");
    let widget_url = pick_string("widget_url");

    warn!(
        id = %widget.id,
        name = %name,
        url = %widget_url,
        "remote widget has no matching manifest; preserving as legacy-remote placeholder",
    );

    let params = json!({
        "_legacy_remote": {
            "name": name,
            "description": pick_string("description"),
            "widget_url": widget_url,
            "icon_url": pick_string("icon_url"),
            "params": widget.params.get("params").cloned().unwrap_or(Value::Null),
        }
    });
    (Uuid::nil(), params)
}

/// Catch-all placeholder for v0 widgets that don't have a current
/// manifest yet. Keeps `kind` + `params` in the `_legacy` payload.
fn unavailable_placeholder(widget: &v0::Widget) -> (Uuid, Value) {
    warn!(
        kind = %widget.kind,
        id = %widget.id,
        "widget has no matching manifest; preserving as placeholder",
    );
    let params = json!({
        "_legacy": {
            "kind": widget.kind,
            "params": widget.params,
        }
    });
    (Uuid::nil(), params)
}

/// Re-parse the v0 `accounts` array through the current `Account`
/// type. The shape is identical on both sides so this is a validate
/// step, not a transformation; any malformed entry is logged and
/// dropped rather than failing the whole migration.
fn deserialize_accounts_passthrough(
    accounts: Vec<Value>,
) -> indexmap::IndexMap<bmc_display::data::AccountId, bmc_display::data::Account> {
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
                    "legacy account dropped: failed to parse into current schema",
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
    fn digital_clock_maps_full_params() {
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
        let upgraded = upgrade_widget(&w);
        assert_eq!(upgraded.widget_type_id, DIGITAL_CLOCK_UID);
        assert_eq!(upgraded.params["showSeconds"], false);
        assert_eq!(upgraded.params["showTimezone"], true);
        assert_eq!(upgraded.params["fontStyle"], "bold");
        assert!(
            upgraded.params.get("show_date").is_none(),
            "show_date must be dropped"
        );
    }

    #[test]
    fn analog_clock_is_placeholder() {
        let w = mk_widget(
            "clock",
            json!({ "clock_style": "analog_rect", "numbers_font_style": "medium" }),
        );
        let upgraded = upgrade_widget(&w);
        assert_eq!(upgraded.widget_type_id, Uuid::nil());
        assert!(upgraded.params["_legacy"].is_object());
    }

    #[test]
    fn unknown_kind_is_placeholder() {
        let w = mk_widget("ticker_btc", json!({ "time_frame": "day1" }));
        let upgraded = upgrade_widget(&w);
        assert_eq!(upgraded.widget_type_id, Uuid::nil());
        assert_eq!(upgraded.params["_legacy"]["kind"], "ticker_btc");
    }

    #[test]
    fn invalid_font_style_falls_back_to_medium() {
        let w = mk_widget(
            "clock",
            json!({ "clock_style": "digital", "numbers_font_style": "gigantic" }),
        );
        let upgraded = upgrade_widget(&w);
        assert_eq!(upgraded.params["fontStyle"], "medium");
    }

    #[test]
    fn remote_widget_preserves_full_metadata() {
        let w = mk_widget(
            "remote_widget",
            json!({
                "name": "Mempool Fees",
                "description": "Current sat/vB",
                "widget_url": "https://example.com/mempool.wasm",
                "icon_url": "https://example.com/mempool.png",
                "params": { "theme": "dark" },
            }),
        );
        let upgraded = upgrade_widget(&w);
        assert_eq!(upgraded.widget_type_id, Uuid::nil());
        let legacy = &upgraded.params["_legacy_remote"];
        assert_eq!(legacy["name"], "Mempool Fees");
        assert_eq!(legacy["description"], "Current sat/vB");
        assert_eq!(legacy["widget_url"], "https://example.com/mempool.wasm");
        assert_eq!(legacy["icon_url"], "https://example.com/mempool.png");
        assert_eq!(legacy["params"]["theme"], "dark");
    }

    #[test]
    fn remote_widget_tolerates_missing_fields() {
        let w = mk_widget("remote_widget", json!({ "name": "Partial" }));
        let upgraded = upgrade_widget(&w);
        let legacy = &upgraded.params["_legacy_remote"];
        assert_eq!(legacy["name"], "Partial");
        assert_eq!(legacy["description"], "");
        assert_eq!(legacy["widget_url"], "");
        assert_eq!(legacy["icon_url"], "");
        assert!(legacy["params"].is_null());
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
