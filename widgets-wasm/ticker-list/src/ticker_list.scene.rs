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

//! Every state of the list, rendered natively over fixture data
//! on each device frame the manifest admits.

use bmc_gallery::prelude::*;
use ticker_list::fixtures::{self, List};
use ticker_list::render;

scene_meta! { title: "Widgets / Tickers / Ticker List" }

/// Every device frame the gallery knows of, less the round face,
/// which the manifest does not admit.
fn viewports() -> impl Iterator<Item = DeviceViewport> {
    DEVICE_VIEWPORTS
        .into_iter()
        .filter(|viewport| !viewport.size.is_round())
}

/// Which viewport to stage, as an index into [`viewports`], `None` for all of them.
/// A capture recipe pins one: six stages stacked overrun the renderer's texture bound.
fn only_size(ctx: &mut SceneCtx) -> Option<usize> {
    let mut labels = vec!["All"];
    labels.extend(viewports().map(|viewport| viewport.label));
    ctx.select("Size", &labels, 0).checked_sub(1)
}

fn list_stages(ctx: &mut SceneCtx, ui: &mut Ui, list: fn() -> List) {
    let only = only_size(ctx);
    system_settings(ctx);
    for (index, viewport) in viewports().enumerate() {
        if only.is_some_and(|wanted| wanted != index) {
            continue;
        }
        let (width, height) = viewport.pixels();
        let size = WidgetSize::from_dimensions(width, height);
        ui.heading(viewport.label);
        ctx.node_stage(ui, viewport.size, move || {
            let list = list();
            render::view(&list.symbols, &list.states, &list.names, &list.stale, size)
        });
    }
}

#[scene(default)]
fn healthy(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::healthy);
}

#[scene]
fn mixed_states(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::mixed);
}

#[scene]
fn loading(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::loading);
}

#[scene]
fn failed(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::failed);
}

#[scene]
fn stale(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::stale);
}

#[scene]
fn closed_markets(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::closed_markets);
}

#[scene]
fn no_symbols(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::no_symbols);
}

#[scene]
fn one_symbol(ctx: &mut SceneCtx, ui: &mut Ui) {
    list_stages(ctx, ui, fixtures::one_symbol);
}
