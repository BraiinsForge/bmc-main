// Copyright (C) 2025  Braiins Systems s.r.o.

//! Translate a parsed legacy config into the new manifest-driven
//! format. Every translator is a pure function so it can be
//! unit-tested without touching disk or the BMC stack.

use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use super::legacy;
use crate::config::CONFIG_VERSION;

/// Manifest UID of the digital-clock widget.
/// Mirrors `widgets/digital-clock/manifest.json`.
const DIGITAL_CLOCK_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0001);

/// Result of translating a single legacy widget.
#[derive(Debug)]
pub enum MigrationOutcome {
    /// A matching manifest was found and params were mapped.
    Translated { widget_type_id: Uuid, params: Value },
    /// Legacy `remote_widget` kind — closer to manifest shape than
    /// other legacy widgets, so we preserve its full metadata
    /// (`name`, `description`, `widget_url`, `icon_url`, `params`)
    /// under `params._legacy_remote`. A future WASM remote-widget
    /// host can adopt these placeholders without asking the user to
    /// re-enter URLs.
    LegacyRemote(LegacyRemoteData),
    /// No matching manifest. Kind + original params are preserved
    /// under `params._legacy` on a placeholder with `Uuid::nil()`
    /// as the type id so a future translator can promote them.
    Unavailable,
}

/// Data carried alongside a `MigrationOutcome::LegacyRemote` so the
/// JSON writer can emit the `_legacy_remote` placeholder.
#[derive(Debug)]
pub struct LegacyRemoteData {
    pub name: String,
    pub description: String,
    pub widget_url: String,
    pub icon_url: String,
    pub params: Value,
}

/// Aggregate counts returned alongside the translated config.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub was_legacy: bool,
    pub scenes: usize,
    pub translated_widgets: usize,
    pub legacy_remote_widgets: usize,
    pub unavailable_widgets: usize,
}

impl Report {
    /// Report produced by a no-op migration (file already at
    /// `CONFIG_VERSION`, nothing translated).
    #[must_use]
    pub fn noop() -> Self {
        Self::default()
    }
}

/// Translate the whole legacy config into a JSON value ready to be
/// deserialized into `crate::config::Config`.
///
/// Returns the JSON plus an aggregate report so callers can log
/// counts or fail CI tests when the translation coverage regresses.
pub fn translate_config(legacy: legacy::Config) -> (Value, Report) {
    let mut report = Report {
        was_legacy: true,
        scenes: legacy.scenes.len(),
        ..Report::default()
    };

    let scenes: Vec<Value> = legacy
        .scenes
        .into_iter()
        .map(|scene| translate_scene(scene, &mut report))
        .collect();

    let config = json!({
        "version": CONFIG_VERSION,
        "scenes": scenes,
        "accounts": legacy.accounts,
    });

    (config, report)
}

fn translate_scene(scene: legacy::Scene, report: &mut Report) -> Value {
    let widgets: Vec<Value> = scene
        .widgets
        .into_iter()
        .map(|widget| translate_widget_into_json(widget, report))
        .collect();

    json!({
        "id": scene.id,
        "enabled": scene.enabled,
        "kind": match scene.kind {
            legacy::SceneKind::Fullscreen => "fullscreen",
            legacy::SceneKind::Combined => "combined",
        },
        "widgets": widgets,
    })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "widget.kind and widget.params are moved into the _legacy payload in the \
              Unavailable arm; taking by reference would force a clone"
)]
fn translate_widget_into_json(widget: legacy::Widget, report: &mut Report) -> Value {
    let outcome = translate_widget(&widget);
    let (widget_type_id, params) = match outcome {
        MigrationOutcome::Translated {
            widget_type_id,
            params,
        } => {
            report.translated_widgets += 1;
            (widget_type_id, params)
        }
        MigrationOutcome::LegacyRemote(data) => {
            report.legacy_remote_widgets += 1;
            warn!(
                id = %widget.id,
                name = %data.name,
                url = %data.widget_url,
                "remote widget has no matching manifest; preserving as legacy-remote placeholder",
            );
            let legacy_payload = json!({
                "_legacy_remote": {
                    "name": data.name,
                    "description": data.description,
                    "widget_url": data.widget_url,
                    "icon_url": data.icon_url,
                    "params": data.params,
                }
            });
            (Uuid::nil(), legacy_payload)
        }
        MigrationOutcome::Unavailable => {
            report.unavailable_widgets += 1;
            warn!(
                kind = %widget.kind,
                id = %widget.id,
                "widget has no matching manifest; preserving as placeholder",
            );
            let legacy_payload = json!({
                "_legacy": {
                    "kind": widget.kind,
                    "params": widget.params,
                }
            });
            (Uuid::nil(), legacy_payload)
        }
    };

    json!({
        "id": widget.id,
        "row": widget.row,
        "col": widget.col,
        "size": widget.size,
        "widget_type_id": widget_type_id,
        "params": params,
    })
}

/// Dispatch a single widget to its kind-specific translator. The
/// match arm is the single place new manifests get wired in as they
/// ship.
pub fn translate_widget(widget: &legacy::Widget) -> MigrationOutcome {
    match widget.kind.as_str() {
        "clock" => translate_clock(widget),
        "remote_widget" => translate_remote_widget(widget),
        // ticker_btc, block_height, and friends have no manifest yet.
        _ => MigrationOutcome::Unavailable,
    }
}

/// Translate a legacy `remote_widget` into a `LegacyRemote` placeholder.
///
/// The old proto carried `name`, `description`, `widget_url`, `icon_url`,
/// and free-form `params` — essentially a remote-manifest snapshot. We
/// preserve all of it verbatim so a future WASM-hosted remote widget
/// can adopt the placeholders without user re-entry.
fn translate_remote_widget(widget: &legacy::Widget) -> MigrationOutcome {
    let pick_string = |key: &str| -> String {
        widget
            .params
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };

    let inner_params = widget.params.get("params").cloned().unwrap_or(Value::Null);

    MigrationOutcome::LegacyRemote(LegacyRemoteData {
        name: pick_string("name"),
        description: pick_string("description"),
        widget_url: pick_string("widget_url"),
        icon_url: pick_string("icon_url"),
        params: inner_params,
    })
}

/// Translate a legacy `clock` widget.
///
/// Only `clock_style == "digital"` maps to an existing manifest; the
/// analog styles have no target yet. `show_date` has no equivalent
/// in the new digital-clock manifest and is silently dropped (a
/// warning is logged).
fn translate_clock(widget: &legacy::Widget) -> MigrationOutcome {
    let style = widget
        .params
        .get("clock_style")
        .and_then(Value::as_str)
        .unwrap_or("digital");

    if style != "digital" {
        return MigrationOutcome::Unavailable;
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

    MigrationOutcome::Translated {
        widget_type_id: DIGITAL_CLOCK_UID,
        params: json!({
            "showSeconds": show_seconds,
            "showTimezone": show_timezone,
            "fontStyle": font_style,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_widget(kind: &str, params: Value) -> legacy::Widget {
        legacy::Widget {
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
        let outcome = translate_widget(&w);
        let MigrationOutcome::Translated {
            widget_type_id,
            params,
        } = outcome
        else {
            panic!("expected Translated, got {outcome:?}");
        };
        assert_eq!(widget_type_id, DIGITAL_CLOCK_UID);
        assert_eq!(params["showSeconds"], false);
        assert_eq!(params["showTimezone"], true);
        assert_eq!(params["fontStyle"], "bold");
        assert!(
            params.get("show_date").is_none(),
            "show_date must be dropped"
        );
    }

    #[test]
    fn analog_clock_is_unavailable() {
        let w = mk_widget(
            "clock",
            json!({ "clock_style": "analog_rect", "numbers_font_style": "medium" }),
        );
        assert!(matches!(
            translate_widget(&w),
            MigrationOutcome::Unavailable
        ));
    }

    #[test]
    fn unknown_kind_is_unavailable() {
        let w = mk_widget("ticker_btc", json!({ "time_frame": "day1" }));
        assert!(matches!(
            translate_widget(&w),
            MigrationOutcome::Unavailable
        ));
    }

    #[test]
    fn invalid_font_style_falls_back_to_medium() {
        let w = mk_widget(
            "clock",
            json!({ "clock_style": "digital", "numbers_font_style": "gigantic" }),
        );
        let MigrationOutcome::Translated { params, .. } = translate_widget(&w) else {
            panic!("expected Translated");
        };
        assert_eq!(params["fontStyle"], "medium");
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
        let outcome = translate_widget(&w);
        let MigrationOutcome::LegacyRemote(data) = outcome else {
            panic!("expected LegacyRemote, got {outcome:?}");
        };
        assert_eq!(data.name, "Mempool Fees");
        assert_eq!(data.description, "Current sat/vB");
        assert_eq!(data.widget_url, "https://example.com/mempool.wasm");
        assert_eq!(data.icon_url, "https://example.com/mempool.png");
        assert_eq!(data.params["theme"], "dark");
    }

    #[test]
    fn remote_widget_tolerates_missing_fields() {
        let w = mk_widget("remote_widget", json!({ "name": "Partial" }));
        let MigrationOutcome::LegacyRemote(data) = translate_widget(&w) else {
            panic!("expected LegacyRemote");
        };
        assert_eq!(data.name, "Partial");
        assert_eq!(data.description, "");
        assert_eq!(data.widget_url, "");
        assert_eq!(data.icon_url, "");
        assert!(data.params.is_null());
    }

    #[test]
    fn translate_config_emits_current_version_field() {
        let cfg = legacy::Config {
            scenes: vec![],
            accounts: vec![],
        };
        let (json, report) = translate_config(cfg);
        assert_eq!(json["version"], CONFIG_VERSION);
        assert!(report.was_legacy);
        assert_eq!(report.scenes, 0);
    }
}
