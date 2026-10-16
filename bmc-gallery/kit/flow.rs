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

//! Stages of mixed sizes, laid out as a wrapping flow.

use bmc_wasm_sdk::tree::Node;
use gallery::eframe::egui;
use gallery::{PADDING, SceneCtx};

use super::{DeckSceneCtx, DeckSize};

/// A stage under its heading, as [`WrappingFlow::wrapping_flow`] lays it out.
pub struct Titled<Label, Build> {
    heading: Label,
    size: DeckSize,
    build: Build,
}

impl<Label, Build> std::fmt::Debug for Titled<Label, Build> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Titled")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// Stage `build`'s tree at `size` under `heading`.
///
/// `build` runs inside the stage, where the asset registrars are live.
pub fn titled<Label, Build>(
    heading: Label,
    size: impl Into<DeckSize>,
    build: Build,
) -> Titled<Label, Build>
where
    Label: Into<egui::RichText>,
    Build: FnOnce() -> Node,
{
    Titled {
        heading,
        size: size.into(),
        build,
    }
}

/// Blocks of mixed widths, left to right and top-aligned,
/// wrapped onto a new row where the next would overrun the pane.
///
/// Unlike [`SceneCtx::matrix_with`], no block takes the widest one's column
/// or the tallest one's row, so a 1280 px frame sits beside a 638 px one.
/// A block is as tall as its content, so content that fills the height
/// it is offered, such as a `ScrollArea`, has none to fill.
pub trait WrappingFlow {
    /// Each stage under its heading, packed by its own width.
    ///
    /// ```ignore
    /// ctx.wrapping_flow(ui, BUCKETS.map(|(bucket, label)| {
    ///     titled(label, bucket.design_size(), move || view(bucket))
    /// }));
    /// ```
    fn wrapping_flow<Label, Build>(
        &mut self,
        ui: &mut egui::Ui,
        stages: impl IntoIterator<Item = Titled<Label, Build>>,
    ) where
        Label: Into<egui::RichText>,
        Build: FnOnce() -> Node;

    /// [`wrapping_flow`](Self::wrapping_flow) with each block left to `add`.
    /// `width` gives a block's stage width up front, as [`stage_width`] measures it,
    /// since a block wraps only on a width known before it is laid out.
    fn wrapping_flow_with<T>(
        &mut self,
        ui: &mut egui::Ui,
        items: impl IntoIterator<Item = T>,
        width: impl Fn(&T) -> f32,
        add: impl FnMut(&mut Self, &mut egui::Ui, T),
    );
}

impl WrappingFlow for SceneCtx<'_> {
    fn wrapping_flow<Label, Build>(
        &mut self,
        ui: &mut egui::Ui,
        stages: impl IntoIterator<Item = Titled<Label, Build>>,
    ) where
        Label: Into<egui::RichText>,
        Build: FnOnce() -> Node,
    {
        self.wrapping_flow_with(
            ui,
            stages,
            |stage| stage_width(stage.size),
            |ctx, ui, stage| {
                ui.heading(stage.heading);
                ctx.node_stage(ui, stage.size, stage.build);
            },
        );
    }

    fn wrapping_flow_with<T>(
        &mut self,
        ui: &mut egui::Ui,
        items: impl IntoIterator<Item = T>,
        width: impl Fn(&T) -> f32,
        mut add: impl FnMut(&mut Self, &mut egui::Ui, T),
    ) {
        let rows = egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true);
        let column = egui::Layout::top_down(egui::Align::Min);
        ui.with_layout(rows, |ui| {
            for item in items {
                // The checkerboard pads the stage on either side.
                let block = egui::vec2(width(&item) + 2.0 * f32::from(PADDING), 0.0);
                ui.allocate_ui_with_layout(block, column, |cell| add(self, cell, item));
            }
        });
    }
}

/// The width a stage of `size` takes in a [`WrappingFlow`], checkerboard aside.
#[must_use]
pub fn stage_width(size: DeckSize) -> f32 {
    f32::from(u16::try_from(size.width()).expect("BUG: a device frame fits u16"))
}
