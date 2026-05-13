// Copyright (C) 2026  Braiins Systems s.r.o.

//! Right-side params sidebar: SidePanel housing, per-row type-appropriate
//! inputs (text / number with optional GIMP-style filled slider / dropdown /
//! checkbox / clear-to-null toggle), and the `apply_params_update` delivery
//! path that drives `WasmWidgetRuntime::deliver_params_update` on every tile
//! plus appends a `ParamDelivery` event when recording is active.

#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "PARAM_PANEL_W u32 → f32 cast on a fixed UI constant"
)]

use bmc_wasm_runtime::unified_fixture::{TimelineEvent, UnifiedEvent};

use super::{PARAM_PANEL_W, TestbedApp};

impl TestbedApp {
    /// Push a new params snapshot to every tile's runtime via `deliver_params_update`,
    /// update the local cache, and (when recording is active) append a `ParamDelivery`
    /// event to the timeline.
    ///
    /// A no-op when the new snapshot matches the cached one.
    fn apply_params_update(
        &mut self,
        new_params: std::collections::BTreeMap<
            bmc_widget_manifest::ParamKey,
            bmc_widget_manifest::ParamValue,
        >,
    ) {
        if new_params == self.params {
            return;
        }
        // Fire `on_params_update` on every tile — operator-driven changes apply to all
        // size variants previewed in the testbed, not just the active recording tile.
        for tile in &mut self.tiles {
            tile.runtime.deliver_params_update(new_params.clone());
        }
        self.params = new_params;

        if let Some(rec) = self.recording_mode.state.as_mut() {
            let at_ms = rec.recording_start.elapsed().as_millis() as u64;
            let json_params: serde_json::Map<String, serde_json::Value> = self
                .params
                .iter()
                .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
                .collect();
            rec.events.push(TimelineEvent {
                at_ms,
                event: UnifiedEvent::ParamDelivery {
                    params: json_params,
                },
            });
        }
    }

    /// Render the param-mutation form as a fixed right-side sidebar.
    ///
    /// Only shown when the manifest declares any params;
    /// The window's outer width is extended by `PARAM_PANEL_W` at startup
    /// to host this panel without compressing the central tile area.
    ///
    /// Each declared key gets a type-appropriate egui input (text / number / dropdown / checkbox / clear-to-null)
    /// honouring manifest constraints (`enum_values`, `min` / `max` / `step`, `optional`).
    ///
    /// Two columns aligned by `egui::Grid` so the labels and controls stack cleanly regardless of key length.
    pub(super) fn paint_params_panel(&mut self, root_ui: &mut egui::Ui) {
        if self.manifest.params.is_empty() {
            return;
        }
        // Take the current snapshot out so we can mutate while the egui closure borrows it,
        // then put it back via `apply_params_update` which detects diffs and propagates.
        let mut working = self.params.clone();
        let manifest_params = self.manifest.params.clone();
        let mut changed = false;
        let style = root_ui.ctx().style();

        egui::SidePanel::right("params_panel")
            .resizable(false)
            .exact_width(PARAM_PANEL_W as f32)
            .frame(egui::Frame::side_top_panel(&style).inner_margin(egui::Margin::same(8)))
            .show_inside(root_ui, |ui| {
                ui.label(
                    egui::RichText::new("Params")
                        .color(egui::Color32::from_gray(160))
                        .strong(),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |scroll| {
                        egui::Grid::new("params_grid")
                            .num_columns(2)
                            .spacing([12.0, 4.0])
                            .min_col_width(0.0)
                            .show(scroll, |grid| {
                                for (key, def) in &manifest_params {
                                    let current = working.entry(key.clone()).or_insert_with(|| {
                                        bmc_widget_manifest::ParamValue::from_param_kind_default(
                                            &def.kind,
                                        )
                                    });
                                    if paint_param_row(grid, key.as_str(), def, current) {
                                        changed = true;
                                    }
                                }
                            });
                    });
            });

        if changed {
            self.apply_params_update(working);
        }
    }
}

// ── Param-mutation inputs ───────────────────────────────────────────

/// Render one row inside the params Grid: monospace key in the left column,
/// type-appropriate input + optional clear-to-null toggle in the right column.
///
/// Returns `true` when the operator changed the value this frame.
///
/// Caller (`paint_params_panel`) wraps this in `egui::Grid::show` so the two columns
/// stay aligned across rows regardless of key length or input width.
fn paint_param_row(
    grid: &mut egui::Ui,
    key: &str,
    def: &bmc_widget_manifest::ParamDefinition,
    value: &mut bmc_widget_manifest::ParamValue,
) -> bool {
    use bmc_widget_manifest::{ParamKind, ParamValue};

    let mut changed = false;

    // Column 1: monospace key label
    grid.label(
        egui::RichText::new(key)
            .font(egui::FontId::monospace(11.0))
            .color(egui::Color32::from_gray(180)),
    );

    // Column 2: optional toggle + typed input on one row
    grid.horizontal(|row| {
        if def.is_optional {
            let is_null = matches!(value, ParamValue::Null);
            // Plain-text labels — `✗` and similar dingbats aren't in egui's bundled font
            // and render as a missing-glyph box.
            let label = if is_null { "(unset)" } else { "clear" };
            if row.small_button(label).clicked() {
                if is_null {
                    *value = ParamValue::from_param_kind_default(&def.kind);
                    // If the default is also Null (optional-without-default), seed with a
                    // type-appropriate zero so the input below has something to edit.
                    if matches!(value, ParamValue::Null) {
                        *value = match &def.kind {
                            ParamKind::String { .. } | ParamKind::Timezone { .. } => {
                                ParamValue::String(String::new())
                            }
                            ParamKind::Integer { .. } => ParamValue::Integer(0),
                            ParamKind::Double { .. } => ParamValue::Double(0.0),
                            ParamKind::Boolean { .. } => ParamValue::Boolean(false),
                        };
                    }
                } else {
                    *value = ParamValue::Null;
                }
                changed = true;
            }
            if matches!(value, ParamValue::Null) {
                // Nothing to render after the (unset) button when the value is cleared.
                return;
            }
        }
        changed |= paint_typed_input(row, key, &def.kind, value);
    });
    grid.end_row();
    changed
}

/// Render an `egui::Slider` so its compound widget (track + value box) fills `cell_w`.
/// `Slider` doesn't honour `add_sized` for its track width — the track size comes from
/// `ui.spacing().slider_width`. We pre-allocate the cell, scope a temporary `slider_width`
/// equal to the cell minus the value-box estimate, then run the slider inside the scoped
/// ui so the operator sees a track that actually fills the column.
///
/// The value-box reserve is derived from `ui.spacing().interact_size.x` (egui's default
/// minimum interactable width for value-like widgets), not a constant.
fn stretched_slider<R>(ui: &mut egui::Ui, cell_w: f32, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let row_h = ui.spacing().interact_size.y;
    let value_box_reserve = ui.spacing().interact_size.x + ui.spacing().item_spacing.x;
    let track_w = (cell_w - value_box_reserve).max(0.0);
    let result = ui.allocate_ui(egui::vec2(cell_w, row_h), |slot| {
        slot.spacing_mut().slider_width = track_w;
        f(slot)
    });
    result.inner
}

/// Inner dispatch: actual editable widget per kind. Caller has already drawn the label
/// and (when applicable) the optional toggle. Each branch wraps its widget in `add_sized`
/// so the column lines up visually with the longest TextEdit-style input.
///
/// Control width comes from `ui.available_width()` — the parent Grid + horizontal layout
/// has already reserved space for the key label and any optional toggle, so what's left is
/// exactly what we want the input to fill. No constant, layout naturally follows sidebar
/// resizes or label changes.
///
/// `too_many_lines` is `expect`ed because the match is one arm per `ParamKind` variant +
/// enum-or-not split; pulling each branch into its own function would obscure the otherwise
/// trivial widget construction at every site.
#[expect(
    clippy::too_many_lines,
    reason = "linear ParamKind dispatch — splitting hurts readability"
)]
fn paint_typed_input(
    ui: &mut egui::Ui,
    key: &str,
    kind: &bmc_widget_manifest::ParamKind,
    value: &mut bmc_widget_manifest::ParamValue,
) -> bool {
    use bmc_widget_manifest::{ParamKind, ParamValue};

    let row_h = ui.spacing().interact_size.y;
    let cell_w = ui.available_width();
    let cell = egui::vec2(cell_w, row_h);
    match (kind, value) {
        (ParamKind::String { enum_values, .. }, ParamValue::String(s))
            if !enum_values.is_empty() =>
        {
            let mut changed = false;
            ui.allocate_ui(cell, |slot| {
                egui::ComboBox::from_id_salt(key)
                    .selected_text(s.clone())
                    .width(cell_w)
                    .show_ui(slot, |menu| {
                        for opt in enum_values {
                            if menu.selectable_label(*s == opt.value, &opt.label).clicked() {
                                s.clone_from(&opt.value);
                                changed = true;
                            }
                        }
                    });
            });
            changed
        }
        (ParamKind::String { .. } | ParamKind::Timezone { .. }, ParamValue::String(s)) => {
            ui.add_sized(cell, egui::TextEdit::singleline(s)).changed()
        }
        (ParamKind::Integer { enum_values, .. }, ParamValue::Integer(n))
            if !enum_values.is_empty() =>
        {
            let mut changed = false;
            let label = enum_values
                .iter()
                .find(|o| o.value == *n)
                .map_or_else(|| n.to_string(), |o| o.label.clone());
            ui.allocate_ui(cell, |slot| {
                egui::ComboBox::from_id_salt(key)
                    .selected_text(label)
                    .width(cell_w)
                    .show_ui(slot, |menu| {
                        for opt in enum_values {
                            if menu.selectable_label(*n == opt.value, &opt.label).clicked() {
                                *n = opt.value;
                                changed = true;
                            }
                        }
                    });
            });
            changed
        }
        (ParamKind::Integer { min, max, step, .. }, ParamValue::Integer(n)) => {
            // Bounded ranges use a `Slider` with `trailing_fill` so the cell shows the
            // value as a progress fill against `min..=max` (the GIMP-style look). Unbounded
            // integers fall back to a `DragValue` since `Slider` requires a finite range.
            if let (Some(lo), Some(hi)) = (min, max) {
                stretched_slider(ui, cell_w, |sl| {
                    sl.add(
                        egui::Slider::new(n, *lo..=*hi)
                            .step_by(step.map_or(1.0, f64::from))
                            .trailing_fill(true),
                    )
                    .changed()
                })
            } else {
                let mut dv = egui::DragValue::new(n).speed(step.map_or(1.0, f64::from));
                if let Some(lo) = min {
                    dv = dv.range(*lo..=i32::MAX);
                } else if let Some(hi) = max {
                    dv = dv.range(i32::MIN..=*hi);
                }
                ui.add_sized(cell, dv).changed()
            }
        }
        (ParamKind::Double { enum_values, .. }, ParamValue::Double(f))
            if !enum_values.is_empty() =>
        {
            let mut changed = false;
            let label = enum_values
                .iter()
                .find(|o| (o.value - *f).abs() < f64::EPSILON)
                .map_or_else(|| format!("{f}"), |o| o.label.clone());
            ui.allocate_ui(cell, |slot| {
                egui::ComboBox::from_id_salt(key)
                    .selected_text(label)
                    .width(cell_w)
                    .show_ui(slot, |menu| {
                        for opt in enum_values {
                            if menu
                                .selectable_label((opt.value - *f).abs() < f64::EPSILON, &opt.label)
                                .clicked()
                            {
                                *f = opt.value;
                                changed = true;
                            }
                        }
                    });
            });
            changed
        }
        (ParamKind::Double { min, max, step, .. }, ParamValue::Double(f)) => {
            // Same dispatch as Integer: bounded ranges get the filled-slider treatment,
            // unbounded fall back to DragValue.
            if let (Some(lo), Some(hi)) = (min, max) {
                stretched_slider(ui, cell_w, |sl| {
                    sl.add(
                        egui::Slider::new(f, *lo..=*hi)
                            .step_by(step.unwrap_or(0.0))
                            .trailing_fill(true),
                    )
                    .changed()
                })
            } else {
                let mut dv = egui::DragValue::new(f).speed(step.unwrap_or(0.1));
                if let Some(lo) = min {
                    dv = dv.range(*lo..=f64::INFINITY);
                } else if let Some(hi) = max {
                    dv = dv.range(f64::NEG_INFINITY..=*hi);
                }
                ui.add_sized(cell, dv).changed()
            }
        }
        // Checkbox stays at its natural icon size — stretching it would make the entire row
        // a giant click target with the box ghosted in the corner.
        (ParamKind::Boolean { .. }, ParamValue::Boolean(b)) => ui.checkbox(b, "").changed(),
        // Type mismatch (value's variant doesn't match kind) — shouldn't happen with a
        // well-formed manifest + default-init path, but render a read-only label rather than
        // crashing if it does.
        _ => {
            ui.label(
                egui::RichText::new("(type mismatch)")
                    .color(egui::Color32::from_rgb(200, 80, 80))
                    .font(egui::FontId::monospace(10.0)),
            );
            false
        }
    }
}
