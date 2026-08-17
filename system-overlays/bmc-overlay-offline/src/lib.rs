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

//! Bottom-right "OFFLINE" indicator. Mapped only while the device has no
//! routable IPv4; unmaps when connectivity returns and remaps if it drops.

use std::time::{Duration, Instant};

use bmc_render::colors::Color;
use bmc_render::renderer::Renderer;
use bmc_render::tree::{FontFamily, FontWeight, TextAlign, TextStyle, VerticalAlign};
use bmc_system_overlay::{
    LayerConfig, SnapshotVersion, SystemOverlay, TickOutcome, VersionedSnapshot,
};

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

/// Injected connectivity source for testing.
trait Env {
    /// Latest snapshot and its version when the content changed since `seen`
    /// (`None` = nothing seen yet); `None` otherwise.
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ChipRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

struct OsEnv;
impl Env for OsEnv {
    fn snapshot_if_changed(&self, seen: Option<SnapshotVersion>) -> Option<VersionedSnapshot> {
        bmc_system_overlay::snapshot_if_changed(seen)
    }
}

/// Probed connectivity state driving the chip. `Unknown` is "not yet probed"
/// and keeps the chip hidden so boot never flashes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Connectivity {
    Unknown,
    Online,
    Offline,
}

/// Pure: given the probed connectivity state and last-rendered visibility,
/// decide the next `(visible, wants_render)`. The chip shows only when
/// `Offline`.
fn decide(state: Connectivity, was_visible: bool) -> (bool, bool) {
    let visible = matches!(state, Connectivity::Offline);
    let wants_render = visible && !was_visible;
    (visible, wants_render)
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
fn offline_chip_rect(size: (u32, u32), text_width: f32) -> ChipRect {
    let (w, h, font) = (size.0 as f32, size.1 as f32, FONT_PX as f32);
    let chip_w = text_width + PAD_X * 2.0;
    let chip_h = font * LINE_HEIGHT + PAD_Y * 2.0;
    ChipRect {
        x: w - chip_w,
        y: h - chip_h,
        w: chip_w,
        h: chip_h,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfflineView {
    pub visible: bool,
}

pub struct OfflineOverlay {
    visible: bool,
    /// Last probed connectivity; kept across polls where the snapshot is
    /// unchanged (the versioned read returns nothing then).
    state: Connectivity,
    /// Version of the snapshot `state` was derived from (`None` = none yet).
    snapshot_version: Option<SnapshotVersion>,
    env: Box<dyn Env>,
}

impl std::fmt::Debug for OfflineOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OfflineOverlay")
            .field("visible", &self.visible)
            .finish_non_exhaustive()
    }
}

impl Default for OfflineOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            state: Connectivity::Unknown,
            snapshot_version: None,
            env: Box::new(OsEnv),
        }
    }
}

impl OfflineOverlay {
    #[must_use]
    fn view(&self) -> OfflineView {
        OfflineView {
            visible: self.visible,
        }
    }
}

pub fn render_offline(r: &mut dyn Renderer, size: (u32, u32), _view: OfflineView) {
    let style = offline_text_style();
    #[expect(
        clippy::cast_precision_loss,
        reason = "font dimensions fit comfortably in f32 mantissa"
    )]
    let chip = offline_chip_rect(size, r.measure_text(LABEL, FONT_PX as f32));

    let (bg_r, bg_g, bg_b, bg_a) = BACKGROUND_RGBA;
    r.fill_rect(
        chip.x,
        chip.y,
        chip.w,
        chip.h,
        Color::from_rgba(bg_r, bg_g, bg_b, bg_a),
    );
    r.draw_canvas_text(LABEL, chip.x + chip.w / 2.0, chip.y + chip.h / 2.0, &style);
}

impl SystemOverlay for OfflineOverlay {
    fn layer_config(&self) -> LayerConfig {
        LayerConfig::bottom_right("bmc-overlay-offline", SIZE)
    }

    fn tick(&mut self, now: Instant) -> TickOutcome {
        if let Some(VersionedSnapshot { version, snapshot }) =
            self.env.snapshot_if_changed(self.snapshot_version)
        {
            self.snapshot_version = Some(version);
            self.state = if snapshot.ipv4.is_some() {
                Connectivity::Online
            } else {
                Connectivity::Offline
            };
        }
        let (visible, wants_render) = decide(self.state, self.visible);
        self.visible = visible;
        TickOutcome {
            visible,
            wants_render,
            next_wake: Some(now + POLL),
        }
    }

    fn render(&mut self, r: &mut dyn Renderer, size: (u32, u32)) {
        render_offline(r, size, self.view());
    }
}

#[cfg(test)]
mod tests {
    use bmc_system_overlay::Snapshot;

    use super::*;

    struct StaticEnv {
        snapshot: Option<Snapshot>,
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
    }

    #[test]
    fn offline_maps_and_renders_on_transition() {
        assert_eq!(decide(Connectivity::Offline, false), (true, true));
    }

    #[test]
    fn offline_stays_mapped_without_extra_render() {
        assert_eq!(decide(Connectivity::Offline, true), (true, false));
    }

    #[test]
    fn online_unmaps() {
        assert_eq!(decide(Connectivity::Online, true), (false, false));
    }

    #[test]
    fn unknown_keeps_chip_hidden() {
        assert_eq!(decide(Connectivity::Unknown, false), (false, false));
        assert_eq!(decide(Connectivity::Unknown, true), (false, false));
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
        let chip = offline_chip_rect(SIZE, measured_text_width);
        assert_close(chip.x, 48.0);
        assert_close(chip.y, 16.0);
        assert_close(chip.w, 112.0);
        assert_close(chip.h, 32.0);
        assert_eq!(BACKGROUND_RGBA.3, 0xC0);
        assert_close(offline_text_style().line_height, 1.0);
    }

    #[test]
    fn view_reflects_current_visibility_after_tick() {
        let start = Instant::now();
        let mut overlay = OfflineOverlay {
            env: Box::new(StaticEnv {
                snapshot: Some(Snapshot {
                    ipv4: None,
                    station_ipv4: None,
                    station_ssid: None,
                    wifi_signal_dbm: None,
                }),
            }),
            ..OfflineOverlay::default()
        };

        let _ = overlay.tick(start);

        assert_eq!(overlay.view(), OfflineView { visible: true });
    }

    // The versioned read returns nothing while the snapshot is unchanged, so
    // the chip must keep showing the last derived state instead of falling
    // back to Unknown (which would unmap it between probes).
    #[test]
    fn chip_stays_mapped_across_unchanged_polls() {
        let start = Instant::now();
        let mut overlay = OfflineOverlay {
            env: Box::new(StaticEnv {
                snapshot: Some(Snapshot {
                    ipv4: None,
                    station_ipv4: None,
                    station_ssid: None,
                    wifi_signal_dbm: None,
                }),
            }),
            ..OfflineOverlay::default()
        };

        let _ = overlay.tick(start);
        let second = overlay.tick(start + POLL);

        assert!(second.visible);
        assert!(!second.wants_render);
    }

    #[test]
    fn tick_stays_hidden_until_first_snapshot() {
        let start = Instant::now();
        let mut overlay = OfflineOverlay {
            env: Box::new(StaticEnv { snapshot: None }),
            ..OfflineOverlay::default()
        };

        let tick = overlay.tick(start);

        assert!(!tick.visible);
        assert!(!tick.wants_render);
        assert_eq!(tick.next_wake, Some(start + POLL));
    }
}
