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

//! Pure UI-tree construction for the settings overlay. Builds a
//! `bmc_render::tree::TreeNode` the GPU renderer lays out and paints. Kept free
//! of host/GL imports so it compiles and unit-tests on the host.

use bmc_platform::DisplayShape;
use bmc_render::tree::{PropsData, TreeNode, col};
use bmc_wasm_protocol::{Color, SvgId};

mod compact;
mod controls;
mod notice;
mod parts;
mod round;
mod station;
mod wide;

#[cfg(test)]
mod test_support;

use compact::compact_children;
use controls::{control_groups, control_rows};
use notice::Notice;
use parts::close_button;
use round::round_children;
use wide::wide_children;

/// Stable touch key for the WiFi reconfiguration hold button.
pub const WIFI_RECONFIG_KEY: &str = "wifi_reconfig";

/// Stable touch key for the night-mode tap toggle.
pub const NIGHT_MODE_KEY: &str = "night_mode";

/// Stable touch key for the restart hold button.
pub const RESTART_KEY: &str = "restart";

/// Stable touch key for the volume −10 step button.
pub const VOLUME_DOWN_KEY: &str = "volume_down";

/// Stable touch key for the volume +10 step button.
pub const VOLUME_UP_KEY: &str = "volume_up";

/// Stable touch key for the brightness −10 step button.
pub const BRIGHTNESS_DOWN_KEY: &str = "brightness_down";

/// Stable touch key for the brightness +10 step button.
pub const BRIGHTNESS_UP_KEY: &str = "brightness_up";

/// Stable touch key for the close (dismiss) button.
pub const CLOSE_KEY: &str = "close";

/// Stable touch key for the brightness slider drag.
pub const BRIGHTNESS_SLIDER_KEY: &str = "brightness_slider";

/// Brightness floor: below this the panel is too dark to find the control
/// that would undo it, so neither the step buttons nor the slider go lower.
pub const MIN_BRIGHTNESS: u8 = 10;

/// Percentage points between the slider's stops. Every drag frame reports a
/// position, so the value has to land on a grid: without one a single sweep
/// would queue a `SetBrightness` per frame, and each of those rewrites the
/// config file on flash.
const BRIGHTNESS_GRID: u8 = 5;

/// Stops between the floor and full brightness, the floor not counted.
const BRIGHTNESS_STOPS: u8 = 18;

const _: () = assert!(
    MIN_BRIGHTNESS + BRIGHTNESS_STOPS * BRIGHTNESS_GRID == 100,
    "BUG: the slider's stops must land on full brightness exactly",
);

/// Where the thumb sits for `percent`. The track spans the floor to full
/// rather than zero to full, so its left end is the dimmest the panel goes
/// and no part of the track is dead.
#[must_use]
pub fn brightness_fraction(percent: u8) -> f32 {
    let span = f32::from(100 - MIN_BRIGHTNESS);
    f32::from(percent.clamp(MIN_BRIGHTNESS, 100) - MIN_BRIGHTNESS) / span
}

/// The brightness a thumb dragged to `fraction` asks for, snapped to
/// [`BRIGHTNESS_GRID`]. Inverse of [`brightness_fraction`].
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a rounded 0..=1 fraction scaled by the stop count is a small count"
)]
pub fn brightness_from_fraction(fraction: f32) -> u8 {
    let stop = (fraction.clamp(0.0, 1.0) * f32::from(BRIGHTNESS_STOPS)).round() as u8;
    MIN_BRIGHTNESS + stop * BRIGHTNESS_GRID
}

const NO_DATA_PLACEHOLDER: &str = "---";

/// Panel scrim: the tray composites over the live scene, so its background is a
/// near-opaque black that lets the scene faintly show through. Matches the
/// `0.95`-opacity black the shipped swipe rollettes used.
const SCRIM: Color = Color::from_rgba(0, 0, 0, 0xF2);

/// Resting circle fill behind every control icon.
const CIRCLE_FILL: Color = Color::from_rgba(255, 255, 255, 77);

/// Circle fill while the finger is down; the icon is tinted black so it stays
/// legible on the near-white disc.
const CIRCLE_PRESSED: Color = Color::from_rgba(255, 255, 255, 204);

/// Icon tint paired with [`CIRCLE_PRESSED`].
const ICON_PRESSED_TINT: Color = Color::from_rgba(0, 0, 0, 255);

/// Circle fill of the night-mode button while night mode is active. The icon
/// stays white on this blue in both press states — the press inversion is
/// deliberately suppressed so active night mode always reads as blue.
const NIGHT_ACTIVE: Color = Color::from_rgba(0x10, 0x43, 0xCD, 255);

/// Hold-progress circle color.
const HOLD_FILL: Color = Color::from_rgba(0x8B, 0x7C, 0xFF, 255);

/// Hold fraction by which the circle is fully opaque.
/// It appears as soon as the button is touched either way; finishing the fade
/// this late means it reaches full strength while the shrink is under way,
/// not before the shrink starts — 1.75 s into the longest (5 s) hold.
const HOLD_ALPHA_FULL_AT: f32 = 0.35;

/// Edge length of the square close touch target.
const CLOSE_TARGET: f32 = 48.0;

/// Edge length of the close glyph inside its touch target.
const CLOSE_GLYPH: f32 = 24.0;

/// Gap between the two buttons of a ± pair on the Large tier.
const STEP_GAP_LARGE: f32 = 12.0;

/// Fixed text-block widths on the Large tier so caption swaps never shift
/// the centered-row math.
const LARGE_PAIR_W: f32 = 236.0;
const LARGE_SINGLE_W: f32 = 180.0;

/// BMM101's single-button width. Three groups and their gaps must fit 480px,
/// which three of the Large tier's 180 do not.
const BMM101_SINGLE_W: f32 = 146.0;

/// Stable geometry of the Large tier's top info section: panel top padding,
/// left inset, and the right inset keeping the section clear of the close
/// target (26px edge + 48px glyph + 32px spacing).
const WIDE_TOP_PAD: f32 = 33.0;
const WIDE_INFO_LEFT_PAD: f32 = 32.0;
const WIDE_INFO_RIGHT_PAD: f32 = 106.0;

/// Size of the gray headers above the Large tier's info values.
const INFO_HEADER_SIZE: u32 = 16;

/// Gap between an info header and its value.
const INFO_HEADER_GAP: f32 = 4.0;

/// Widths past which the runtime strings in the Large tier's info blocks end in "…".
const WIDE_HOSTNAME_WIDTH: u32 = 320;
const WIDE_SSID_WIDTH: u32 = 400;

/// Edge length of the IP QR code, and of the canvas it is drawn on.
/// An `http://<ipv4>` payload fits the 26 bytes a version-2 symbol holds
/// at the renderer's ECC level, so the grid is always 25 modules
/// plus the quiet zone. That puts a module at ~4.4px — 0.51mm
/// at the panel's 217 DPI, well clear of what a phone camera resolves.
const WIDE_QR_SIZE: f32 = 144.0;

/// The same 25-module symbol as [`WIDE_QR_SIZE`], shrunk to share its row
/// with the address table. With the quiet zone the grid is 33 modules, so
/// one is 2.9 px: 0.45 mm at BMM101's 165 DPI, the smallest code the tray draws.
const COMPACT_QR_SIZE: f32 = 96.0;

/// The compact layout's one spacing unit, vertical and horizontal alike.
const COMPACT_GAP: f32 = 16.0;

/// Clearance between the compact info row's right edge and the close target,
/// so the row never runs under the button's hit region.
const COMPACT_CLOSE_MARGIN: f32 = 16.0;

/// Text size of the compact info table, labels and values alike.
const COMPACT_INFO_SIZE: u32 = 16;

/// Side of the square brightness icon beside the slider.
const BRIGHTNESS_ICON_SIZE: f32 = 32.0;

/// Track thickness of the brightness slider. The node budgets a drag thumb
/// above and below, so it lays out five times this tall.
const SLIDER_TRACK_H: f32 = 8.0;

/// Modules of blank margin around the IP QR code; ISO/IEC 18004 asks four.
/// Finder-pattern detection measures the light run just outside the pattern,
/// so a thin margin costs detection outright rather than just contrast.
const WIDE_QR_QUIET_ZONE: u8 = 4;

/// Gap between the IP address and hostname blocks stacked beside the QR code.
/// The stack stays shorter than the code, so the code sets the header height.
const WIDE_INFO_STACK_GAP: f32 = 20.0;

/// Gap between the QR code and the address stack beside it.
const WIDE_INFO_GAP: f32 = 32.0;

/// Gap between the WiFi signal icon and the SSID beside it.
const WIDE_WIFI_GAP: f32 = 16.0;

/// Size of the SETUP badge in the Large tier's WiFi block.
const WIDE_SETUP_BADGE_SIZE: u32 = 14;

/// Size of the values in the Large tier's info blocks.
const WIDE_INFO_VALUE_SIZE: u32 = 24;

/// Size of the notice lines over the Large tier's top half,
/// a step above its largest text so the notice reads first.
const WIDE_NOTICE_SIZE: u32 = 32;

/// Side of the WiFi signal icon in the Large tier's header.
const WIDE_WIFI_ICON_SIZE: f32 = 32.0;

/// Gap between the Large tier's control rows.
const WIDE_ROW_GAP: f32 = 16.0;

/// Line-height factor the renderer applies to text nodes.
const LINE_H: f32 = 1.4;

/// Size of the address line heading the disc.
const ROUND_HEADER_SIZE: u32 = 18;

/// Gap between the disc's control rows.
const ROUND_ROW_GAP: f32 = 8.0;

/// The disc's station line: the WiFi icon's side, and the text beside it.
/// Keep the icon no taller than the text. Outgrow it and the icon sets the
/// line height, which busts the vertical budget.
const ROUND_WIFI_ICON_SIZE: f32 = 20.0;
const ROUND_WIFI_TEXT_SIZE: u32 = 14;

/// Top edge (px) of the control rows on round panels: below the chord-safe
/// close target, so control and close hit regions are disjoint
/// (hit-testing favors the smaller region).
const ROUND_CONTROLS_TOP: f32 = 142.0;

/// Gap kept below the Wi-Fi info on round panels, so it clears the curved
/// bottom edge instead of sitting flush against it.
const ROUND_BOTTOM_GAP: f32 = 48.0;

/// Gap kept above the header on round panels so the first row clears the
/// curved top edge.
const ROUND_TOP_GAP: f32 = 48.0;

/// Horizontal inset on round panels, keeping content inside the inscribed
/// circle where the usable width is narrower than the full panel.
const ROUND_H_PAD: f32 = 48.0;

/// Usable header width (px) on round panels. The inscribed circle's chord
/// near the top curve is narrower than the full width, so the centered header
/// is budgeted against that chord, not the panel width.
const ROUND_HEADER_WIDTH: f32 = 256.0;

/// What to show in the WiFi/reconfig area of the overlay.
#[derive(Debug, Clone, Copy)]
pub enum WifiView<'a> {
    /// Normal mode: the station info line.
    Idle,
    /// Setup mode: compact row with a SETUP badge and the AP SSID.
    Setup { ap_ssid: &'a str },
}

/// Display panel the overlay is laid out for.
#[derive(Debug, Clone, Copy)]
pub struct Panel {
    pub shape: DisplayShape,
    pub width: u32,
    pub height: u32,
    /// Whether to render the WiFi reconfigure button. On a v2 compositor
    /// `caps.wifi_setup` decides; v1 falls back to the product allowlist,
    /// since reconfiguration needs the setup AP on a mac80211 radio
    /// (BMC100, BFM100), not a BMM board's ESP32 firmware path.
    pub wifi_button: bool,
}

/// Presentation bucket of a Wi-Fi signal reading — which icon it selects.
/// Content-change detection diffs at this granularity so dBm jitter inside a
/// bucket does not count as a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignalBand {
    Problem,
    Strong,
    Fair,
    Low,
}

/// Band for a signal level in dBm (`None` = no reading).
#[must_use]
pub(crate) fn signal_band(dbm: Option<i32>) -> SignalBand {
    match dbm {
        None | Some(0) => SignalBand::Problem,
        Some(level) if level >= -60 => SignalBand::Strong,
        Some(level) if level >= -75 => SignalBand::Fair,
        Some(_) => SignalBand::Low,
    }
}

/// Registered icon ids for each Wi-Fi signal-strength state.
#[derive(Debug, Clone, Copy, Default)]
pub struct WifiIcons {
    pub problem: Option<SvgId>,
    pub low: Option<SvgId>,
    pub fair: Option<SvgId>,
    pub strong: Option<SvgId>,
}

impl WifiIcons {
    /// Icon id for the given Wi-Fi signal level in dBm (`None` = no reading).
    #[must_use]
    pub fn for_signal(&self, dbm: Option<i32>) -> Option<SvgId> {
        match signal_band(dbm) {
            SignalBand::Problem => self.problem,
            SignalBand::Strong => self.strong,
            SignalBand::Fair => self.fair,
            SignalBand::Low => self.low,
        }
    }
}

/// Registered icon ids for the control icons vendored from the stable tray.
#[derive(Debug, Clone, Copy)]
pub struct ControlIcons {
    pub sound_low: Option<SvgId>,
    pub sound_high: Option<SvgId>,
    pub brightness_low: Option<SvgId>,
    pub brightness_high: Option<SvgId>,
    pub night_mode: Option<SvgId>,
    /// Width/height of the night-mode glyph, read from its viewBox (the
    /// only non-square control icon; the host stretches without it).
    pub night_mode_aspect: f32,
    pub restart: Option<SvgId>,
    pub close: Option<SvgId>,
}

impl Default for ControlIcons {
    fn default() -> Self {
        Self {
            sound_low: None,
            sound_high: None,
            brightness_low: None,
            brightness_high: None,
            night_mode: None,
            night_mode_aspect: 1.0,
            restart: None,
            close: None,
        }
    }
}

/// The hold button an action runs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Restart,
    WifiReconfig,
}

impl Action {
    /// The touch key of the button the action runs from.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Action::Restart => RESTART_KEY,
            Action::WifiReconfig => WIFI_RECONFIG_KEY,
        }
    }
}

/// How far a hold action has come.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    /// The finger is down; `progress` is the 0..=1 hold fraction.
    Holding { progress: f32 },
    /// The hold completed and the request is out.
    Pending,
    /// The request was declined or timed out, shown for a moment.
    Failed,
}

/// The action the notice reports on. `reason` is the one bmc gave
/// for declining a restart, while that failure shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status<'a> {
    pub action: Action,
    pub phase: Phase,
    pub reason: Option<&'a str>,
}

/// Night-mode toggle state: whether it is active and the formatted end time
/// (`None` when the schedule is disabled).
#[derive(Debug, Clone, Copy)]
pub struct NightMode<'a> {
    pub active: bool,
    pub until: Option<&'a str>,
}

/// The control surfaces to render. `None` fields are hidden — either the
/// capability is missing (brightness, volume) or the compositor is v1 (night
/// mode, restart). `pressed` is the touch key currently held down, inverting
/// that button's colors.
#[derive(Debug, Clone, Copy, Default)]
pub struct Controls<'a> {
    pub brightness: Option<u8>,
    pub volume: Option<u8>,
    pub night_mode: Option<NightMode<'a>>,
    pub restart: bool,
    pub status: Option<Status<'a>>,
    pub pressed: Option<&'a str>,
}

impl Controls<'_> {
    /// The hold fraction of `action`'s button: zero unless it is being held.
    fn hold_progress(&self, action: Action) -> f32 {
        match self.status {
            Some(Status {
                action: held,
                phase: Phase::Holding { progress },
                ..
            }) if held == action => progress,
            Some(_) | None => 0.0,
        }
    }
}

/// Per-panel control sizing, picked by [`tier_for`] from the panel's width
/// and shape. A labeled tier captions every group; an unlabeled tier renders
/// bare buttons.
#[derive(Debug, Clone, Copy)]
struct Tier {
    circle: f32,
    icon: f32,
    /// Gap inside a ± pair. Equal to `group_gap` on an unlabeled tier:
    /// its bare circles read a tighter gap as uneven spacing,
    /// not as grouping.
    pair_gap: f32,
    group_gap: f32,
    /// Whether each group carries its own label and sublabel.
    labeled: bool,
    /// Fixed widths of a labeled group, so swapping a caption never shifts
    /// the centered row. Unread while `labeled` is false.
    pair_w: f32,
    single_w: f32,
    value_size: u32,
    caption_size: u32,
    /// Inset of the close target from the panel's corner.
    padding: f32,
}

/// Narrowest panel that takes the Deck's labeled layout; everything below is compact.
const WIDE_MIN_WIDTH: u32 = 960;

fn tier_for(panel: &Panel) -> Tier {
    if panel.width >= WIDE_MIN_WIDTH {
        // BMC100.
        Tier {
            circle: 112.0,
            icon: 48.0,
            pair_gap: STEP_GAP_LARGE,
            group_gap: 20.0,
            labeled: true,
            pair_w: LARGE_PAIR_W,
            single_w: LARGE_SINGLE_W,
            value_size: 24,
            caption_size: 20,
            padding: 24.0,
        }
    } else if panel.width <= 320 {
        // BMM100, too narrow to caption three buttons.
        Tier {
            circle: 48.0,
            icon: 22.0,
            pair_gap: 12.0,
            group_gap: 12.0,
            labeled: false,
            pair_w: 0.0,
            single_w: 0.0,
            value_size: 12,
            caption_size: 12,
            padding: 12.0,
        }
    } else if matches!(panel.shape, DisplayShape::Rectangular) {
        // BMM101. The 12pt caption is what keeps the widest label on one
        // line; see `the_widest_label_fits_a_bmm101_group`.
        Tier {
            circle: 64.0,
            icon: 28.0,
            pair_gap: 20.0,
            group_gap: 20.0,
            labeled: true,
            pair_w: LARGE_PAIR_W,
            single_w: BMM101_SINGLE_W,
            value_size: 14,
            caption_size: 12,
            padding: 16.0,
        }
    } else {
        // BFM100. Labeled groups do not fit across the disc,
        // so its chord-safe band takes bare buttons.
        Tier {
            circle: 64.0,
            icon: 28.0,
            pair_gap: 20.0,
            group_gap: 20.0,
            labeled: false,
            pair_w: 0.0,
            single_w: 0.0,
            value_size: 14,
            caption_size: 14,
            padding: 16.0,
        }
    }
}

/// Which arrangement a panel gets, decided once from its shape and width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// The Deck's 1280×480: labeled buttons in one row under an info header.
    Wide,
    /// The BMM100 and BMM101 rectangles: the address table on top,
    /// the brightness slider under it, bare buttons last.
    Compact,
    /// The BFM100's disc: one column inside the chord-safe band.
    Round,
}

fn layout_for(panel: &Panel) -> Layout {
    match panel.shape {
        DisplayShape::Round => Layout::Round,
        DisplayShape::Rectangular if panel.width >= WIDE_MIN_WIDTH => Layout::Wide,
        DisplayShape::Rectangular => Layout::Compact,
    }
}

/// What the tray shows, before a layout decides where.
/// `wifi_button` already carries the hold button's gate: reconfiguration is supported
/// and setup is not running, so setup mode keeps its badge and loses the button.
#[derive(Debug, Clone, Copy)]
struct Content<'a> {
    hostname: Option<&'a str>,
    ip: Option<&'a str>,
    wifi_signal: Option<i32>,
    ssid: &'a str,
    wifi_view: WifiView<'a>,
    wifi_button: bool,
    icons: WifiIcons,
    control_icons: ControlIcons,
    controls: Controls<'a>,
    notice: Notice<'a>,
}

/// The control rows the content calls for, in the row split the tier wants.
fn control_row_nodes(content: Content<'_>, tier: Tier) -> Vec<TreeNode> {
    let (pairs, singles) = control_groups(
        tier,
        &content.controls,
        content.control_icons,
        content.icons,
        content.wifi_button,
    );
    control_rows(tier, pairs, singles)
}

/// Build the overlay UI tree for the current state.
#[must_use]
#[expect(
    clippy::too_many_arguments,
    reason = "overlay state is a flat set of display fields"
)]
pub fn build_tree(
    hostname: Option<&str>,
    ip: Option<&str>,
    wifi_signal: Option<i32>,
    ssid: Option<&str>,
    icons: WifiIcons,
    panel: Panel,
    wifi_view: WifiView<'_>,
    controls_icons: ControlIcons,
    controls: Controls<'_>,
) -> TreeNode {
    let tier = tier_for(&panel);
    let notice = Notice::for_controls(&controls);
    let content = Content {
        hostname,
        ip,
        wifi_signal,
        ssid: ssid.unwrap_or("Not configured"),
        wifi_view,
        wifi_button: panel.wifi_button && matches!(wifi_view, WifiView::Idle),
        icons,
        control_icons: controls_icons,
        controls,
        notice,
    };
    let mut children = match layout_for(&panel) {
        Layout::Wide => wide_children(content, tier),
        Layout::Compact => compact_children(content, panel, tier),
        Layout::Round => round_children(content, panel, tier),
    };
    // Last child: absolute positioning takes it out of flow, and rendering
    // follows child order, so it paints on top of everything.
    children.push(notice.close(close_button(&panel, tier, controls_icons.close)));
    col(
        PropsData {
            background: SCRIM,
            ..PropsData::default()
        },
        children,
    )
}

#[cfg(test)]
mod tests {
    use bmc_render::tree::{PropsData, TreeNode};

    use bmc_wasm_protocol::{SvgId, TextOverflow};

    use super::*;
    use crate::ui::parts::close_origin;
    use crate::ui::test_support::*;

    /// The track's ends are the floor and full brightness, so a thumb at
    /// either end is at a reachable value and no stretch of track is dead.
    #[test]
    fn the_slider_track_spans_the_floor_to_full() {
        assert!(brightness_fraction(MIN_BRIGHTNESS).abs() < f32::EPSILON);
        assert!((brightness_fraction(100) - 1.0).abs() < f32::EPSILON);
        assert_eq!(brightness_from_fraction(0.0), MIN_BRIGHTNESS);
        assert_eq!(brightness_from_fraction(1.0), 100);
    }

    #[test]
    fn brightness_below_the_floor_reads_as_the_floor() {
        assert!(brightness_fraction(0).abs() < f32::EPSILON);
        assert!(brightness_fraction(5).abs() < f32::EPSILON);
    }

    #[test]
    fn a_dragged_fraction_snaps_to_the_grid() {
        for fraction in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let value = brightness_from_fraction(fraction);
            assert_eq!(
                value % BRIGHTNESS_GRID,
                0,
                "{fraction} gave {value}, which is off the grid"
            );
        }
        assert_eq!(brightness_from_fraction(0.5), 55);
    }

    /// Every stop the slider can produce maps back to the fraction that
    /// produced it, so reading a value and dragging to it agree.
    #[test]
    fn every_stop_round_trips_through_the_fraction() {
        let mut stop = MIN_BRIGHTNESS;
        while stop <= 100 {
            assert_eq!(
                brightness_from_fraction(brightness_fraction(stop)),
                stop,
                "stop {stop} did not survive the round trip"
            );
            stop += BRIGHTNESS_GRID;
        }
    }

    #[test]
    fn a_fraction_past_either_end_clamps() {
        assert_eq!(brightness_from_fraction(-0.4), MIN_BRIGHTNESS);
        assert_eq!(brightness_from_fraction(1.7), 100);
    }

    #[test]
    fn layout_follows_shape_then_width() {
        assert_eq!(layout_for(&wide_panel()), Layout::Wide);
        assert_eq!(layout_for(&narrow_panel()), Layout::Compact);
        assert_eq!(layout_for(&small_panel()), Layout::Compact);
        assert_eq!(layout_for(&round_panel()), Layout::Round);
    }

    #[test]
    fn tier_selection_by_panel() {
        assert_circle(&wide_panel(), 112.0);
        assert_circle(&narrow_panel(), 64.0);
        assert_circle(&round_panel(), 64.0);
        assert_circle(&small_panel(), 48.0);
    }

    /// Assert the wide layout's two-equal-flex-halves structure: the control
    /// block's top edge is the vertical middle by construction, both halves'
    /// fixed content fits within its half, and the middle clears the close
    /// target.
    fn assert_wide_halves(
        panel: &Panel,
        tier: Tier,
        setup: bool,
        kids: &[TreeNode],
        close_bottom: f32,
        panel_h: f32,
    ) {
        let [top, bottom, _close] = kids else {
            panic!("{panel:?}: wide root must be two halves + close");
        };
        let (TreeNode::Column(top_props, top_kids), TreeNode::Column(bottom_props, bottom_kids)) =
            (top, bottom)
        else {
            panic!("{panel:?}: halves must be columns");
        };
        assert!(top_props.flex > 0.0, "{panel:?}: halves must flex");
        assert_close(
            top_props.flex,
            bottom_props.flex,
            "equal flex weights pin the middle",
        );
        let mut top_keys = Vec::new();
        canvas_keys(top, &mut top_keys);
        assert!(
            !top_keys.iter().any(|k| is_control_key(k)),
            "{panel:?}: controls live in the bottom half"
        );
        let mut bottom_keys = Vec::new();
        canvas_keys(bottom, &mut bottom_keys);
        assert!(
            bottom_keys.iter().any(|k| is_control_key(k)),
            "{panel:?}: control rows must render"
        );
        let top_h: f32 = top_kids
            .iter()
            .map(|k| expected_flow_height(k, tier, Layout::Wide, setup))
            .sum();
        let bottom_h: f32 = bottom_kids
            .iter()
            .map(|k| expected_flow_height(k, tier, Layout::Wide, setup))
            .sum();
        assert!(
            top_h <= panel_h / 2.0 + 1e-3,
            "{panel:?} setup={setup}: header stack {top_h} overflows its half — \
             min-content would push the controls below the middle"
        );
        assert!(
            bottom_h <= panel_h / 2.0 + 1e-3,
            "{panel:?} setup={setup}: control stack {bottom_h} overflows its half"
        );
        assert!(
            panel_h / 2.0 >= close_bottom - 1e-3,
            "{panel:?}: controls start at the middle, close bottom is {close_bottom}"
        );
    }

    /// Expected height of [`wide_header`], derived from the same constants
    /// the builder uses so the test cannot drift from the layout silently.
    #[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
    fn wide_info_height(setup: bool) -> f32 {
        let header = INFO_HEADER_SIZE as f32 * LINE_H + INFO_HEADER_GAP;
        let wifi_value = (WIDE_INFO_VALUE_SIZE as f32 * LINE_H).max(WIDE_WIFI_ICON_SIZE);
        let wifi = if setup {
            header + wifi_value + 6.0 + INFO_HEADER_SIZE as f32 * LINE_H
        } else {
            header + wifi_value
        };
        let addresses = 2.0 * (header + WIDE_INFO_VALUE_SIZE as f32 * LINE_H) + WIDE_INFO_STACK_GAP;
        // The QR is the tallest child but renders only with a known IP,
        // so folding it in unconditionally bounds the worst case.
        wifi.max(addresses).max(WIDE_QR_SIZE)
    }

    /// Expected height of one flow child of the root column, derived from the
    /// same `Tier` fields the builders use so the test cannot drift from the
    /// layout silently. The flex filler reports 0 (its worst case);
    /// so does anything out of flow, such as the hold notice.
    #[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
    fn expected_flow_height(node: &TreeNode, tier: Tier, layout: Layout, setup: bool) -> f32 {
        let node = undimmed(node);
        if is_absolute(node) {
            return 0.0;
        }
        if let TreeNode::Column(props, kids) = node
            && kids.is_empty()
        {
            return props.height;
        }
        if matches!(node, TreeNode::Spacer { .. }) {
            return 0.0;
        }
        let mut keys = Vec::new();
        canvas_keys(node, &mut keys);
        if keys.iter().any(|k| k == CLOSE_KEY) {
            return 0.0;
        }
        let has_pair = keys.iter().any(|k| PAIR_KEYS.contains(&k.as_str()));
        let has_single = keys.iter().any(|k| SINGLE_KEYS.contains(&k.as_str()));
        // The section the notice hangs off stacks its children like the root does.
        if let TreeNode::Column(_, kids) = node
            && kids.len() > 1
            && !(has_pair || has_single)
        {
            return kids
                .iter()
                .map(|k| expected_flow_height(k, tier, layout, setup))
                .sum();
        }
        if has_pair || has_single {
            let value_gap = if tier.labeled { 8.0 } else { 2.0 };
            let pair_h = tier.circle
                + value_gap
                + tier.value_size as f32 * LINE_H
                + if tier.labeled {
                    tier.caption_size as f32 * LINE_H
                } else {
                    0.0
                };
            let single_h = tier.circle
                + if tier.labeled {
                    8.0 + 2.0 * tier.caption_size as f32 * LINE_H
                } else {
                    0.0
                };
            return match (has_pair, has_single) {
                (true, true) => pair_h.max(single_h),
                (true, false) => pair_h,
                (false, true) => single_h,
                (false, false) => unreachable!(),
            };
        }
        if has_unkeyed_canvas(node) {
            return if layout == Layout::Wide {
                wide_info_height(setup)
            } else {
                ROUND_WIFI_ICON_SIZE.max(ROUND_WIFI_TEXT_SIZE as f32 * LINE_H)
            };
        }
        let size = max_text_size(node);
        if size > 0 {
            return size as f32 * LINE_H;
        }
        0.0
    }

    /// Holds included: the notice wraps the section above the rows
    /// without moving them.
    #[test]
    fn controls_start_below_the_close_target() {
        let long_ssid = "An-Extremely-Long-Setup-Network-Name-420";
        assert_eq!(long_ssid.chars().count(), 40);
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for ((view, setup), controls) in [
                (WifiView::Idle, false),
                (WifiView::Setup { ap_ssid: long_ssid }, true),
            ]
            .into_iter()
            .flat_map(|view| [(view, all_controls()), (view, held_controls())])
            {
                let tier = tier_for(&panel);
                let tree = build_tree(
                    Some("braiins-deck"),
                    Some("10.0.0.2"),
                    Some(-55),
                    Some("MyWifi"),
                    WifiIcons::default(),
                    panel,
                    view,
                    ControlIcons::default(),
                    controls,
                );
                let kids = children(&tree).expect("BUG: root must be a container");

                let close_bottom = close_origin(&panel, tier).1 + CLOSE_TARGET;
                #[expect(clippy::cast_precision_loss, reason = "panel sizes are small")]
                let panel_h = panel.height as f32;
                if layout_for(&panel) == Layout::Wide {
                    assert_wide_halves(&panel, tier, setup, kids, close_bottom, panel_h);
                    continue;
                }

                let mut before_controls = 0.0;
                let mut total = 0.0;
                let mut seen_controls = false;
                for kid in kids {
                    let mut keys = Vec::new();
                    canvas_keys(kid, &mut keys);
                    if keys.iter().any(|k| is_control_key(k)) {
                        seen_controls = true;
                    }
                    if !seen_controls {
                        before_controls +=
                            expected_flow_height(kid, tier, layout_for(&panel), setup);
                    }
                    total += expected_flow_height(kid, tier, layout_for(&panel), setup);
                }
                assert!(seen_controls, "{panel:?}: control rows must render");
                assert!(
                    before_controls >= close_bottom - 1e-3,
                    "{panel:?} setup={setup}: controls start at {before_controls}, \
                     close bottom is {close_bottom}"
                );
                assert!(
                    total <= panel_h + 1e-3,
                    "{panel:?} setup={setup}: fixed stack {total} overflows {panel_h} — \
                     flex would shrink the pinned spacers and drag rows over the close target"
                );
            }
        }
    }

    /// The rectangular layouts print a QR beside the address.
    /// The disc has no room for one, and none of them renders a QR
    /// without an IP to encode.
    #[test]
    fn qr_encodes_the_ip_url_on_the_rectangular_layouts() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for ip in [Some("10.0.0.2"), None] {
                let tree = build_tree(
                    Some("braiins-deck"),
                    ip,
                    Some(-55),
                    Some("MyWifi"),
                    WifiIcons::default(),
                    panel,
                    WifiView::Idle,
                    ControlIcons::default(),
                    all_controls(),
                );
                let mut qrs = Vec::new();
                qr_texts(&tree, &mut qrs);
                let expected: Vec<String> = match layout_for(&panel) {
                    Layout::Wide | Layout::Compact => {
                        ip.map(|ip| format!("http://{ip}")).into_iter().collect()
                    }
                    Layout::Round => Vec::new(),
                };
                assert_eq!(qrs, expected, "{panel:?} ip={ip:?}");
            }
        }
    }

    /// Every layout shows the address, falling back to `---`.
    /// The disc is the only one that drops the hostname; the other two
    /// have room to label both.
    #[test]
    fn every_layout_shows_the_address_and_only_the_disc_drops_the_hostname() {
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for ip in [Some("10.0.0.2"), Some("255.255.255.255"), None] {
                let tree = build_tree(
                    Some("braiins-deck"),
                    ip,
                    Some(-55),
                    Some("MyWifi"),
                    WifiIcons::default(),
                    panel,
                    WifiView::Idle,
                    ControlIcons::default(),
                    all_controls(),
                );
                let mut texts = Vec::new();
                collect_texts(&tree, &mut texts);
                let address = ip.unwrap_or(NO_DATA_PLACEHOLDER);

                assert!(
                    texts.iter().any(|t| t == address),
                    "{panel:?} ip={ip:?}: the address must render somewhere"
                );
                assert_eq!(
                    texts.iter().any(|t| t == "braiins-deck"),
                    layout_for(&panel) != Layout::Round,
                    "{panel:?} ip={ip:?}: only the disc is too tight to keep \
                     the hostname alongside the address"
                );
            }
        }
    }

    #[test]
    fn close_and_hold_circles_are_the_only_out_of_flow_canvases() {
        fn absolute_canvases<'t>(
            node: &'t TreeNode,
            out: &mut Vec<(&'t PropsData, Option<&'t str>)>,
        ) {
            if let TreeNode::Canvas {
                props, touch_key, ..
            } = node
                && props.is_absolute()
            {
                out.push((props, touch_key.as_deref()));
            }
            if let Some(kids) = children(node) {
                for k in kids {
                    absolute_canvases(k, out);
                }
            }
        }
        fn hold_circle_columns(node: &TreeNode) -> usize {
            let own = usize::from(matches!(
                node,
                TreeNode::Column(_, kids) if matches!(
                    kids.as_slice(),
                    [
                        TreeNode::Canvas { props, touch_key: None, .. },
                        TreeNode::Canvas { touch_key: Some(_), .. },
                    ] if props.is_absolute()
                )
            ));
            own + children(node)
                .into_iter()
                .flatten()
                .map(hold_circle_columns)
                .sum::<usize>()
        }
        let controls = held_controls();
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_with_controls(panel, controls);
            let mut absolute = Vec::new();
            absolute_canvases(&tree, &mut absolute);
            let (keyed, hold_circles) = absolute
                .into_iter()
                .partition::<Vec<_>, _>(|(_, key)| key.is_some());
            assert_eq!(
                keyed.len(),
                1,
                "{panel:?}: the close target is the only out-of-flow touchable"
            );
            assert_eq!(keyed[0].1, Some(CLOSE_KEY));
            assert!(
                !hold_circles.is_empty(),
                "{panel:?}: held buttons must carry progress circles"
            );
            for (props, _) in &hold_circles {
                let expected_outset = tier_for(&panel).circle / 2.0;
                assert_close(
                    props.inset_top,
                    -expected_outset,
                    "hold canvas outset above its button",
                );
                assert_close(
                    props.inset_left,
                    -expected_outset,
                    "hold canvas outset left of its button",
                );
            }
            assert_eq!(
                hold_circles.len(),
                hold_circle_columns(&tree),
                "{panel:?}: every keyless out-of-flow canvas is a hold circle \
                 layered behind its keyed button"
            );

            let kids = children(&tree).expect("BUG: root must be a container");
            let last = undimmed(kids.last().expect("BUG: root must have children"));
            assert!(
                matches!(
                    last,
                    TreeNode::Canvas { touch_key: Some(k), .. } if k == CLOSE_KEY
                ),
                "{panel:?}: the close canvas is the last root child so it paints on top"
            );
        }
    }

    #[test]
    fn round_panel_roots_a_column() {
        assert!(matches!(
            build(round_panel(), WifiView::Idle),
            TreeNode::Column(..)
        ));
    }

    #[test]
    fn wide_and_narrow_both_root_a_column() {
        assert!(matches!(
            build(wide_panel(), WifiView::Idle),
            TreeNode::Column(..)
        ));
        assert!(matches!(
            build(narrow_panel(), WifiView::Idle),
            TreeNode::Column(..)
        ));
    }

    fn distinct_icons() -> WifiIcons {
        let id = |raw| SvgId::from_wire(raw).expect("BUG: test SvgId must be non-zero");
        WifiIcons {
            problem: Some(id(1)),
            low: Some(id(2)),
            fair: Some(id(3)),
            strong: Some(id(4)),
        }
    }

    #[test]
    fn no_reading_is_a_problem() {
        assert_eq!(distinct_icons().for_signal(None), distinct_icons().problem);
    }

    #[test]
    fn zero_dbm_is_a_problem() {
        assert_eq!(
            distinct_icons().for_signal(Some(0)),
            distinct_icons().problem
        );
    }

    #[test]
    fn signal_band_thresholds() {
        let icons = distinct_icons();
        assert_eq!(icons.for_signal(Some(-50)), icons.strong);
        assert_eq!(icons.for_signal(Some(-60)), icons.strong);
        assert_eq!(icons.for_signal(Some(-61)), icons.fair);
        assert_eq!(icons.for_signal(Some(-75)), icons.fair);
        assert_eq!(icons.for_signal(Some(-76)), icons.low);
        assert_eq!(icons.for_signal(Some(-90)), icons.low);
    }

    #[test]
    fn signal_band_maps_dbm_to_icon_buckets() {
        assert_eq!(signal_band(None), SignalBand::Problem);
        assert_eq!(signal_band(Some(0)), SignalBand::Problem);
        assert_eq!(signal_band(Some(-59)), SignalBand::Strong);
        assert_eq!(signal_band(Some(-60)), SignalBand::Strong);
        assert_eq!(signal_band(Some(-61)), SignalBand::Fair);
        assert_eq!(signal_band(Some(-75)), SignalBand::Fair);
        assert_eq!(signal_band(Some(-76)), SignalBand::Low);
    }

    // Locks the dirtying intent: jitter inside one bucket compares equal (no
    // repaint), a bucket crossing compares unequal (repaint). This is the
    // exact comparison refresh_network_if_due performs; the overlay-level
    // path is not unit-tested because it walks getifaddrs and spawns uci.
    #[test]
    fn jitter_within_a_band_is_not_a_change_but_a_crossing_is() {
        assert_eq!(signal_band(Some(-65)), signal_band(Some(-70)));
        assert_eq!(signal_band(Some(-45)), signal_band(Some(-59)));
        assert_ne!(signal_band(Some(-59)), signal_band(Some(-61)));
        assert_ne!(signal_band(Some(-75)), signal_band(Some(-76)));
        assert_ne!(signal_band(Some(-65)), signal_band(None));
    }

    /// Runtime strings reach the renderer whole and are ellipsized there,
    /// at their slot's width or at a cap where nothing else bounds them.
    #[test]
    fn long_runtime_strings_are_left_for_the_renderer_to_cut() {
        let hostname = "braiins-deck-".repeat(6);
        let ssid = "a-network-name-".repeat(5);
        let ip = "10.0.0.2";
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_tree(
                Some(&hostname),
                Some(ip),
                Some(-55),
                Some(&ssid),
                WifiIcons::default(),
                panel,
                WifiView::Idle,
                ControlIcons::default(),
                Controls::default(),
            );
            let large = layout_for(&panel) == Layout::Wide;

            let ssid_style = style_of(&tree, &ssid)
                .unwrap_or_else(|| panic!("{panel:?}: the SSID must reach the tree whole"));
            assert_eq!(
                ssid_style.text_overflow,
                TextOverflow::Ellipsis,
                "{panel:?}"
            );
            if large {
                let hostname_style =
                    style_of(&tree, &hostname).expect("BUG: the wide tier shows the hostname");
                assert_eq!(
                    (hostname_style.max_width, hostname_style.text_overflow),
                    (WIDE_HOSTNAME_WIDTH, TextOverflow::Ellipsis)
                );
                assert_eq!(ssid_style.max_width, WIDE_SSID_WIDTH);
            } else {
                let header_style = style_of(&tree, ip)
                    .unwrap_or_else(|| panic!("{panel:?}: the address heads the panel"));
                assert_eq!(
                    header_style.text_overflow,
                    TextOverflow::Ellipsis,
                    "{panel:?}"
                );
            }
        }
    }
}
