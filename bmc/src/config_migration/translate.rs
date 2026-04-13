// Copyright (C) 2025  Braiins Systems s.r.o.

//! Translate a parsed legacy config into the new manifest-driven
//! format. Every translator is a pure function so it can be
//! unit-tested without touching disk or the BMC stack.

use serde_json::{Value, json};
use tracing::warn;
use uuid::Uuid;

use super::legacy;

/// Manifest UID of the digital-clock widget.
/// Mirrors `widgets/digital-clock/manifest.json`.
const DIGITAL_CLOCK_UID: Uuid = Uuid::from_u128(0x550e_8400_e29b_41d4_a716_4466_5544_0001);

/// Result of translating a single legacy widget.
#[derive(Debug)]
pub enum MigrationOutcome {
    /// A matching manifest was found and params were mapped.
    Translated { widget_type_id: Uuid, params: Value },
    /// No matching manifest. The original data is preserved under
    /// `params._legacy` on a placeholder widget with `Uuid::nil()`
    /// as the type id.
    Unavailable,
}

/// Aggregate counts returned alongside the translated config.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub was_legacy: bool,
    pub scenes: usize,
    pub translated_widgets: usize,
    pub unavailable_widgets: usize,
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
        // ticker_btc, block_height, and friends have no manifest yet.
        _ => MigrationOutcome::Unavailable,
    }
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
}
