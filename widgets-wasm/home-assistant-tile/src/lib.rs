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

//! Home Assistant Tile widget: one entity as a card, with the controls its size has room for.

#[cfg(target_arch = "wasm32")]
mod manifest_params;
pub mod view;

#[cfg(target_arch = "wasm32")]
mod wasm_glue {
    use bmc_wasm_sdk::*;

    use crate::manifest_params::Params;

    const MESSAGE_SIZE: u32 = 24;

    #[unsafe(no_mangle)]
    pub extern "C" fn init() {
        request_frame();
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn on_params_update() {
        request_frame();
    }

    /// No domain is supported yet, so every entity is refused.
    #[unsafe(no_mangle)]
    pub extern "C" fn render(_delta_ms: u32) {
        let entity_id = Params::current().entity_id;
        let message = if entity_id.trim().is_empty() {
            "Set an entity ID in the widget settings"
        } else {
            "This entity type is not supported yet"
        };
        let size = widget_size();
        let _ = render_ui(
            size.width,
            size.height,
            center(
                props!(background: BLACK),
                [text(
                    message.to_owned(),
                    style!(size: MESSAGE_SIZE, weight: FontWeight::REGULAR, color: GRAY_60),
                )],
            ),
        );
    }
}
