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

//! Deck-wide system-snapshot section of the right sidebar.
//! Counterpart to [`crate::params_ui`]'s Params section for the
//! [`SystemSnapshot`] channel.
//!
//! Nine controls (timezone text, six enum dropdowns, next-alarm sub-form,
//! night-mode checkbox) mutate a working copy of `TestbedApp::system`; on any change
//! [`TestbedApp::apply_system_update`] pushes the new snapshot to every
//! tile's runtime via [`WasmWidgetRuntime::deliver_system_update`], records
//! a `UnifiedEvent::SystemDelivery` event, and (when auto-capture is on)
//! inserts a debounced auto-`Capture` event so each settled state yields
//! one frame in the baseline.
//!
//! Mirrors the slide-pending-capture-forward debounce from [`crate::params_ui`].

use bmc_wasm_protocol::system::{
    DateFormat, NumberFormat, TemperatureUnit, TimeFormat, UnitSystem, Weekday,
};
use bmc_wasm_runtime::platform_catalog::PLATFORMS;
use bmc_wasm_runtime::unified_fixture::UnifiedEvent;
use bmc_wasm_runtime::{NextAlarm, SystemSnapshot};

use super::TestbedApp;
use super::recording::record_delivery;
use super::ui_helpers::{combo_cell, key_label};
use super::view::{Delivery, ViewCommand};

impl TestbedApp {
    /// Render the platform selector dropdown. Returns the newly chosen
    /// platform id when the operator picks a different one.
    pub(super) fn paint_platform_selector(&self, ui: &mut egui::Ui) -> Option<String> {
        let mut chosen: Option<String> = None;
        egui::Grid::new("platform_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .min_col_width(0.0)
            .show(ui, |grid| {
                grid.add(key_label("platform", 180));
                let cell_w = grid.available_width();
                let current_label = format!(
                    "{} ({})",
                    self.active_platform.id, self.active_platform.label
                );
                combo_cell(grid, "platform", cell_w, current_label, |menu| {
                    let mut changed = false;
                    for p in PLATFORMS {
                        let selected = p.id == self.active_platform.id;
                        let text = format!("{} ({})", p.id, p.label);
                        if menu.selectable_label(selected, text).clicked() && !selected {
                            chosen = Some(p.id.to_owned());
                            changed = true;
                        }
                    }
                    changed
                });
                grid.end_row();
            });
        chosen
    }

    /// Push a new system snapshot to every tile's runtime
    /// via `deliver_system_update`, update the local cache,
    /// and (when recording is active) append a `SystemDelivery` event
    /// to the timeline plus a debounced auto-`Capture`.
    /// See [`super::recording::record_delivery`] for the debounce semantics.
    ///
    /// A no-op when the new snapshot matches the cached one.
    pub(super) fn apply_system_update(&mut self, new_system: SystemSnapshot) {
        if new_system == self.system {
            return;
        }
        for view in &mut self.tiles {
            view.send(ViewCommand::Deliver(Delivery::System(Box::new(
                new_system.clone(),
            ))));
        }
        self.system = new_system;

        if let Some(rec) = self.recording_mode.state.as_mut() {
            let snapshot = self.system.clone();
            record_delivery(rec, || UnifiedEvent::SystemDelivery { system: snapshot });
        }
    }

    /// Render the system-mutation form directly into the provided UI.
    ///
    /// Called from [`Self::paint_right_panel`] underneath the Params grid;
    /// shares the right sidebar's `ScrollArea` rather than owning its own
    /// `SidePanel`. The system channel is host-defined and applies to every
    /// widget, so this section is always shown — there's no manifest-empty
    /// short-circuit.
    ///
    /// Controls map 1:1 to the nine `SystemSnapshot` fields, plus a toggle
    /// that switches `next_alarm` between `None` and `Some { … }`. Returns
    /// `true` when any control mutated `working`.
    pub(super) fn paint_system_section(
        scroll: &mut egui::Ui,
        working: &mut SystemSnapshot,
    ) -> bool {
        let mut changed = false;
        egui::Grid::new("system_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .min_col_width(0.0)
            .show(scroll, |grid| {
                changed |= paint_timezone_row(grid, &mut working.settings.timezone);
                changed |= paint_enum_row(
                    grid,
                    "time_format",
                    &mut working.settings.time_format,
                    TIME_FORMAT_VARIANTS,
                    time_format_label,
                );
                changed |= paint_enum_row(
                    grid,
                    "date_format",
                    &mut working.settings.date_format,
                    DATE_FORMAT_VARIANTS,
                    date_format_label,
                );
                changed |= paint_enum_row(
                    grid,
                    "number_format",
                    &mut working.settings.number_format,
                    NUMBER_FORMAT_VARIANTS,
                    number_format_label,
                );
                changed |= paint_enum_row(
                    grid,
                    "first_day_of_week",
                    &mut working.settings.first_day_of_week,
                    WEEKDAY_VARIANTS,
                    weekday_label,
                );
                changed |= paint_enum_row(
                    grid,
                    "temperature_unit",
                    &mut working.settings.temperature_unit,
                    TEMPERATURE_UNIT_VARIANTS,
                    temperature_unit_label,
                );
                changed |= paint_enum_row(
                    grid,
                    "unit_system",
                    &mut working.settings.unit_system,
                    UNIT_SYSTEM_VARIANTS,
                    unit_system_label,
                );
                changed |= paint_next_alarm_rows(grid, &mut working.next_alarm);
                changed |= paint_night_mode_row(grid, &mut working.night_mode);
            });
        changed
    }
}

/// One grid row.
///
/// ```text
/// key         [ control fills cell_w        ]
/// ─label_resp─^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
/// ```
///
/// `control` receives the cell `Ui`, its width, and the label's `Response` —
/// so it can wire label-click to focus / toggle the adjacent widget.
/// One label + control row with the sidebar's shared geometry.
/// The control takes the full remaining width at one interact-height,
/// so every section's rows line up whatever control they hold.
pub(super) fn row<R>(
    grid: &mut egui::Ui,
    key: &str,
    control: impl FnOnce(&mut egui::Ui, f32, &egui::Response) -> R,
) -> R {
    let label_resp = grid.add(key_label(key, 180));
    let cell_w = grid.available_width();
    let row_h = grid.spacing().interact_size.y;
    let r = grid
        .allocate_ui(egui::vec2(cell_w, row_h), |slot| {
            control(slot, cell_w, &label_resp)
        })
        .inner;
    grid.end_row();
    r
}

fn supported_tz_names() -> impl Iterator<Item = &'static str> {
    bmc_shared_time::time::Timezone::list()
        .iter()
        .map(bmc_shared_time::time::Timezone::iana)
}

/// Searchable IANA timezone picker over [`supported_tz_names`].
fn paint_timezone_row(grid: &mut egui::Ui, value: &mut String) -> bool {
    row(grid, "timezone", |slot, _w, label| {
        let resp = slot.add(
            egui_autocomplete::AutoCompleteTextEdit::new(value, supported_tz_names())
                .max_suggestions(20)
                .highlight_matches(true)
                .popup_on_focus(true),
        );
        if label.clicked() {
            resp.request_focus();
        }
        resp.changed()
    })
}

fn paint_enum_row<T: Copy + PartialEq>(
    grid: &mut egui::Ui,
    key: &str,
    value: &mut T,
    variants: &[T],
    label: fn(T) -> &'static str,
) -> bool {
    row(grid, key, |slot, w, _label| {
        combo_cell(slot, key, w, label(*value), |menu| {
            let mut changed = false;
            for &variant in variants {
                if menu
                    .selectable_label(*value == variant, label(variant))
                    .clicked()
                {
                    *value = variant;
                    changed = true;
                }
            }
            changed
        })
    })
}

/// ```text
/// next_alarm   [x]          ← toggle row
/// name         [ Wake up ]  ← shown only when toggled on
/// fire_at_ms   [ 1747… ]
/// ```
fn paint_next_alarm_rows(grid: &mut egui::Ui, next_alarm: &mut Option<NextAlarm>) -> bool {
    let mut changed = row(grid, "next_alarm", |slot, _w, label| {
        let mut scheduled = next_alarm.is_some();
        let cb_changed = slot.checkbox(&mut scheduled, "").changed();
        let label_clicked = label.clicked();
        if label_clicked {
            scheduled = !scheduled;
        }
        if cb_changed || label_clicked {
            *next_alarm = scheduled.then(|| NextAlarm {
                // Default fire time: roughly +1h from now.
                // The operator edits it via the rows below once the toggle flips on.
                fire_at_utc_ms: chrono::Utc::now().timestamp_millis() + 3_600_000,
                name: "Wake up".to_owned(),
            });
            true
        } else {
            false
        }
    });

    if let Some(alarm) = next_alarm.as_mut() {
        changed |= row(grid, "name", |slot, _w, label| {
            let resp =
                slot.add(egui::TextEdit::singleline(&mut alarm.name).desired_width(f32::INFINITY));
            if label.clicked() {
                resp.request_focus();
            }
            resp.changed()
        });
        changed |= row(grid, "fire_at_utc_ms", |slot, _w, label| {
            let resp = slot.add(egui::DragValue::new(&mut alarm.fire_at_utc_ms).speed(60_000.0));
            if label.clicked() {
                resp.request_focus();
            }
            resp.changed()
        });
    }
    changed
}

fn paint_night_mode_row(grid: &mut egui::Ui, active: &mut bool) -> bool {
    row(grid, "night_mode", |slot, _w, label| {
        let cb_changed = slot.checkbox(active, "").changed();
        let label_clicked = label.clicked();
        if label_clicked {
            *active = !*active;
        }
        cb_changed || label_clicked
    })
}

// ── Enum variant tables + labels ────────────────────────────────────

const TIME_FORMAT_VARIANTS: &[TimeFormat] = &[TimeFormat::Hour12, TimeFormat::Hour24];

fn time_format_label(t: TimeFormat) -> &'static str {
    match t {
        TimeFormat::Hour12 => "Hour12",
        TimeFormat::Hour24 => "Hour24",
    }
}

const DATE_FORMAT_VARIANTS: &[DateFormat] = &[
    DateFormat::DdMmYyyyDot,
    DateFormat::DdMmYyyySlash,
    DateFormat::DMYyyySlash,
    DateFormat::MDYyyySlash,
    DateFormat::DdMmYyyyDash,
    DateFormat::YyyyMDSlash,
    DateFormat::YyyyMmDdDot,
    DateFormat::YyyyMmDdDash,
];

fn date_format_label(d: DateFormat) -> &'static str {
    match d {
        DateFormat::DdMmYyyyDot => "DD.MM.YYYY",
        DateFormat::DdMmYyyySlash => "DD/MM/YYYY",
        DateFormat::DMYyyySlash => "D/M/YYYY",
        DateFormat::MDYyyySlash => "M/D/YYYY",
        DateFormat::DdMmYyyyDash => "DD-MM-YYYY",
        DateFormat::YyyyMDSlash => "YYYY/M/D",
        DateFormat::YyyyMmDdDot => "YYYY.MM.DD",
        DateFormat::YyyyMmDdDash => "YYYY-MM-DD",
    }
}

const NUMBER_FORMAT_VARIANTS: &[NumberFormat] = &[
    NumberFormat::SpaceGroupCommaDecimal,
    NumberFormat::CommaGroupDotDecimal,
    NumberFormat::DotGroupCommaDecimal,
    NumberFormat::SpaceGroupDotDecimal,
];

fn number_format_label(n: NumberFormat) -> &'static str {
    match n {
        NumberFormat::SpaceGroupCommaDecimal => "1 234 567,89",
        NumberFormat::CommaGroupDotDecimal => "1,234,567.89",
        NumberFormat::DotGroupCommaDecimal => "1.234.567,89",
        NumberFormat::SpaceGroupDotDecimal => "1 234 567.89",
    }
}

const WEEKDAY_VARIANTS: &[Weekday] = &[
    Weekday::Monday,
    Weekday::Tuesday,
    Weekday::Wednesday,
    Weekday::Thursday,
    Weekday::Friday,
    Weekday::Saturday,
    Weekday::Sunday,
];

fn weekday_label(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

const TEMPERATURE_UNIT_VARIANTS: &[TemperatureUnit] =
    &[TemperatureUnit::Celsius, TemperatureUnit::Fahrenheit];

fn temperature_unit_label(u: TemperatureUnit) -> &'static str {
    match u {
        TemperatureUnit::Celsius => "Celsius",
        TemperatureUnit::Fahrenheit => "Fahrenheit",
    }
}

const UNIT_SYSTEM_VARIANTS: &[UnitSystem] = &[UnitSystem::Metric, UnitSystem::Imperial];

fn unit_system_label(u: UnitSystem) -> &'static str {
    match u {
        UnitSystem::Metric => "Metric",
        UnitSystem::Imperial => "Imperial",
    }
}

#[cfg(test)]
mod tests {
    use super::supported_tz_names;
    use bmc_shared_time::time::Timezone;
    use std::str::FromStr;

    #[test]
    fn dropdown_names_all_resolve_via_host_validator() {
        let mut unresolved: Vec<&str> = supported_tz_names()
            .filter(|name| Timezone::from_str(name).is_err())
            .collect();
        unresolved.sort_unstable();
        assert!(
            unresolved.is_empty(),
            "testbed offers {} tz name(s) the host validator rejects: {:?}",
            unresolved.len(),
            unresolved,
        );
    }
}
