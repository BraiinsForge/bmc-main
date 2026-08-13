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

//! Keyboard icon assets compiled from Carbon Design System SVGs (Apache 2.0).
//!
//! Icons are compiled at build time via `include_svg!` and registered with
//! the renderer on each call. Registration is idempotent host-side, so
//! re-registering returns the cached ID without re-parsing.
//!
//! ## Fail-soft on registration failure
//!
//! If [`Renderer::register_svg`] returns `None` (renderer-side parse failure
//! or ID-space exhaustion — see BDK-458 for ID-lifecycle), the function key's
//! icon spot renders blank: function keys (Shift / Backspace / Enter) carry
//! an empty `key.label`, so the renderer's text path draws nothing. The
//! keyboard remains operable — only the visual cue is missing. The first
//! failure per tag is logged via `tracing`; subsequent failures are
//! suppressed to avoid per-frame spam.

use std::cell::RefCell;
use std::collections::HashSet;

use bmc_render::renderer::Renderer;
use bmc_render_macros::include_svg;
use bmc_wasm_protocol::SvgId;
use bmc_wasm_sdk::assets::Svg;

// Carbon Design System icons (Apache 2.0).
// TODO(kubijo): manually clean up SVGs — remove padding, scale to 24, union paths.
const ICON_SHIFT: Svg = include_svg!("bmc-render/keyboard/assets/icons/shift.svg");
const ICON_BACKSPACE: Svg = include_svg!("bmc-render/keyboard/assets/icons/backspace.svg");
const ICON_ENTER: Svg = include_svg!("bmc-render/keyboard/assets/icons/enter.svg");

thread_local! {
    static WARNED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
}

fn id_for(renderer: &mut dyn Renderer, icon: &Svg) -> Option<SvgId> {
    let id = renderer.register_svg(icon.name, icon.source.data());
    if id.is_none() {
        WARNED.with_borrow_mut(|w| {
            if w.insert(icon.name) {
                tracing::error!(
                    icon = icon.name,
                    "keyboard icon failed to register; the function key will render blank"
                );
            }
        });
    }
    id
}

pub fn shift_id(renderer: &mut dyn Renderer) -> Option<SvgId> {
    id_for(renderer, &ICON_SHIFT)
}

pub fn backspace_id(renderer: &mut dyn Renderer) -> Option<SvgId> {
    id_for(renderer, &ICON_BACKSPACE)
}

pub fn enter_id(renderer: &mut dyn Renderer) -> Option<SvgId> {
    id_for(renderer, &ICON_ENTER)
}
