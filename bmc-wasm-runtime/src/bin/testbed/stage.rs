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

//! What is on the canvas: which devices are open, the views realising them,
//! whatever is on its way out, and a pending arrangement.
//!
//! One type rather than four fields, because they only make sense together.
//! A view may leave `tiles` for `teardown` and nowhere else — dropping one
//! instead leaks its GL texture, since the painter is only in hand between
//! passes — and the open set drives what the next redraw builds, so the two
//! have to move as one. Both facts are ops here rather than a sequence each
//! caller is trusted to repeat.

use bmc_wasm_runtime::platform_catalog::Platform;

use super::view::{self, DeviceView};

#[derive(Default)]
pub(crate) struct Stage {
    open: Vec<&'static Platform>,
    tiles: Vec<DeviceView>,
    teardown: Teardown,
    arrange: bool,
}

impl Stage {
    /// The devices on the canvas, one window each.
    pub(crate) fn open(&self) -> &[&'static Platform] {
        &self.open
    }

    pub(crate) fn is_open(&self, platform: &Platform) -> bool {
        self.open.iter().any(|p| p.id == platform.id)
    }

    pub(crate) fn tiles(&self) -> &[DeviceView] {
        &self.tiles
    }

    pub(crate) fn tiles_mut(&mut self) -> &mut [DeviceView] {
        &mut self.tiles
    }

    pub(crate) fn tile(&self, idx: usize) -> Option<&DeviceView> {
        self.tiles.get(idx)
    }

    pub(crate) fn tile_mut(&mut self, idx: usize) -> Option<&mut DeviceView> {
        self.tiles.get_mut(idx)
    }

    /// Where a platform's views sit, in viewport order — [`Self::realise`]
    /// appends a platform's worth contiguously, so the order holds.
    pub(crate) fn tiles_of(&self, platform: &Platform) -> Vec<usize> {
        self.tiles
            .iter()
            .enumerate()
            .filter(|(_, view)| view.platform.id == platform.id)
            .map(|(idx, _)| idx)
            .collect()
    }

    pub(crate) fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub(crate) fn is_bare(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Open a closed device or close an open one, returning whether it is now
    /// open. Closing retires its views here and then: releasing them needs the
    /// painter, which the next redraw has and this does not.
    pub(crate) fn toggle(&mut self, platform: &'static Platform) -> bool {
        if self.is_open(platform) {
            let mut open = std::mem::take(&mut self.open);
            open.retain(|p| p.id != platform.id);
            self.set_open(open);
            false
        } else {
            self.open.push(platform);
            true
        }
    }

    /// Make `platforms` the open set, retiring the views of everything that
    /// just left it. Views of platforms that stay are untouched, so a canvas
    /// that only grows rebuilds nothing.
    pub(crate) fn set_open(&mut self, platforms: Vec<&'static Platform>) {
        let (kept, closed): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tiles)
            .into_iter()
            .partition(|view| platforms.iter().any(|p| p.id == view.platform.id));
        self.tiles = kept;
        self.teardown.views.extend(closed);
        self.open = platforms;
    }

    /// Pin the canvas to one device and retire every view, returning the open
    /// set that was displaced so its holder can put it back.
    ///
    /// Every view retires, the pinned platform's included: recording rebuilds
    /// them against a different runtime config, which a live view cannot swap.
    pub(crate) fn pin_to(&mut self, platform: &'static Platform) -> Vec<&'static Platform> {
        self.retire_all();
        std::mem::replace(&mut self.open, vec![platform])
    }

    /// Retire every view while leaving the open set alone, so the next redraw
    /// builds the same canvas afresh.
    pub(crate) fn retire_all(&mut self) {
        self.teardown.views.extend(std::mem::take(&mut self.tiles));
    }

    /// Open devices with no views yet — what the next build has to make good.
    pub(crate) fn unrealised(&self) -> Vec<&'static Platform> {
        self.open
            .iter()
            .copied()
            .filter(|p| !self.tiles.iter().any(|view| view.platform.id == p.id))
            .collect()
    }

    /// Take custody of freshly built views.
    pub(crate) fn realise(&mut self, tiles: Vec<DeviceView>) {
        self.tiles.extend(tiles);
    }

    /// Ask for the windows to be rearranged at the next paint that can.
    pub(crate) fn request_arrange(&mut self) {
        self.arrange = true;
    }

    /// Whether an arrangement is due, once there is a canvas to arrange.
    ///
    /// Held rather than handed over while any open device still lacks its
    /// views: a mode swap retires them all, and an arrangement computed over
    /// that one-frame gap would place — and spend itself on — an empty canvas.
    pub(crate) fn take_arrange(&mut self) -> bool {
        let ready = self
            .open
            .iter()
            .all(|p| self.tiles.iter().any(|view| view.platform.id == p.id));
        ready && std::mem::take(&mut self.arrange)
    }

    /// Advance the teardown pipeline one step, with the painter in hand.
    pub(crate) fn drain_teardown(
        &mut self,
        gl: &egui_glow::glow::Context,
        painter: &mut egui_glow::Painter,
    ) {
        self.teardown.drain(gl, painter);
    }

    /// Retire everything and wait it out. For process exit only.
    pub(crate) fn shutdown(
        &mut self,
        gl: &egui_glow::glow::Context,
        painter: &mut egui_glow::Painter,
    ) {
        self.retire_all();
        self.teardown.finish(gl, painter);
    }
}

/// The teardown pipeline: whatever was closed, on its way out across frames.
///
/// A closed view first waits for a pass that can free its texture — the
/// painter is only in hand between passes — and a threaded view's worker then
/// winds down in the background until a poll collects it. Spreading that over
/// frames is the point: a runtime's teardown can hold a fetch for its whole
/// I/O timeout, and closing a platform must never stall the UI on it.
#[derive(Default)]
struct Teardown {
    /// Views taken out of service — a closed platform's worth,
    /// or every open one at once when recording mode swaps the whole canvas.
    views: Vec<DeviceView>,
    /// Worker threads asked to stop, polled until they exit.
    workers: Vec<view::worker::Retired>,
}

impl Teardown {
    /// Advance the pipeline one step: free what the painter can, poll the rest.
    fn drain(&mut self, gl: &egui_glow::glow::Context, painter: &mut egui_glow::Painter) {
        for view in std::mem::take(&mut self.views) {
            self.workers.extend(view.release(gl, painter));
        }
        self.workers = std::mem::take(&mut self.workers)
            .into_iter()
            .filter_map(view::worker::Retired::reap)
            .collect();
    }

    /// Run the pipeline to the end, waiting out every worker.
    ///
    /// For process exit only: blocking is fine with no UI left, and a detached
    /// worker would race its GL context against the dying display connection.
    fn finish(&mut self, gl: &egui_glow::glow::Context, painter: &mut egui_glow::Painter) {
        for view in std::mem::take(&mut self.views) {
            self.workers.extend(view.release(gl, painter));
        }
        for retired in std::mem::take(&mut self.workers) {
            retired.reap_blocking();
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_runtime::platform_catalog;

    use super::{Platform, Stage};

    fn platform(id: &str) -> &'static Platform {
        platform_catalog::platform(id)
            .unwrap_or_else(|| panic!("BUG: '{id}' must be in the catalog"))
    }

    #[test]
    fn toggling_opens_a_platform_and_toggling_again_closes_it() {
        let mut stage = Stage::default();
        stage.set_open(vec![platform("bmc100")]);

        assert!(stage.toggle(platform("bmm101")), "opens");
        assert_eq!(stage.open().len(), 2, "both devices stay open together");

        assert!(!stage.toggle(platform("bmm101")), "closes");
        assert_eq!(stage.open().len(), 1);
        assert_eq!(
            stage.open()[0].id,
            "bmc100",
            "the other device is untouched"
        );
    }

    #[test]
    fn an_arrangement_waits_for_the_views_it_would_place() {
        let mut stage = Stage::default();
        stage.set_open(vec![platform("bmc100")]);
        stage.request_arrange();

        assert!(
            !stage.take_arrange(),
            "an open device with no views yet would be arranged as an empty canvas",
        );

        stage.set_open(Vec::new());
        assert!(
            stage.take_arrange(),
            "with no device waiting for views the arrangement is due",
        );
    }
}
