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

//! Segmented view-switcher component: a rounded pill of icon tabs, one active.

#![expect(
    clippy::cast_precision_loss,
    reason = "tab counts and indices stay tiny, exact within f32's integer range"
)]
#![allow(clippy::wildcard_imports)]

use bmc_wasm_protocol::*;

use crate::interaction::{InteractionState, Rect};
use crate::renderer::{RenderTarget, Renderer};
use crate::tree::{TouchHit, TreeResult};

// ── Geometry / theme ─────────────────────────────────────────────────

const TAB_W: f32 = 48.0;
const TAB_H: f32 = 40.0;
/// Outer corner radius; the two tabs meet flat in the middle.
const RADIUS: f32 = 4.0;
const ICON: f32 = 16.0;

const PILL_BG: Color = GRAY_100;
const ACTIVE_BG: Color = GRAY_10;
const ACTIVE_TINT: Color = GRAY_100;
const INACTIVE_TINT: Color = WHITE;
/// Opacity applied to every layer when the switcher is disabled.
const DISABLED_ALPHA: f32 = 0.4;

// ── Data ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SwitcherTabData {
    pub icon: Option<SvgId>,
    pub click_id: String,
}

#[derive(Clone, Debug)]
pub struct SwitcherData {
    pub active: usize,
    pub disabled: bool,
    pub tabs: Vec<SwitcherTabData>,
}

/// Intrinsic pill size: one fixed-width cell per tab.
#[must_use]
pub fn switcher_size(data: &SwitcherData) -> (f32, f32) {
    (TAB_W * data.tabs.len().max(1) as f32, TAB_H)
}

// ── Rendering ────────────────────────────────────────────────────────

/// Width of one tab, and the left edge of the tab at `index`.
fn tab_bounds(x: f32, w: f32, tabs: usize, index: usize) -> (f32, f32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a switcher carries a handful of tabs"
    )]
    let tab_w = w / tabs.max(1) as f32;
    #[expect(
        clippy::cast_precision_loss,
        reason = "a switcher carries a handful of tabs"
    )]
    let tx = x + index as f32 * tab_w;
    (tx, tab_w)
}

/// Draw the pill and its tabs; [`register_switcher_hits`] owns the hit regions.
pub(crate) fn render_switcher(
    data: &SwitcherData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut RenderTarget<'_, '_, '_>,
) {
    let dim = |c: Color| {
        if data.disabled {
            c.with_alpha(DISABLED_ALPHA)
        } else {
            c
        }
    };

    renderer.fill_rounded_rect(x, y, w, h, RADIUS, dim(PILL_BG));

    for (i, tab) in data.tabs.iter().enumerate() {
        let (tx, tab_w) = tab_bounds(x, w, data.tabs.len(), i);
        let active = i == data.active;

        if active {
            renderer.fill_rounded_rect(tx, y, tab_w, h, RADIUS, dim(ACTIVE_BG));
        }

        if let Some(id) = tab.icon {
            let tint = if active { ACTIVE_TINT } else { INACTIVE_TINT };
            renderer.draw_svg(
                tx + (tab_w - ICON) / 2.0,
                y + (h - ICON) / 2.0,
                ICON,
                ICON,
                dim(tint),
                id,
                true,
                &[],
            );
        }
    }
}

/// Register each tab's hit region and collect the clicks landing on them.
///
/// Split from the paint because the two run on different passes: hit regions are
/// rebuilt every frame and tested against the previous frame's set, so a pass
/// that paints nothing still has to register or it swallows the next tap.
pub(crate) fn register_switcher_hits(
    data: &SwitcherData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    interaction: &mut InteractionState,
    result: &mut TreeResult,
) {
    if data.disabled {
        return;
    }
    for (i, tab) in data.tabs.iter().enumerate() {
        let (tx, tab_w) = tab_bounds(x, w, data.tabs.len(), i);
        let (clicked, pos) = interaction.button_with_pos(&tab.click_id, Rect::new(tx, y, tab_w, h));
        if clicked && let Some((lx, ly)) = pos {
            result.clicks.insert(
                tab.click_id.clone(),
                TouchHit {
                    x: lx,
                    y: ly,
                    width: tab_w,
                    height: h,
                },
            );
        }
    }
}
