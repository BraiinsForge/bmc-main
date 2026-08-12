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

//! Credentials section of the right sidebar,
//! beside the Params grid and the System form.
//!
//! One row per slot the widget's manifest declares.
//! The operator types an account name to bind
//! and clears it to unbind.
//!
//! The credential type stays fixed by the manifest,
//! mirroring production, where the editor picks an account
//! for a slot whose type it cannot change.
//!
//! Only the guest-visible view is editable here.
//! The host substitutes the secret half at egress, and no widget can read it.
//! The testbed holds secrets only when `--secrets` supplies them — recording
//! a fetch-backed widget needs one real egress pass;
//!  without the flag every substitution refuses before the wire.

use bmc_wasm_runtime::unified_fixture::UnifiedEvent;

use super::TestbedApp;
use super::recording::record_delivery;

/// Wire-shape entry for one bound slot, matching the `credentials` event.
fn bound_entry(type_id: &str, account: &str) -> serde_json::Value {
    serde_json::json!({ "type": type_id, "account": account })
}

/// Label-column width, so credential rows line up
/// with the Params and System sections above.
///
/// Those two land here on their own, because their longest keys
/// (`optional_boolean`, `first_day_of_week`) run ~17 mono characters.
/// Slot names are far shorter, so this section asks for the width.
const LABEL_COL_WIDTH: f32 = 118.0;

impl TestbedApp {
    /// Push a new credential view to every tile, cache it,
    /// and record a `CredentialDelivery` when recording is active.
    pub(super) fn apply_credentials_update(
        &mut self,
        new_credentials: serde_json::Map<String, serde_json::Value>,
    ) {
        if new_credentials == self.credentials {
            return;
        }
        let view = bmc_wasm_runtime::parse_credentials_json(&new_credentials);
        for tile in &mut self.tiles {
            if !tile.dead
                && let Some(runtime) = tile.runtime.as_mut()
            {
                runtime.deliver_credentials_update(view.clone(), self.secrets.clone());
            }
        }
        self.credentials = new_credentials;

        if let Some(rec) = self.recording_mode.state.as_mut() {
            let credentials = self.credentials.clone();
            record_delivery(rec, || UnifiedEvent::CredentialDelivery { credentials });
        }
    }

    /// Slot key paired with the type its manifest fixes it to.
    /// The operator picks the account; the type is never theirs to change.
    pub(super) fn credential_slots(&self) -> Vec<(String, String)> {
        self.manifest
            .credentials
            .iter()
            .map(|(key, slot)| (key.as_str().to_owned(), slot.type_id.clone()))
            .collect()
    }

    /// Render the whole section, header bar included,
    /// and report whether a row changed.
    /// Draws nothing for a widget that declares no slots.
    ///
    /// An empty account name means unbound, which is why each row
    /// is a plain text field rather than a checkbox beside a name:
    /// one control, whose empty state is the unbound state.
    pub(super) fn paint_credentials_section(
        scroll: &mut egui::Ui,
        slots: &[(String, String)],
        working: &mut serde_json::Map<String, serde_json::Value>,
    ) -> bool {
        if slots.is_empty() {
            return false;
        }
        super::params_ui::section_header_bar(
            scroll,
            "Credentials",
            super::params_ui::SYSTEM_ACCENT,
        );
        let mut changed = false;
        egui::Frame::NONE
            .inner_margin(egui::Margin::same(8))
            .show(scroll, |inner| {
                egui::Grid::new("credentials_grid")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .min_col_width(LABEL_COL_WIDTH)
                    .show(inner, |grid| {
                        for (slot, type_id) in slots {
                            changed |= super::system_ui::row(grid, slot, |cell, cell_w, _label| {
                                let mut account = working
                                    .get(slot)
                                    .and_then(|entry| entry.get("account"))
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or_default()
                                    .to_owned();
                                let response = cell.add_sized(
                                    [cell_w, cell.spacing().interact_size.y],
                                    egui::TextEdit::singleline(&mut account)
                                        .hint_text(format!("unbound ({type_id})")),
                                );
                                if !response.changed() {
                                    return false;
                                }
                                if account.trim().is_empty() {
                                    working.remove(slot);
                                } else {
                                    working
                                        .insert(slot.clone(), bound_entry(type_id, account.trim()));
                                }
                                true
                            });
                        }
                    });
            });
        scroll.add_space(12.0);

        changed
    }
}
