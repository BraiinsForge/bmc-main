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

//! Bottom-right corner status. Two indicators share the one surface:
//!
//! - the "OFFLINE" chip, mapped while the device has no routable IPv4;
//! - the mining-status pickaxe, violet while the miner tunes and red while it
//!   underperforms, is stopped, or cannot be reached. A miner that is simply
//!   mining draws nothing.
//!   It exists only where the compositor reports the `mining` platform capability.
//!
//! The chip wins the corner: while the device is offline the pickaxe is not
//! drawn, and it comes back in its current state once connectivity returns.

mod bos;
mod mining;
mod poller;

use std::time::{Duration, Instant};

use bmc_render::colors::{Color, RED_60, VIOLET_60};
use bmc_render::renderer::Renderer;
use bmc_render::tree::{FontFamily, FontWeight, TextAlign, TextStyle, VerticalAlign};
use bmc_render_macros::include_svg;
use bmc_system_overlay::{
    LayerConfig, PlatformCaps, SnapshotVersion, SystemOverlay, TickOutcome, VersionedSnapshot,
    register_icon,
};
use bmc_wasm_sdk::assets::Svg;

use crate::bos::BosClient;
pub use crate::mining::Status;
use crate::poller::{Poller, StatusVersion};

/// Surface size in logical pixels. The visible indicator is a content-tight box
/// drawn at the surface's bottom-right corner; the remainder stays transparent.
const SIZE: (u32, u32) = (160, 48);
/// Legacy display text; keep capital case.
const LABEL: &str = "OFFLINE";
/// Label font size in logical pixels.
const FONT_PX: u32 = 16;
/// Stable Slint status item used one 16px text line inside 8px vertical padding.
const LINE_HEIGHT: f32 = 1.0;
/// Horizontal padding around the label (8px outer + 16px inner item padding).
const PAD_X: f32 = 24.0;
/// Vertical padding around the label.
const PAD_Y: f32 = 8.0;
/// Translucent black indicator background.
const BACKGROUND_RGBA: (u8, u8, u8, u8) = (0, 0, 0, 0xC0);
/// Red label text (palette red-50).
const TEXT_RGBA: (u8, u8, u8, u8) = (249, 83, 85, 255);
/// Snapshot re-read (wake) cadence.
const POLL: Duration = Duration::from_secs(2);

/// Height of the card both indicators draw on: one label line inside its padding.
#[expect(
    clippy::cast_precision_loss,
    reason = "indicator dimensions fit comfortably in f32 mantissa"
)]
const CARD_H: f32 = FONT_PX as f32 * LINE_HEIGHT + PAD_Y * 2.0;

/// The pickaxe icon, drawn at the 20 px its own viewBox is cut for.
const PICKAXE: Svg = include_svg!("assets/mining.svg");
const PICKAXE_PX: f32 = 20.0;
const _: () = assert!(PICKAXE_PX <= CARD_H, "the pickaxe must fit the card");

/// Injected connectivity and mining sources for testing.
trait Env {
    /// Latest snapshot and its version when the content changed since `seen`
    /// (`None` = nothing seen yet); `None` otherwise.
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot>;

    /// Begin polling the BOS API. Called once, when the platform says it mines.
    fn start_mining_polls(&mut self);

    /// The mining status to show and its version when it changed since `seen`.
    fn mining_status_if_changed(
        &self,
        seen: Option<StatusVersion>,
    ) -> Option<(StatusVersion, Status)>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ChipRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Default)]
struct OsEnv {
    poller: Option<Poller>,
}

impl Env for OsEnv {
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
        bmc_system_overlay::snapshot_if_changed(seen)
    }

    fn start_mining_polls(&mut self) {
        if self.poller.is_none() {
            self.poller = Some(Poller::spawn(BosClient::from_env()));
        }
    }

    fn mining_status_if_changed(
        &self,
        seen: Option<StatusVersion>,
    ) -> Option<(StatusVersion, Status)> {
        self.poller.as_ref()?.status_if_changed(seen)
    }
}

/// Probed connectivity state driving the chip. `Unknown` is "not yet probed"
/// and keeps the chip hidden so boot never flashes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connectivity {
    Unknown,
    Online,
    Offline,
}

/// The pickaxe's colour, one per status that draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    /// Violet: the tuner is working and at least one board hashes.
    Tuning,
    /// Red: underperforming, stopped, or unreachable past the retry budget.
    Low,
}

impl Tint {
    #[must_use]
    pub fn color(self) -> Color {
        match self {
            Self::Tuning => VIOLET_60,
            Self::Low => RED_60,
        }
    }
}

/// What the corner shows. The two indicators never share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OfflineView {
    /// Nothing to draw, which leaves the surface unmapped.
    #[default]
    Hidden,
    /// The offline chip.
    Offline,
    /// The pickaxe, in the colour its status earns.
    Mining(Tint),
}

impl OfflineView {
    #[must_use]
    pub fn visible(self) -> bool {
        !matches!(self, Self::Hidden)
    }
}

/// Pure: the view for a connectivity state and the mining status to show
/// (`None` while there is none yet). The chip takes the corner outright;
/// the pickaxe draws only for the two statuses that have a colour.
#[must_use]
pub fn decide(connectivity: Connectivity, mining: Option<Status>) -> OfflineView {
    match (connectivity, mining) {
        (Connectivity::Offline, _) => OfflineView::Offline,
        (_, None | Some(Status::Ok)) => OfflineView::Hidden,
        (_, Some(Status::Tuning)) => OfflineView::Mining(Tint::Tuning),
        (_, Some(Status::Low)) => OfflineView::Mining(Tint::Low),
    }
}

/// Pure: whether moving from `was` to `now` needs a frame. A hidden view
/// never does; the framework unmaps it instead.
#[must_use]
fn wants_render(was: OfflineView, now: OfflineView) -> bool {
    now.visible() && now != was
}

fn offline_text_style() -> TextStyle {
    let (t_r, t_g, t_b, t_a) = TEXT_RGBA;
    TextStyle {
        size: FONT_PX,
        color: Color::from_rgba(t_r, t_g, t_b, t_a),
        weight: FontWeight::SEMIBOLD,
        line_height: LINE_HEIGHT,
        align: TextAlign::Center,
        vertical_align: VerticalAlign::Center,
        family: FontFamily::Sans,
        ..TextStyle::default()
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "indicator dimensions fit comfortably in f32 mantissa"
)]
fn card_rect(size: (u32, u32), card_w: f32) -> ChipRect {
    ChipRect {
        x: size.0 as f32 - card_w,
        y: size.1 as f32 - CARD_H,
        w: card_w,
        h: CARD_H,
    }
}

fn chip_rect(size: (u32, u32), label_width: f32) -> ChipRect {
    card_rect(size, label_width + PAD_X * 2.0)
}

/// The pickaxe's card is square: the chip's height on both axes.
fn pickaxe_card_rect(size: (u32, u32)) -> ChipRect {
    card_rect(size, CARD_H)
}

/// Top-left corner of the pickaxe, centred on its card.
fn pickaxe_origin(card: ChipRect) -> (f32, f32) {
    (
        card.x + (card.w - PICKAXE_PX) / 2.0,
        card.y + (card.h - PICKAXE_PX) / 2.0,
    )
}

fn fill_card(r: &mut dyn Renderer, card: ChipRect) {
    let (bg_r, bg_g, bg_b, bg_a) = BACKGROUND_RGBA;
    r.fill_rect(
        card.x,
        card.y,
        card.w,
        card.h,
        Color::from_rgba(bg_r, bg_g, bg_b, bg_a),
    );
}

pub struct OfflineOverlay {
    view: OfflineView,
    /// Last probed connectivity; kept across polls where the snapshot is
    /// unchanged (the versioned read returns nothing then).
    connectivity: Connectivity,
    /// Version of the snapshot `connectivity` was derived from (`None` = none yet).
    snapshot_version: Option<SnapshotVersion>,
    /// Last published mining status (`None` = none yet).
    mining: Option<Status>,
    /// Version of the status in `mining` (`None` = none yet).
    status_version: Option<StatusVersion>,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for OfflineOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineOverlay")
            .field("view", &self.view)
            .field("mining", &self.mining)
            .finish_non_exhaustive()
    }
}

impl Default for OfflineOverlay {
    fn default() -> Self {
        Self {
            view: OfflineView::default(),
            connectivity: Connectivity::Unknown,
            snapshot_version: None,
            mining: None,
            status_version: None,
            env: Box::new(OsEnv::default()),
        }
    }
}

fn render_chip(r: &mut dyn Renderer, size: (u32, u32)) {
    let style = offline_text_style();
    #[expect(
        clippy::cast_precision_loss,
        reason = "font dimensions fit comfortably in f32 mantissa"
    )]
    let card = chip_rect(size, r.measure_text(LABEL, FONT_PX as f32));
    fill_card(r, card);
    r.draw_canvas_text(LABEL, card.x + card.w / 2.0, card.y + card.h / 2.0, &style);
}

fn register_pickaxe(r: &mut dyn Renderer) -> Option<bmc_wasm_protocol::SvgId> {
    register_icon(PICKAXE.name, || {
        r.register_svg(PICKAXE.name, PICKAXE.source.data())
    })
}

fn render_pickaxe(r: &mut dyn Renderer, size: (u32, u32), tint: Tint) {
    let Some(icon) = register_pickaxe(r) else {
        return;
    };
    let card = pickaxe_card_rect(size);
    fill_card(r, card);
    let (x, y) = pickaxe_origin(card);
    r.draw_svg(x, y, PICKAXE_PX, PICKAXE_PX, tint.color(), icon, true, &[]);
}

pub fn render_offline(r: &mut dyn Renderer, size: (u32, u32), view: OfflineView) {
    match view {
        OfflineView::Hidden => {}
        OfflineView::Offline => render_chip(r, size),
        OfflineView::Mining(tint) => render_pickaxe(r, size, tint),
    }
}

impl SystemOverlay for OfflineOverlay {
    fn init(&mut self) {
        // A compositor without `deck_platform_v1` never answers, and this
        // line with no capabilities line after it is how that reads in a log.
        tracing::debug!("waiting for the platform capabilities that gate the pickaxe");
    }

    fn layer_config(&self) -> LayerConfig {
        LayerConfig::bottom_right("bmc-overlay-offline", SIZE)
    }

    fn uses_platform(&self) -> bool {
        true
    }

    fn on_platform_capabilities(&mut self, caps: PlatformCaps) {
        if caps.mining {
            self.env.start_mining_polls();
        } else {
            tracing::info!("platform does not mine; the corner shows the offline chip only");
        }
    }

    fn prewarm(&mut self, renderer: &mut dyn Renderer) {
        register_pickaxe(renderer);
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        if let Some(VersionedSnapshot { version, snapshot }) =
            self.env.snapshot_if_changed(self.snapshot_version)
        {
            self.snapshot_version = Some(version);
            self.connectivity = if snapshot.ipv4.is_some() {
                Connectivity::Online
            } else {
                Connectivity::Offline
            };
        }
        if let Some((version, status)) = self.env.mining_status_if_changed(self.status_version) {
            self.status_version = Some(version);
            self.mining = Some(status);
        }
        let view = decide(self.connectivity, self.mining);
        let wants_render = wants_render(self.view, view);
        self.view = view;
        TickOutcome {
            visible: view.visible(),
            wants_render,
            next_wake: Some(now + POLL),
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        render_offline(r, size, self.view);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use bmc_system_overlay::Snapshot;

    use super::*;

    struct StaticEnv {
        snapshot: Option<Snapshot>,
        polling: Cell<bool>,
        /// Statuses handed out one per read, in order, each as a new version.
        statuses: RefCell<VecDeque<Status>>,
        next_version: Cell<StatusVersion>,
    }

    impl StaticEnv {
        fn new(snapshot: Option<Snapshot>, statuses: Vec<Status>) -> Self {
            Self {
                snapshot,
                polling: Cell::new(false),
                statuses: RefCell::new(statuses.into()),
                next_version: Cell::new(StatusVersion::FIRST),
            }
        }
    }

    impl Env for StaticEnv {
        fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
            // Mimic the prober contract: the fixed snapshot is the first
            // version, so a caller that has folded it in gets no re-read.
            if seen.is_some() {
                return None;
            }
            self.snapshot.clone().map(|snapshot| VersionedSnapshot {
                version: SnapshotVersion::FIRST,
                snapshot,
            })
        }

        fn start_mining_polls(&mut self) {
            self.polling.set(true);
        }

        fn mining_status_if_changed(
            &self,
            _seen: Option<StatusVersion>,
        ) -> Option<(StatusVersion, Status)> {
            if !self.polling.get() {
                return None;
            }
            let status = self.statuses.borrow_mut().pop_front()?;
            let version = self.next_version.get();
            self.next_version.set(version.next());
            Some((version, status))
        }
    }

    fn online() -> Snapshot {
        Snapshot {
            ipv4: Some(std::net::Ipv4Addr::new(10, 0, 0, 5)),
            station_ipv4: None,
            station_ssid: None,
            wifi_signal_dbm: None,
        }
    }

    fn offline() -> Snapshot {
        Snapshot {
            ipv4: None,
            station_ipv4: None,
            station_ssid: None,
            wifi_signal_dbm: None,
        }
    }

    const MINER: PlatformCaps = PlatformCaps {
        wifi: true,
        ethernet: true,
        mining: true,
        boser_managed: true,
    };

    fn overlay_with(snapshot: Option<Snapshot>, statuses: Vec<Status>) -> OfflineOverlay {
        OfflineOverlay {
            env: Box::new(StaticEnv::new(snapshot, statuses)),
            ..OfflineOverlay::default()
        }
    }

    #[test]
    fn offline_shows_the_chip_and_never_the_pickaxe() {
        for mining in [
            None,
            Some(Status::Tuning),
            Some(Status::Ok),
            Some(Status::Low),
        ] {
            assert_eq!(
                decide(Connectivity::Offline, mining),
                OfflineView::Offline,
                "{mining:?}"
            );
        }
    }

    #[test]
    fn online_shows_a_pickaxe_for_tuning_and_low_only() {
        assert_eq!(
            decide(Connectivity::Online, Some(Status::Tuning)),
            OfflineView::Mining(Tint::Tuning)
        );
        assert_eq!(
            decide(Connectivity::Online, Some(Status::Low)),
            OfflineView::Mining(Tint::Low)
        );
        assert_eq!(
            decide(Connectivity::Online, Some(Status::Ok)),
            OfflineView::Hidden
        );
        assert_eq!(decide(Connectivity::Online, None), OfflineView::Hidden);
    }

    #[test]
    fn unknown_connectivity_hides_the_chip_but_not_the_pickaxe() {
        // Boot never flashes the chip;
        // a mining answer already in does not wait for the first probe.
        assert_eq!(decide(Connectivity::Unknown, None), OfflineView::Hidden);
        assert_eq!(
            decide(Connectivity::Unknown, Some(Status::Low)),
            OfflineView::Mining(Tint::Low)
        );
    }

    #[test]
    fn a_frame_is_wanted_only_for_a_visible_change() {
        let violet = decide(Connectivity::Online, Some(Status::Tuning));
        let red = decide(Connectivity::Online, Some(Status::Low));
        assert!(wants_render(OfflineView::Hidden, violet));
        assert!(wants_render(violet, red));
        assert!(wants_render(red, OfflineView::Offline));
        assert!(!wants_render(violet, violet));
        assert!(!wants_render(violet, OfflineView::Hidden));
    }

    #[test]
    fn tints_are_the_palette_violet_and_red() {
        assert_eq!(Tint::Tuning.color(), VIOLET_60);
        assert_eq!(Tint::Low.color(), RED_60);
    }

    #[test]
    fn constants_keep_offline_indicator_legible_and_translucent() {
        assert_eq!(LABEL, "OFFLINE");
        assert_eq!(FONT_PX, 16);

        assert_eq!(BACKGROUND_RGBA, (0, 0, 0, 0xC0));
        let (red, green, blue, alpha) = TEXT_RGBA;
        assert!(red > green);
        assert!(red > blue);
        assert_eq!(alpha, u8::MAX);
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "expected {expected}, got {actual}",
        );
    }

    #[test]
    fn geometry_matches_stable_status_bar_shape_and_opacity() {
        let measured_text_width = 64.0;
        let chip = chip_rect(SIZE, measured_text_width);
        assert_close(chip.x, 48.0);
        assert_close(chip.y, 16.0);
        assert_close(chip.w, 112.0);
        assert_close(chip.h, 32.0);
        assert_eq!(BACKGROUND_RGBA.3, 0xC0);
        assert_close(offline_text_style().line_height, 1.0);
    }

    #[test]
    fn the_pickaxe_card_is_a_square_of_the_chip_height_in_the_corner() {
        let card = pickaxe_card_rect(SIZE);
        assert_close(card.x, 128.0);
        assert_close(card.y, 16.0);
        assert_close(card.w, 32.0);
        assert_close(card.h, 32.0);
    }

    #[test]
    fn the_pickaxe_is_centred_on_its_card() {
        let (x, y) = pickaxe_origin(pickaxe_card_rect(SIZE));
        assert_close(x, 134.0);
        assert_close(y, 22.0);
    }

    #[test]
    fn view_reflects_current_visibility_after_tick() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(offline()), Vec::new());

        let _ = overlay.tick(start);

        assert_eq!(overlay.view, OfflineView::Offline);
    }

    // The versioned read returns nothing while the snapshot is unchanged, so
    // the chip must keep showing the last derived state instead of falling
    // back to Unknown (which would unmap it between probes).
    #[test]
    fn chip_stays_mapped_across_unchanged_polls() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(offline()), Vec::new());

        let _ = overlay.tick(start);
        let second = overlay.tick(start + POLL);

        assert!(second.visible);
        assert!(!second.wants_render);
    }

    #[test]
    fn tick_stays_hidden_until_first_snapshot() {
        let start = Instant::now();
        let mut overlay = overlay_with(None, Vec::new());

        let tick = overlay.tick(start);

        assert!(!tick.visible);
        assert!(!tick.wants_render);
        assert_eq!(tick.next_wake, Some(start + POLL));
    }

    #[test]
    fn a_platform_without_mining_never_polls() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(online()), vec![Status::Tuning]);
        overlay.on_platform_capabilities(PlatformCaps::default());

        let tick = overlay.tick(start);

        assert!(
            !tick.visible,
            "the status must not have been read: {overlay:?}"
        );
    }

    #[test]
    fn a_tuning_miner_maps_a_violet_pickaxe_once_online() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(online()), vec![Status::Tuning]);
        overlay.on_platform_capabilities(MINER);

        let tick = overlay.tick(start);

        assert!(tick.visible);
        assert!(tick.wants_render);
        assert_eq!(overlay.view, OfflineView::Mining(Tint::Tuning));
    }

    #[test]
    fn a_healthy_miner_unmaps_the_pickaxe() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(online()), vec![Status::Tuning, Status::Ok]);
        overlay.on_platform_capabilities(MINER);

        let _ = overlay.tick(start);
        let settled = overlay.tick(start + POLL);

        assert!(!settled.visible);
        assert_eq!(overlay.view, OfflineView::Hidden);
    }

    #[test]
    fn an_unreachable_miner_maps_a_red_pickaxe() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(online()), vec![Status::Low]);
        overlay.on_platform_capabilities(MINER);

        let tick = overlay.tick(start);

        assert!(tick.visible);
        assert_eq!(overlay.view, OfflineView::Mining(Tint::Low));
    }

    #[test]
    fn the_chip_covers_the_pickaxe_while_offline() {
        let start = Instant::now();
        let mut overlay = overlay_with(Some(offline()), vec![Status::Tuning]);
        overlay.on_platform_capabilities(MINER);

        let tick = overlay.tick(start);

        assert!(tick.visible);
        assert_eq!(overlay.view, OfflineView::Offline);
        assert_eq!(
            overlay.mining,
            Some(Status::Tuning),
            "the status is still read, so the pickaxe returns with connectivity"
        );
    }
}
