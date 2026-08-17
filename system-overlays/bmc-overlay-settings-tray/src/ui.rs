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
use bmc_render::tree::{
    DrawCommand, PropsData, TextStyle, TreeNode, col, fixed_height, row, spacer, text,
};
use bmc_wasm_protocol::colors::{BLACK, GRAY_50, GREEN_50, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{
    ArcCap, ArcFill, ArcSegments, Color, CrossAlign, Fill, FontWeight, SvgId, TextAlign,
};

/// Stable touch key for the WiFi reconfiguration hold button.
pub const WIFI_RECONFIG_KEY: &str = "wifi_reconfig";

/// Stable touch key for the bare WiFi reconnect hold button.
pub const WIFI_RECONNECT_KEY: &str = "wifi_reconnect";

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

/// Brightness floor: the step buttons never dim the panel below this.
pub const MIN_BRIGHTNESS: u8 = 10;

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

/// Hold-progress ring color; its alpha grows with the hold fraction.
const HOLD_RING: Color = Color::from_rgba(0x8B, 0x7C, 0xFF, 255);

/// Stroke width of the hold-progress ring at full hold.
const RING_W: f32 = 12.0;

/// Stroke width of the hold-progress ring while idle.
const RING_MIN_W: f32 = 3.0;

/// Ring alpha while idle; grows to fully opaque at full hold.
const RING_MIN_ALPHA: f32 = 0.3;

/// Edge length of the square close touch target.
const CLOSE_TARGET: f32 = 48.0;

/// Edge length of the close glyph inside its touch target.
const CLOSE_GLYPH: f32 = 24.0;

/// Gap between the two buttons of a ± pair on the Large tier.
const STEP_GAP_LARGE: f32 = 12.0;

/// Fixed text-block widths on the Large tier so caption swaps never shift
/// the centered-row math (bmc-render cannot ellipsize; strings are fitted).
const LARGE_PAIR_W: f32 = 236.0;
const LARGE_SINGLE_W: f32 = 160.0;

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

/// Fit budgets for the runtime strings in the Large tier's info blocks.
const WIDE_HOSTNAME_WIDTH: f32 = 320.0;
const WIDE_SSID_WIDTH: f32 = 400.0;

/// Edge length of the IP QR code, and of the canvas it is drawn on.
/// An `http://<ipv4>` payload fits the 26 bytes a version-2 symbol holds
/// at the renderer's ECC level, so the grid is always 25 modules
/// plus the quiet zone. That puts a module at ~4.4px — 0.51mm
/// at the panel's 217 DPI, well clear of what a phone camera resolves.
const WIDE_QR_SIZE: f32 = 144.0;

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

/// Line-height factor the renderer applies to text nodes.
const LINE_H: f32 = 1.4;

/// Top edge (px) of the control rows on round panels: below the chord-safe
/// close target, so control and close hit regions are disjoint
/// (hit-testing favors the smaller region).
const ROUND_CONTROLS_TOP: f32 = 142.0;

/// Gap kept below the Wi-Fi info on round panels, so it clears the curved
/// bottom edge instead of sitting flush against it.
const ROUND_BOTTOM_GAP: f32 = 48.0;

/// Gap kept above the hostname on round panels so the first row clears the
/// curved top edge.
const ROUND_TOP_GAP: f32 = 48.0;

/// Horizontal inset on round panels, keeping content inside the inscribed
/// circle where the usable width is narrower than the full panel.
const ROUND_H_PAD: f32 = 48.0;

/// Usable hostname width (px) on round panels. Near the top curve the
/// inscribed circle's chord is narrower than the full width, so the centered
/// hostname is budgeted against this chord rather than the panel width.
const ROUND_HOSTNAME_WIDTH: f32 = 256.0;

/// Conservative per-glyph advance (px) for the 24px bold hostname font,
/// rounded up from the measured Braiins Sans bold width so the character
/// budget never under-counts and lets a string overflow its row. The
/// renderer has no single-line ellipsis, so the fit is enforced on the
/// string in [`fit_line`] instead.
const HOSTNAME_CHAR_W: f32 = 16.0;

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
    /// Whether to render the WiFi reconfigure/reconnect buttons. WiFi
    /// reconfiguration is only supported on boards whose AP runs over the
    /// mac80211 radio (BMC100, BFM100); the BMM boards drive their ESP32 AP
    /// through a separate firmware path the overlay does not implement, so the
    /// buttons are hidden there.
    pub wifi_buttons: bool,
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

/// Dynamic state of one hold-to-confirm control: its shared-caption text (the
/// FSM caption, `None` when resting) and the 0..=1 hold fraction for the ring.
#[derive(Debug, Clone, Copy, Default)]
pub struct HoldControl<'a> {
    pub caption: Option<&'a str>,
    pub progress: f32,
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
    pub restart: Option<HoldControl<'a>>,
    pub wifi_reconfig: HoldControl<'a>,
    pub wifi_reconnect: HoldControl<'a>,
    pub pressed: Option<&'a str>,
}

/// Per-panel control sizing. Selected by panel width in [`tier_for`]: the
/// Large tier (BMC100) adds static text blocks under every group; the
/// medium/small tiers render bare buttons with a shared caption line.
#[derive(Debug, Clone, Copy)]
struct Tier {
    circle: f32,
    icon: f32,
    pair_gap: f32,
    group_gap: f32,
    large_text: bool,
    value_size: u32,
    caption_size: u32,
    hostname_size: u32,
    wifi_text_size: u32,
    /// Height/width of the WiFi signal icon in the info section — must not
    /// exceed the info line's text height on the compact tiers or it, not
    /// the text, sets the line height and busts the vertical budget.
    wifi_icon_size: f32,
    padding: f32,
    row_gap: f32,
}

fn tier_for(panel: &Panel) -> Tier {
    if panel.width >= 960 {
        Tier {
            circle: 112.0,
            icon: 48.0,
            pair_gap: STEP_GAP_LARGE,
            group_gap: 20.0,
            large_text: true,
            value_size: 24,
            caption_size: 20,
            hostname_size: 24,
            wifi_text_size: 22,
            wifi_icon_size: 32.0,
            padding: 24.0,
            row_gap: 16.0,
        }
    } else if panel.width <= 320 {
        Tier {
            circle: 48.0,
            icon: 22.0,
            pair_gap: 6.0,
            group_gap: 12.0,
            large_text: false,
            value_size: 12,
            caption_size: 12,
            hostname_size: 16,
            wifi_text_size: 12,
            wifi_icon_size: 17.0,
            padding: 12.0,
            row_gap: 6.0,
        }
    } else {
        Tier {
            circle: 64.0,
            icon: 28.0,
            pair_gap: 8.0,
            group_gap: 20.0,
            large_text: false,
            value_size: 14,
            caption_size: 14,
            hostname_size: 18,
            wifi_text_size: 14,
            wifi_icon_size: 20.0,
            padding: 16.0,
            row_gap: 8.0,
        }
    }
}

/// Generic width-fitting for single-line UI strings: a conservative per-glyph
/// budget scaled from the 24px hostname advance. The renderer cannot
/// ellipsize, so overlong strings are truncated here.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "max_chars is a small non-negative count; text sizes are small"
)]
fn fit_line(s: &str, width_px: f32, size: u32) -> String {
    let glyph = HOSTNAME_CHAR_W * (size as f32) / 24.0;
    let max_chars = (width_px / glyph).floor().max(1.0) as usize;
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let kept: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn text_style(size: u32, color: Color) -> TextStyle {
    TextStyle {
        size,
        color,
        ..TextStyle::default()
    }
}

fn fixed_width(width: f32) -> TreeNode {
    row(
        PropsData {
            width,
            ..PropsData::default()
        },
        Vec::new(),
    )
}

/// Wrap `node` with equal left/right padding by flanking it with fixed-width
/// spacers; the content takes the remaining width.
fn pad_horizontal(node: TreeNode, padding: f32) -> TreeNode {
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![
            fixed_width(padding),
            col(
                PropsData {
                    flex: 1.0,
                    ..PropsData::default()
                },
                vec![node],
            ),
            fixed_width(padding),
        ],
    )
}

/// Centered hostname header row. The string is pre-fitted by [`fit_line`],
/// so it is laid out on a single line without wrapping. The centering column
/// is what actually centers the text node; a bare paragraph with
/// `align: Center` does not center under a stretching parent.
fn hostname_row(hostname: &str, size: u32) -> TreeNode {
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![text(
            hostname,
            TextStyle {
                size,
                weight: FontWeight::BOLD,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        )],
    )
}

fn wifi_icon(icons: WifiIcons, wifi_signal: Option<i32>, size: f32) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: size,
            height: size,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Svg {
            x: 0.0,
            y: 0.0,
            w: size,
            h: size,
            color: TRANSPARENT,
            icon_id: icons.for_signal(wifi_signal),
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// An icon inside a round button. `aspect` is width/height — 1.0 for the
/// square control glyphs; nightmode.svg is 49×48 and keeps its ratio
/// instead of stretching.
#[derive(Debug, Clone, Copy)]
struct ButtonIcon {
    id: Option<SvgId>,
    aspect: f32,
}

impl ButtonIcon {
    fn square(id: Option<SvgId>) -> Self {
        Self { id, aspect: 1.0 }
    }
}

/// One round icon button: a filled circle, an optional full-circle hold ring
/// whose width and alpha grow with the hold progress, and a centered icon.
/// `icon_tint`
/// `TRANSPARENT` keeps the SVG's native fill (white for controls); an opaque
/// tint colorizes the whole icon.
fn round_button(
    key: &str,
    icon: ButtonIcon,
    diameter: f32,
    icon_size: f32,
    fill: Color,
    icon_tint: Color,
    ring_progress: Option<f32>,
) -> TreeNode {
    let c = diameter / 2.0;
    // Hold buttons keep the stable inner-circle-inside-the-ring geometry
    // (88px fill inside the 112px ring, leaving room for the full-hold ring
    // width); ringless buttons fill the whole tier diameter.
    let fill_r = if ring_progress.is_some() {
        c - RING_W
    } else {
        c
    };
    let mut draws = vec![DrawCommand::Circle {
        cx: c,
        cy: c,
        r: fill_r,
        fill: Fill::Solid(fill),
    }];
    if let Some(p) = ring_progress {
        // The ring starts on the fill radius and grows outward; at full hold
        // its outer edge reaches the button rim.
        let ring_w = RING_MIN_W + (RING_W - RING_MIN_W) * p;
        let ring_r = fill_r + (RING_W / 2.0) * p;
        let alpha = RING_MIN_ALPHA + (1.0 - RING_MIN_ALPHA) * p;
        draws.push(DrawCommand::Arc {
            cx: c,
            cy: c,
            radius: ring_r,
            start_angle: 0.0,
            end_angle: std::f32::consts::TAU,
            width: ring_w,
            fill: ArcFill::Solid(HOLD_RING.scale_alpha(alpha)),
            segments: ArcSegments::Continuous,
            cap: ArcCap::Butt,
        });
    }
    let (icon_w, icon_h) = (icon_size * icon.aspect, icon_size);
    draws.push(DrawCommand::Svg {
        x: c - icon_w / 2.0,
        y: c - icon_h / 2.0,
        w: icon_w,
        h: icon_h,
        color: icon_tint,
        icon_id: icon.id,
        anti_alias: true,
        fills: Vec::new(),
    });
    TreeNode::Canvas {
        props: PropsData {
            width: diameter,
            height: diameter,
            ..PropsData::default()
        },
        touch_key: Some(key.to_owned()),
        draws,
    }
}

fn press_fill(pressed: bool) -> Color {
    if pressed { CIRCLE_PRESSED } else { CIRCLE_FILL }
}

fn press_tint(pressed: bool) -> Color {
    if pressed {
        ICON_PRESSED_TINT
    } else {
        TRANSPARENT
    }
}

/// A ±step pair (volume / brightness) with its value text below. On the Large
/// tier the block is a fixed-width column with bold value + gray name.
#[expect(
    clippy::too_many_arguments,
    reason = "flat display fields, same as build_tree"
)]
fn pair_group(
    tier: Tier,
    down_key: &'static str,
    up_key: &'static str,
    low_icon: Option<SvgId>,
    high_icon: Option<SvgId>,
    value: u8,
    name: &str,
    pressed: Option<&str>,
) -> TreeNode {
    let btn = |key: &'static str, icon: Option<SvgId>| {
        let p = pressed == Some(key);
        round_button(
            key,
            ButtonIcon::square(icon),
            tier.circle,
            tier.icon,
            press_fill(p),
            press_tint(p),
            None,
        )
    };
    let buttons = row(
        PropsData {
            gap: tier.pair_gap,
            ..PropsData::default()
        },
        vec![btn(down_key, low_icon), btn(up_key, high_icon)],
    );
    let mut kids = vec![
        buttons,
        fixed_height(if tier.large_text { 8.0 } else { 2.0 }),
    ];
    if tier.large_text {
        kids.push(text(
            format!("{value}"),
            TextStyle {
                size: tier.value_size,
                weight: FontWeight::BOLD,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
        kids.push(text(
            name,
            TextStyle {
                size: tier.caption_size,
                color: GRAY_50,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
    } else {
        kids.push(text(
            format!("{value}"),
            TextStyle {
                size: tier.value_size,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
    }
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            width: if tier.large_text { LARGE_PAIR_W } else { 0.0 },
            ..PropsData::default()
        },
        kids,
    )
}

/// A single-button group. Large tier adds the fixed-width label/sublabel
/// block; other tiers render the bare button.
#[expect(
    clippy::too_many_arguments,
    reason = "flat display fields, same as build_tree"
)]
fn single_group(
    tier: Tier,
    key: &'static str,
    icon: ButtonIcon,
    fill: Color,
    tint: Color,
    ring: Option<f32>,
    label: &str,
    sublabel: &str,
) -> TreeNode {
    let btn = round_button(key, icon, tier.circle, tier.icon, fill, tint, ring);
    if !tier.large_text {
        return btn;
    }
    // The label/sublabel copy is fixed at compile time ("Night Mode: Off",
    // "hold 5 seconds", …) and sized to its column; the conservative
    // `fit_line` glyph budget would truncate it, so it is rendered verbatim.
    let mut kids = vec![btn, fixed_height(8.0)];
    kids.push(text(
        label,
        TextStyle {
            size: tier.caption_size,
            weight: FontWeight::BOLD,
            color: WHITE,
            align: TextAlign::Center,
            ..TextStyle::default()
        },
    ));
    if !sublabel.is_empty() {
        kids.push(text(
            sublabel,
            TextStyle {
                size: tier.caption_size,
                color: GRAY_50,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        ));
    }
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            width: LARGE_SINGLE_W,
            ..PropsData::default()
        },
        kids,
    )
}

/// All control groups in spec order, split into the ± pair groups
/// (volume/brightness) and the single-button groups. On the Large tier the
/// two halves concatenate into one row; medium/small render them as two rows.
/// `wifi` is true only when the WiFi buttons apply (product/caps gate, not
/// in setup mode).
fn control_groups(
    tier: Tier,
    controls: &Controls<'_>,
    icons: ControlIcons,
    wifi_icons: WifiIcons,
    wifi: bool,
) -> (Vec<TreeNode>, Vec<TreeNode>) {
    let mut pairs = Vec::new();
    if let Some(v) = controls.volume {
        pairs.push(pair_group(
            tier,
            VOLUME_DOWN_KEY,
            VOLUME_UP_KEY,
            icons.sound_low,
            icons.sound_high,
            v,
            "Volume",
            controls.pressed,
        ));
    }
    if let Some(b) = controls.brightness {
        pairs.push(pair_group(
            tier,
            BRIGHTNESS_DOWN_KEY,
            BRIGHTNESS_UP_KEY,
            icons.brightness_low,
            icons.brightness_high,
            b,
            "Brightness",
            controls.pressed,
        ));
    }

    let mut singles = Vec::new();
    if let Some(night) = controls.night_mode {
        // The boundary reads as "current state lasts until HH:MM" in both
        // states: the end of the night window while active, its next start
        // while inactive (absent when the schedule is disabled).
        let sublabel = night
            .until
            .map_or_else(String::new, |until| format!("Until {until}"));
        singles.push(single_group(
            tier,
            NIGHT_MODE_KEY,
            ButtonIcon {
                id: icons.night_mode,
                aspect: icons.night_mode_aspect,
            },
            if night.active {
                NIGHT_ACTIVE
            } else {
                CIRCLE_FILL
            },
            TRANSPARENT,
            None,
            if night.active {
                "Night Mode: On"
            } else {
                "Night Mode: Off"
            },
            &sublabel,
        ));
    }
    if let Some(restart) = controls.restart {
        let p = controls.pressed == Some(RESTART_KEY);
        singles.push(single_group(
            tier,
            RESTART_KEY,
            ButtonIcon::square(icons.restart),
            press_fill(p),
            press_tint(p),
            Some(restart.progress),
            "Restart",
            "hold 5 seconds",
        ));
    }
    if wifi {
        let p = controls.pressed == Some(WIFI_RECONFIG_KEY);
        singles.push(single_group(
            tier,
            WIFI_RECONFIG_KEY,
            ButtonIcon::square(wifi_icons.problem),
            press_fill(p),
            press_tint(p),
            Some(controls.wifi_reconfig.progress),
            "Reset WiFi",
            "hold 3 seconds",
        ));
        let p = controls.pressed == Some(WIFI_RECONNECT_KEY);
        singles.push(single_group(
            tier,
            WIFI_RECONNECT_KEY,
            ButtonIcon::square(wifi_icons.strong),
            press_fill(p),
            press_tint(p),
            Some(controls.wifi_reconnect.progress),
            "Reconnect",
            "hold 3 seconds",
        ));
    }
    (pairs, singles)
}

/// The control row nodes: one row (pairs + singles) on the Large tier, two
/// rows ([pairs], [singles]) on medium/small, each row centered. Empty rows
/// are dropped entirely.
fn control_rows(tier: Tier, pairs: Vec<TreeNode>, singles: Vec<TreeNode>) -> Vec<TreeNode> {
    let centered = |groups: Vec<TreeNode>| {
        col(
            PropsData {
                cross_align: CrossAlign::Center,
                ..PropsData::default()
            },
            vec![row(
                PropsData {
                    gap: tier.group_gap,
                    cross_align: CrossAlign::Start,
                    ..PropsData::default()
                },
                groups,
            )],
        )
    };
    if tier.large_text {
        let mut groups = pairs;
        groups.extend(singles);
        if groups.is_empty() {
            return Vec::new();
        }
        return vec![centered(groups)];
    }
    [pairs, singles]
        .into_iter()
        .filter(|groups| !groups.is_empty())
        .map(centered)
        .collect()
}

/// Dynamic status line under the control rows. Precedence: restart >
/// reconfigure > reconnect > night-mode-until (medium/small only) > none.
/// Prefixed with the control name so unlabeled small-tier buttons stay
/// attributable.
fn shared_caption(tier: Tier, controls: &Controls<'_>, usable: f32) -> Option<TreeNode> {
    let raw = if let Some(c) = controls.restart.and_then(|r| r.caption) {
        Some(format!("Restart: {c}"))
    } else if let Some(c) = controls.wifi_reconfig.caption {
        Some(format!("Reset WiFi: {c}"))
    } else if let Some(c) = controls.wifi_reconnect.caption {
        Some(format!("Reconnect: {c}"))
    } else if !tier.large_text
        && let Some(n) = controls.night_mode
        && let Some(until) = n.until
    {
        Some(if n.active {
            format!("Night mode on until {until}")
        } else {
            format!("Night mode off until {until}")
        })
    } else {
        None
    };
    raw.map(|s| {
        col(
            PropsData {
                cross_align: CrossAlign::Center,
                ..PropsData::default()
            },
            vec![text(
                fit_line(&s, usable, tier.caption_size),
                TextStyle {
                    size: tier.caption_size,
                    color: GRAY_50,
                    align: TextAlign::Center,
                    ..TextStyle::default()
                },
            )],
        )
    })
}

/// Top-left corner of the absolutely positioned close target, in panel
/// coordinates. Shared by `close_button` and the hit-disjointness test:
/// top-right padded corner on rectangular panels; on round panels centered
/// on the 45° point of the disc inset by 56px, which is chord-safe (farthest
/// corner ≈218px < R = 240).
fn close_origin(panel: &Panel, tier: Tier) -> (f32, f32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "panel sizes are far below f32 mantissa precision"
    )]
    let w = panel.width as f32;
    match panel.shape {
        DisplayShape::Rectangular => (w - tier.padding - CLOSE_TARGET, tier.padding),
        DisplayShape::Round => {
            let r = w / 2.0;
            let d = (r - 56.0) * std::f32::consts::FRAC_1_SQRT_2;
            (r + d - CLOSE_TARGET / 2.0, r - d - CLOSE_TARGET / 2.0)
        }
    }
}

/// The 48×48 close target with its 24×24 gray glyph, absolutely positioned
/// via `PropsData` insets (finite inset = absolute positioning).
fn close_button(panel: &Panel, tier: Tier, icon: Option<SvgId>) -> TreeNode {
    let (left, top) = close_origin(panel, tier);
    let glyph_inset = (CLOSE_TARGET - CLOSE_GLYPH) / 2.0;
    TreeNode::Canvas {
        props: PropsData {
            width: CLOSE_TARGET,
            height: CLOSE_TARGET,
            inset_top: top,
            inset_left: left,
            ..PropsData::default()
        },
        touch_key: Some(CLOSE_KEY.to_owned()),
        draws: vec![DrawCommand::Svg {
            x: glyph_inset,
            y: glyph_inset,
            w: CLOSE_GLYPH,
            h: CLOSE_GLYPH,
            color: TRANSPARENT,
            icon_id: icon,
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// One Large-tier info block: a small gray header over a white value node.
fn info_block(header: &'static str, value: TreeNode) -> TreeNode {
    col(
        PropsData {
            gap: INFO_HEADER_GAP,
            ..PropsData::default()
        },
        vec![text(header, text_style(INFO_HEADER_SIZE, GRAY_50)), value],
    )
}

/// The QR code for the Deck's web UI, so scanning it opens the address
/// printed beside it. Omitted while the IP is unknown: a placeholder
/// would scan as a dead link.
fn ip_qr(ip: &str) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: WIDE_QR_SIZE,
            height: WIDE_QR_SIZE,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Qr {
            x: 0.0,
            y: 0.0,
            size: WIDE_QR_SIZE,
            dark: BLACK,
            light: WHITE,
            quiet_zone: WIDE_QR_QUIET_ZONE,
            text: format!("http://{ip}"),
        }],
    }
}

/// The Large tier's top info section: the QR code in the left corner,
/// the IP address and hostname stacked beside it,
/// the WiFi connection block in the right corner, nothing in between —
/// each text pair a gray header over a 24px value.
/// In setup mode the WiFi block carries the SETUP badge, the AP SSID,
/// and the join hint instead of the station info.
fn wide_header(
    hostname: &str,
    ip: Option<&str>,
    icons: WifiIcons,
    wifi_signal: Option<i32>,
    ssid: &str,
    tier: Tier,
    wifi_view: WifiView<'_>,
) -> TreeNode {
    let value_size = tier.hostname_size;
    let wifi_block = match wifi_view {
        WifiView::Idle => info_block(
            "WiFi Connection",
            row(
                PropsData {
                    cross_align: CrossAlign::Center,
                    gap: WIDE_WIFI_GAP,
                    ..PropsData::default()
                },
                vec![
                    wifi_icon(icons, wifi_signal, tier.wifi_icon_size),
                    text(
                        fit_line(ssid, WIDE_SSID_WIDTH, value_size),
                        text_style(value_size, WHITE),
                    ),
                ],
            ),
        ),
        WifiView::Setup { ap_ssid } => info_block(
            "WiFi Connection",
            col(
                PropsData {
                    gap: 6.0,
                    ..PropsData::default()
                },
                vec![
                    row(
                        PropsData {
                            cross_align: CrossAlign::Center,
                            gap: 12.0,
                            ..PropsData::default()
                        },
                        vec![
                            wifi_icon(icons, None, tier.wifi_icon_size),
                            text(
                                "SETUP",
                                TextStyle {
                                    size: WIDE_SETUP_BADGE_SIZE,
                                    weight: FontWeight::BOLD,
                                    color: GREEN_50,
                                    ..TextStyle::default()
                                },
                            ),
                            text(
                                fit_line(ap_ssid, WIDE_SSID_WIDTH, value_size),
                                text_style(value_size, WHITE),
                            ),
                        ],
                    ),
                    text(
                        "Join this network from your phone to reconfigure WiFi.",
                        text_style(INFO_HEADER_SIZE, GRAY_50),
                    ),
                ],
            ),
        ),
    };

    let addresses = col(
        PropsData {
            gap: WIDE_INFO_STACK_GAP,
            ..PropsData::default()
        },
        vec![
            info_block(
                "IP Address",
                text(ip.unwrap_or("---"), text_style(value_size, WHITE)),
            ),
            info_block(
                "Hostname",
                text(
                    fit_line(hostname, WIDE_HOSTNAME_WIDTH, value_size),
                    text_style(value_size, WHITE),
                ),
            ),
        ],
    );
    let left_info = match ip {
        Some(ip) => vec![ip_qr(ip), addresses],
        None => vec![addresses],
    };

    row(
        PropsData::default(),
        vec![
            fixed_width(WIDE_INFO_LEFT_PAD),
            row(
                PropsData {
                    gap: WIDE_INFO_GAP,
                    ..PropsData::default()
                },
                left_info,
            ),
            spacer(1.0),
            wifi_block,
            fixed_width(WIDE_INFO_RIGHT_PAD),
        ],
    )
}

/// The Large tier's flow children: two equal flex halves pin the control
/// block's top edge to the vertical middle, matching the stable design. The
/// top half holds the info header, the bottom half the control rows and the
/// shared caption.
fn wide_halves(
    header: TreeNode,
    rows: Vec<TreeNode>,
    caption_node: TreeNode,
    tier: Tier,
    h_pad: f32,
) -> [TreeNode; 2] {
    let half = PropsData {
        flex: 1.0,
        ..PropsData::default()
    };
    let mut bottom_half: Vec<TreeNode> = Vec::new();
    for row_node in rows {
        bottom_half.push(pad_horizontal(row_node, h_pad));
        bottom_half.push(fixed_height(tier.row_gap));
    }
    bottom_half.push(pad_horizontal(caption_node, h_pad));
    [
        col(half, vec![fixed_height(WIDE_TOP_PAD), header]),
        col(half, bottom_half),
    ]
}

/// Compact station info for the medium/small tiers: one centered line of
/// icon + fitted SSID. The IP is not shown on these tiers.
fn compact_info(
    icons: WifiIcons,
    wifi_signal: Option<i32>,
    ssid: &str,
    tier: Tier,
    usable: f32,
) -> TreeNode {
    let gap = 12.0;
    let ssid_budget = usable - tier.wifi_icon_size - gap;
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![row(
            PropsData {
                cross_align: CrossAlign::Center,
                gap,
                ..PropsData::default()
            },
            vec![
                wifi_icon(icons, wifi_signal, tier.wifi_icon_size),
                text(
                    fit_line(ssid, ssid_budget, tier.wifi_text_size),
                    text_style(tier.wifi_text_size, WHITE),
                ),
            ],
        )],
    )
}

/// Setup-mode section for the medium/small tiers: one centered line — icon,
/// badge, fitted SSID — occupying the same height the idle info line does,
/// so the vertical budgets hold. The Large tier shows setup mode inside
/// [`wide_header`] instead.
fn setup_row(icons: WifiIcons, ap_ssid: &str, tier: Tier, usable: f32) -> TreeNode {
    let gap = 12.0;
    let badge_size = tier.wifi_text_size;
    let badge = text(
        "SETUP",
        TextStyle {
            size: badge_size,
            weight: FontWeight::BOLD,
            color: GREEN_50,
            ..TextStyle::default()
        },
    );
    #[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
    let badge_w = HOSTNAME_CHAR_W * (badge_size as f32) / 24.0 * 5.0;
    let ssid_budget = usable - tier.wifi_icon_size - badge_w - 2.0 * gap;
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![row(
            PropsData {
                cross_align: CrossAlign::Center,
                gap,
                ..PropsData::default()
            },
            vec![
                wifi_icon(icons, None, tier.wifi_icon_size),
                badge,
                text(
                    fit_line(ap_ssid, ssid_budget, tier.wifi_text_size),
                    text_style(tier.wifi_text_size, WHITE),
                ),
            ],
        )],
    )
}

/// Build the overlay UI tree for the current state.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "display sizes are small")]
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
    let w = panel.width as f32;

    // Fit the hostname to the row's usable width up front: round panels are
    // capped by the inscribed circle's chord, rectangular ones by the panel
    // width minus the close target's horizontal band on both sides.
    let hostname_budget = match panel.shape {
        DisplayShape::Round => ROUND_HOSTNAME_WIDTH,
        DisplayShape::Rectangular => w - 2.0 * (CLOSE_TARGET + tier.padding),
    };
    let hostname_str = fit_line(
        hostname.unwrap_or("N/A"),
        hostname_budget,
        tier.hostname_size,
    );
    let ssid_str = ssid.unwrap_or("Not configured");

    // The WiFi hold buttons render only in normal (non-setup) mode and only
    // when the product supports reconfiguration; the setup badge still
    // renders via setup_row.
    let wifi = panel.wifi_buttons && matches!(wifi_view, WifiView::Idle);
    let (pairs, singles) = control_groups(tier, &controls, controls_icons, icons, wifi);
    let rows = control_rows(tier, pairs, singles);

    let hostname_h = tier.hostname_size as f32 * LINE_H;
    let caption_h = tier.caption_size as f32 * LINE_H;
    let (usable, h_pad) = match panel.shape {
        DisplayShape::Round => (w - 2.0 * ROUND_H_PAD, ROUND_H_PAD),
        DisplayShape::Rectangular => (w - 2.0 * tier.padding, tier.padding),
    };

    // The caption slot always occupies its line height so captions appearing
    // and disappearing never shift the control rows.
    let caption_node =
        shared_caption(tier, &controls, usable).unwrap_or_else(|| fixed_height(caption_h));

    // The wide panel folds the station/setup info into its top header; only
    // the compact tiers keep a dedicated info line at the bottom.
    let compact_wifi_node = || match wifi_view {
        WifiView::Setup { ap_ssid } => setup_row(icons, ap_ssid, tier, usable),
        WifiView::Idle => compact_info(icons, wifi_signal, ssid_str, tier, usable),
    };

    let mut children: Vec<TreeNode> = Vec::new();
    match panel.shape {
        DisplayShape::Rectangular if tier.large_text => {
            let header = wide_header(
                hostname.unwrap_or("N/A"),
                ip,
                icons,
                wifi_signal,
                ssid_str,
                tier,
                wifi_view,
            );
            children.extend(wide_halves(header, rows, caption_node, tier, h_pad));
        }
        DisplayShape::Rectangular => {
            // Top padding is an explicit spacer (not container padding) so the
            // close button's absolute insets resolve against the panel box.
            children.push(fixed_height(tier.padding));
            children.push(pad_horizontal(
                hostname_row(&hostname_str, tier.hostname_size),
                CLOSE_TARGET + tier.padding,
            ));
            // Pin the first control row below the close target's bottom edge
            // so their hit regions stay disjoint.
            children.push(fixed_height((CLOSE_TARGET - hostname_h).max(tier.row_gap)));
            for row_node in rows {
                children.push(pad_horizontal(row_node, h_pad));
                children.push(fixed_height(tier.row_gap));
            }
            children.push(pad_horizontal(caption_node, h_pad));
            children.push(fixed_height(tier.row_gap));
            children.push(spacer(1.0));
            children.push(pad_horizontal(compact_wifi_node(), h_pad));
            children.push(fixed_height(tier.padding));
        }
        DisplayShape::Round => {
            children.push(fixed_height(ROUND_TOP_GAP));
            children.push(hostname_row(&hostname_str, tier.hostname_size));
            // Pin the control rows to a fixed top edge below the chord-safe
            // close target.
            children.push(fixed_height(
                ROUND_CONTROLS_TOP - ROUND_TOP_GAP - hostname_h,
            ));
            for row_node in rows {
                children.push(pad_horizontal(row_node, h_pad));
                children.push(fixed_height(tier.row_gap));
            }
            children.push(pad_horizontal(caption_node, h_pad));
            children.push(spacer(1.0));
            children.push(pad_horizontal(compact_wifi_node(), h_pad));
            children.push(fixed_height(ROUND_BOTTOM_GAP));
        }
    }
    // Last child: absolute positioning takes it out of flow, and rendering
    // follows child order, so it paints on top of everything.
    children.push(close_button(&panel, tier, controls_icons.close));
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
    use super::*;

    fn round_panel() -> Panel {
        Panel {
            shape: DisplayShape::Round,
            width: 480,
            height: 480,
            wifi_buttons: true,
        }
    }

    fn wide_panel() -> Panel {
        Panel {
            shape: DisplayShape::Rectangular,
            width: 1280,
            height: 480,
            wifi_buttons: true,
        }
    }

    fn narrow_panel() -> Panel {
        Panel {
            shape: DisplayShape::Rectangular,
            width: 480,
            height: 320,
            wifi_buttons: true,
        }
    }

    fn small_panel() -> Panel {
        Panel {
            shape: DisplayShape::Rectangular,
            width: 320,
            height: 240,
            wifi_buttons: true,
        }
    }

    /// Every optional control present: the worst-case layout.
    fn all_controls() -> Controls<'static> {
        Controls {
            brightness: Some(100),
            volume: Some(100),
            night_mode: Some(NightMode {
                active: true,
                until: Some("06:30"),
            }),
            restart: Some(HoldControl::default()),
            wifi_reconfig: HoldControl::default(),
            wifi_reconnect: HoldControl::default(),
            pressed: None,
        }
    }

    fn build(panel: Panel, view: WifiView<'_>) -> TreeNode {
        build_tree(
            Some("braiins-deck"),
            Some("10.0.0.2"),
            Some(-55),
            Some("MyWifi"),
            WifiIcons::default(),
            panel,
            view,
            ControlIcons::default(),
            Controls::default(),
        )
    }

    fn build_with_controls(panel: Panel, controls: Controls<'_>) -> TreeNode {
        build_tree(
            Some("braiins-deck"),
            Some("10.0.0.2"),
            Some(-55),
            Some("MyWifi"),
            WifiIcons::default(),
            panel,
            WifiView::Idle,
            ControlIcons::default(),
            controls,
        )
    }

    /// Direct children of a container node, if any.
    fn children(node: &TreeNode) -> Option<&[TreeNode]> {
        match node {
            TreeNode::Column(_, kids)
            | TreeNode::Row(_, kids)
            | TreeNode::Center(_, kids)
            | TreeNode::Scroll { children: kids, .. } => Some(kids),
            TreeNode::Tag { content, .. } => Some(std::slice::from_ref(&**content)),
            TreeNode::Paragraph { .. }
            | TreeNode::Button { .. }
            | TreeNode::Spacer { .. }
            | TreeNode::Canvas { .. }
            | TreeNode::Notification { .. }
            | TreeNode::RelTime { .. }
            | TreeNode::Modal { .. }
            | TreeNode::ProgressBar { .. }
            | TreeNode::Switcher { .. }
            | TreeNode::Skeleton(_) => None,
        }
    }

    /// Depth-first search for the first Canvas carrying `key`.
    fn find_canvas<'t>(node: &'t TreeNode, key: &str) -> Option<&'t Vec<DrawCommand>> {
        if let TreeNode::Canvas {
            touch_key: Some(k),
            draws,
            ..
        } = node
            && k == key
        {
            return Some(draws);
        }
        children(node)?.iter().find_map(|k| find_canvas(k, key))
    }

    /// Recursively collect every keyed Canvas touch key in the tree.
    fn canvas_keys(node: &TreeNode, out: &mut Vec<String>) {
        if let TreeNode::Canvas {
            touch_key: Some(k), ..
        } = node
        {
            out.push(k.clone());
        }
        if let Some(kids) = children(node) {
            for k in kids {
                canvas_keys(k, out);
            }
        }
    }

    /// Whether the subtree contains an unkeyed Canvas (the WiFi status icon).
    fn has_unkeyed_canvas(node: &TreeNode) -> bool {
        if let TreeNode::Canvas {
            touch_key: None, ..
        } = node
        {
            return true;
        }
        children(node).into_iter().flatten().any(has_unkeyed_canvas)
    }

    /// Recursively collect the payload of every QR draw in the tree.
    fn qr_texts(node: &TreeNode, out: &mut Vec<String>) {
        if let TreeNode::Canvas { draws, .. } = node {
            out.extend(draws.iter().filter_map(|draw| {
                if let DrawCommand::Qr { text, .. } = draw {
                    Some(text.clone())
                } else {
                    None
                }
            }));
        }
        if let Some(kids) = children(node) {
            for k in kids {
                qr_texts(k, out);
            }
        }
    }

    /// Recursively collect every span text in the tree.
    fn collect_texts(node: &TreeNode, out: &mut Vec<String>) {
        if let TreeNode::Paragraph { spans, .. } = node {
            for span in spans {
                out.push(span.text.clone());
            }
        }
        if let Some(kids) = children(node) {
            for k in kids {
                collect_texts(k, out);
            }
        }
    }

    /// Largest text size in the subtree — the line height driver of a text
    /// band (hostname, caption).
    fn max_text_size(node: &TreeNode) -> u32 {
        let own = if let TreeNode::Paragraph { base_style, .. } = node {
            base_style.size
        } else {
            0
        };
        children(node)
            .into_iter()
            .flatten()
            .map(max_text_size)
            .fold(own, u32::max)
    }

    /// Abs-diff float assertion (`float_cmp` is denied by the workspace lints).
    fn assert_close(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-3,
            "{what}: expected ~{expected}, got {actual}"
        );
    }

    fn assert_circle(panel: &Panel, expected: f32) {
        assert!((tier_for(panel).circle - expected).abs() < f32::EPSILON);
    }

    #[test]
    fn tier_selection_by_panel() {
        assert_circle(&wide_panel(), 112.0);
        assert_circle(&narrow_panel(), 64.0);
        assert_circle(&round_panel(), 64.0);
        assert_circle(&small_panel(), 48.0);
    }

    #[test]
    fn hold_ring_is_readable_and_grows_with_progress() {
        let btn = round_button(
            "k",
            ButtonIcon::square(None),
            112.0,
            48.0,
            CIRCLE_FILL,
            TRANSPARENT,
            Some(0.5),
        );
        let TreeNode::Canvas {
            draws, touch_key, ..
        } = btn
        else {
            panic!("expected Canvas")
        };
        assert_eq!(touch_key.as_deref(), Some("k"));
        let DrawCommand::Circle {
            fill: Fill::Solid(circle),
            r,
            ..
        } = &draws[0]
        else {
            panic!("expected Circle")
        };
        assert_eq!(*circle, CIRCLE_FILL);
        assert_close(
            *r,
            56.0 - RING_W,
            "a hold button's fill sits inside the ring",
        );
        let DrawCommand::Arc {
            fill: ArcFill::Solid(ring),
            width,
            ..
        } = &draws[1]
        else {
            panic!("expected ring Arc")
        };
        assert_close(*width, 7.5, "half hold grows the ring from 3px to 12px");
        assert_eq!(
            *ring,
            HOLD_RING.scale_alpha(0.65),
            "half hold grows opacity from 30% to 100%",
        );
        assert!(matches!(draws[2], DrawCommand::Svg { .. }));
    }

    #[test]
    fn hold_ring_radius_starts_at_button_and_grows_outward() {
        let radii = |progress| {
            let btn = round_button(
                "k",
                ButtonIcon::square(None),
                112.0,
                48.0,
                CIRCLE_FILL,
                TRANSPARENT,
                Some(progress),
            );
            let TreeNode::Canvas { draws, .. } = btn else {
                panic!("expected Canvas")
            };
            let DrawCommand::Circle { r: button, .. } = &draws[0] else {
                panic!("expected Circle")
            };
            let DrawCommand::Arc { radius: ring, .. } = &draws[1] else {
                panic!("expected ring Arc")
            };
            (*button, *ring)
        };

        let (button, idle) = radii(0.0);
        let (_, half) = radii(0.5);
        let (_, full) = radii(1.0);
        assert_close(idle, button, "the idle ring rests on the button radius");
        assert_close(half, 47.0, "the half-hold ring grows outward");
        assert_close(full, 50.0, "the full-hold ring reaches the outer rim");
        assert!(
            idle < half && half < full,
            "the ring radius grows monotonically with hold progress"
        );
    }

    #[test]
    fn ringless_button_fills_the_whole_diameter() {
        let btn = round_button(
            "k",
            ButtonIcon::square(None),
            112.0,
            48.0,
            CIRCLE_FILL,
            TRANSPARENT,
            None,
        );
        let TreeNode::Canvas { draws, .. } = btn else {
            panic!("expected Canvas")
        };
        let DrawCommand::Circle { r, .. } = &draws[0] else {
            panic!("expected Circle")
        };
        assert_close(*r, 56.0, "no ring: the fill spans the tier diameter");
        assert!(
            !draws.iter().any(|d| matches!(d, DrawCommand::Arc { .. })),
            "no ring arc without hold progress"
        );
    }

    #[test]
    fn gating_decides_which_buttons_exist() {
        let keys = |panel: Panel, view: WifiView<'_>, controls: Controls<'_>| {
            let mut out = Vec::new();
            canvas_keys(
                &build_tree(
                    Some("braiins-deck"),
                    Some("10.0.0.2"),
                    Some(-55),
                    Some("MyWifi"),
                    WifiIcons::default(),
                    panel,
                    view,
                    ControlIcons::default(),
                    controls,
                ),
                &mut out,
            );
            out
        };

        let minimal = keys(wide_panel(), WifiView::Idle, Controls::default());
        assert!(
            minimal.iter().any(|k| k == CLOSE_KEY),
            "{CLOSE_KEY} always renders"
        );
        for key in [
            BRIGHTNESS_DOWN_KEY,
            BRIGHTNESS_UP_KEY,
            VOLUME_DOWN_KEY,
            VOLUME_UP_KEY,
            NIGHT_MODE_KEY,
            RESTART_KEY,
        ] {
            assert!(!minimal.iter().any(|k| k == key), "{key} is gated off");
        }
        assert!(
            minimal.iter().any(|k| k == WIFI_RECONFIG_KEY),
            "wifi buttons render when the panel supports them"
        );

        let full = keys(wide_panel(), WifiView::Idle, all_controls());
        for key in [
            BRIGHTNESS_DOWN_KEY,
            BRIGHTNESS_UP_KEY,
            VOLUME_DOWN_KEY,
            VOLUME_UP_KEY,
            NIGHT_MODE_KEY,
            RESTART_KEY,
            WIFI_RECONFIG_KEY,
            WIFI_RECONNECT_KEY,
            CLOSE_KEY,
        ] {
            assert!(full.iter().any(|k| k == key), "{key} renders when enabled");
        }

        let mut no_wifi_panel = wide_panel();
        no_wifi_panel.wifi_buttons = false;
        let no_wifi = keys(no_wifi_panel, WifiView::Idle, all_controls());
        assert!(!no_wifi.iter().any(|k| k == WIFI_RECONFIG_KEY));
        assert!(!no_wifi.iter().any(|k| k == WIFI_RECONNECT_KEY));

        let setup = keys(
            wide_panel(),
            WifiView::Setup {
                ap_ssid: "Deck ABCD",
            },
            all_controls(),
        );
        assert!(!setup.iter().any(|k| k == WIFI_RECONFIG_KEY));
        assert!(!setup.iter().any(|k| k == WIFI_RECONNECT_KEY));
        assert!(setup.iter().any(|k| k == CLOSE_KEY));
    }

    /// The night button's circle fill and icon tint.
    fn night_colors(controls: Controls<'_>) -> (Color, Color) {
        let tree = build_with_controls(wide_panel(), controls);
        let draws = find_canvas(&tree, NIGHT_MODE_KEY).expect("BUG: night canvas must exist");
        let DrawCommand::Circle {
            fill: Fill::Solid(fill),
            ..
        } = draws[0]
        else {
            panic!("expected Circle")
        };
        let DrawCommand::Svg { color, .. } = draws[draws.len() - 1] else {
            panic!("expected Svg")
        };
        (fill, color)
    }

    #[test]
    fn night_mode_active_fills_blue_and_never_inverts() {
        let night = |active| Controls {
            night_mode: Some(NightMode {
                active,
                until: Some("06:30"),
            }),
            ..Controls::default()
        };
        assert_eq!(night_colors(night(true)), (NIGHT_ACTIVE, TRANSPARENT));
        assert_eq!(night_colors(night(false)), (CIRCLE_FILL, TRANSPARENT));

        let pressed_active = Controls {
            pressed: Some(NIGHT_MODE_KEY),
            ..night(true)
        };
        assert_eq!(
            night_colors(pressed_active),
            (NIGHT_ACTIVE, TRANSPARENT),
            "a pressed active night button must not invert"
        );
        let pressed_inactive = Controls {
            pressed: Some(NIGHT_MODE_KEY),
            ..night(false)
        };
        assert_eq!(
            night_colors(pressed_inactive),
            (CIRCLE_FILL, TRANSPARENT),
            "a pressed inactive night button must not invert either"
        );
    }

    #[test]
    fn pressed_step_button_inverts() {
        let controls = Controls {
            volume: Some(40),
            pressed: Some(VOLUME_UP_KEY),
            ..Controls::default()
        };
        let tree = build_with_controls(wide_panel(), controls);
        let draws = find_canvas(&tree, VOLUME_UP_KEY).expect("BUG: volume-up canvas must exist");
        let DrawCommand::Circle {
            fill: Fill::Solid(fill),
            ..
        } = draws[0]
        else {
            panic!("expected Circle")
        };
        assert_eq!(fill, CIRCLE_PRESSED);
        let DrawCommand::Svg { color, .. } = draws[draws.len() - 1] else {
            panic!("expected Svg")
        };
        assert_eq!(color, ICON_PRESSED_TINT);

        let unpressed =
            find_canvas(&tree, VOLUME_DOWN_KEY).expect("BUG: volume-down canvas must exist");
        let DrawCommand::Circle {
            fill: Fill::Solid(fill),
            ..
        } = unpressed[0]
        else {
            panic!("expected Circle")
        };
        assert_eq!(fill, CIRCLE_FILL, "only the pressed button inverts");
    }

    #[test]
    fn hold_progress_renders_ring_alpha() {
        let ring_of = |progress| {
            let controls = Controls {
                restart: Some(HoldControl {
                    caption: None,
                    progress,
                }),
                ..Controls::default()
            };
            let tree = build_with_controls(wide_panel(), controls);
            let draws = find_canvas(&tree, RESTART_KEY).expect("BUG: restart canvas must exist");
            draws
                .iter()
                .find_map(|d| {
                    if let DrawCommand::Arc {
                        fill: ArcFill::Solid(c),
                        ..
                    } = d
                    {
                        Some(*c)
                    } else {
                        None
                    }
                })
                .expect("BUG: restart canvas must carry the hold ring")
        };
        assert_eq!(
            ring_of(0.5),
            HOLD_RING.scale_alpha(0.65),
            "half hold sits halfway between the 30% floor and opaque",
        );
        assert_eq!(
            ring_of(0.0),
            HOLD_RING.scale_alpha(RING_MIN_ALPHA),
            "an idle hold button keeps a faint 30% ring",
        );
    }

    #[test]
    fn caption_precedence_and_prefixes() {
        let caption_texts = |panel: Panel, controls: Controls<'_>| {
            let mut texts = Vec::new();
            collect_texts(&build_with_controls(panel, controls), &mut texts);
            texts
        };
        let holding = HoldControl {
            caption: Some("Keep holding…"),
            progress: 0.2,
        };

        let all = Controls {
            restart: Some(holding),
            wifi_reconfig: holding,
            wifi_reconnect: holding,
            ..Controls::default()
        };
        let all_texts = caption_texts(narrow_panel(), all);
        assert!(
            all_texts.iter().any(|t| t == "Restart: Keep holding…"),
            "restart beats the wifi captions"
        );
        assert!(
            !all_texts
                .iter()
                .any(|t| t.starts_with("Reset WiFi:") || t.starts_with("Reconnect:")),
            "the losing captions must not render alongside the winner"
        );

        let wifi_only = Controls {
            wifi_reconfig: holding,
            wifi_reconnect: holding,
            ..Controls::default()
        };
        let wifi_texts = caption_texts(narrow_panel(), wifi_only);
        assert!(
            wifi_texts.iter().any(|t| t == "Reset WiFi: Keep holding…"),
            "reconfigure beats reconnect"
        );
        assert!(
            !wifi_texts.iter().any(|t| t.starts_with("Reconnect:")),
            "the losing reconnect caption must not render"
        );

        let reconnect_only = Controls {
            wifi_reconnect: HoldControl {
                caption: Some("Reconnecting…"),
                progress: 0.0,
            },
            ..Controls::default()
        };
        assert!(
            caption_texts(narrow_panel(), reconnect_only)
                .iter()
                .any(|t| t == "Reconnect: Reconnecting…")
        );

        let night = Controls {
            night_mode: Some(NightMode {
                active: true,
                until: Some("22:00"),
            }),
            ..Controls::default()
        };
        assert!(
            caption_texts(narrow_panel(), night)
                .iter()
                .any(|t| t == "Night mode on until 22:00"),
            "compact tiers surface the night end time on the caption line"
        );
        assert!(
            !caption_texts(wide_panel(), night)
                .iter()
                .any(|t| t.starts_with("Night mode on until")),
            "the Large tier shows the end time in the night group instead"
        );
        assert!(
            caption_texts(wide_panel(), night)
                .iter()
                .any(|t| t == "Until 22:00")
        );
    }

    const PAIR_KEYS: [&str; 4] = [
        VOLUME_DOWN_KEY,
        VOLUME_UP_KEY,
        BRIGHTNESS_DOWN_KEY,
        BRIGHTNESS_UP_KEY,
    ];
    const SINGLE_KEYS: [&str; 4] = [
        NIGHT_MODE_KEY,
        RESTART_KEY,
        WIFI_RECONFIG_KEY,
        WIFI_RECONNECT_KEY,
    ];

    fn is_control_key(k: &str) -> bool {
        PAIR_KEYS.contains(&k) || SINGLE_KEYS.contains(&k)
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
            .map(|k| expected_flow_height(k, tier, setup))
            .sum();
        let bottom_h: f32 = bottom_kids
            .iter()
            .map(|k| expected_flow_height(k, tier, setup))
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

    /// Expected height of [`wide_header`], derived from the same `Tier`
    /// fields the builder uses so the test cannot drift from the layout
    /// silently.
    #[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
    fn wide_info_height(tier: Tier, setup: bool) -> f32 {
        let header = INFO_HEADER_SIZE as f32 * LINE_H + INFO_HEADER_GAP;
        let wifi_value = (tier.hostname_size as f32 * LINE_H).max(tier.wifi_icon_size);
        let wifi = if setup {
            header + wifi_value + 6.0 + INFO_HEADER_SIZE as f32 * LINE_H
        } else {
            header + wifi_value
        };
        let addresses = 2.0 * (header + tier.hostname_size as f32 * LINE_H) + WIDE_INFO_STACK_GAP;
        // The QR is the tallest child but renders only with a known IP,
        // so folding it in unconditionally bounds the worst case.
        wifi.max(addresses).max(WIDE_QR_SIZE)
    }

    /// Expected height of one flow child of the root column, derived from the
    /// same `Tier` fields the builders use so the test cannot drift from the
    /// layout silently. The flex filler reports 0 (its worst case).
    #[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
    fn expected_flow_height(node: &TreeNode, tier: Tier, setup: bool) -> f32 {
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
        if has_pair || has_single {
            let value_gap = if tier.large_text { 8.0 } else { 2.0 };
            let pair_h = tier.circle
                + value_gap
                + tier.value_size as f32 * LINE_H
                + if tier.large_text {
                    tier.caption_size as f32 * LINE_H
                } else {
                    0.0
                };
            let single_h = tier.circle
                + if tier.large_text {
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
            return if tier.large_text {
                wide_info_height(tier, setup)
            } else {
                tier.wifi_icon_size.max(tier.wifi_text_size as f32 * LINE_H)
            };
        }
        let size = max_text_size(node);
        if size > 0 {
            return size as f32 * LINE_H;
        }
        0.0
    }

    #[test]
    fn controls_start_below_the_close_target() {
        let long_ssid = "An-Extremely-Long-Setup-Network-Name-420";
        assert_eq!(long_ssid.chars().count(), 40);
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            for (view, setup) in [
                (WifiView::Idle, false),
                (WifiView::Setup { ap_ssid: long_ssid }, true),
            ] {
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
                    all_controls(),
                );
                let kids = children(&tree).expect("BUG: root must be a container");

                let close_bottom = close_origin(&panel, tier).1 + CLOSE_TARGET;
                #[expect(clippy::cast_precision_loss, reason = "panel sizes are small")]
                let panel_h = panel.height as f32;
                if tier.large_text {
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
                        before_controls += expected_flow_height(kid, tier, setup);
                    }
                    total += expected_flow_height(kid, tier, setup);
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

    #[test]
    fn qr_encodes_the_ip_url_only_on_the_wide_tier() {
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
                let wide =
                    matches!(panel.shape, DisplayShape::Rectangular) && tier_for(&panel).large_text;
                let expected: Vec<String> = if wide {
                    ip.map(|ip| format!("http://{ip}")).into_iter().collect()
                } else {
                    Vec::new()
                };
                assert_eq!(qrs, expected, "{panel:?} ip={ip:?}");
            }
        }
    }

    /// Conservative width of a single-line string,
    /// on the same per-glyph budget [`fit_line`] truncates against.
    #[expect(clippy::cast_precision_loss, reason = "text sizes are small")]
    fn line_width(s: &str, size: u32) -> f32 {
        s.chars().count() as f32 * HOSTNAME_CHAR_W * (size as f32) / 24.0
    }

    /// Conservative min-content width of [`wide_header`] at its fit budgets.
    /// Including the SETUP badge bounds both WiFi views, leaving idle mode —
    /// which has no badge — some slack. The setup hint is left out: it wraps,
    /// so a single-line glyph budget does not describe it.
    fn wide_info_width(tier: Tier) -> f32 {
        let addresses = line_width("255.255.255.255", tier.hostname_size).max(WIDE_HOSTNAME_WIDTH);
        let wifi = tier.wifi_icon_size
            + WIDE_WIFI_GAP
            + line_width("SETUP", WIDE_SETUP_BADGE_SIZE)
            + WIDE_WIFI_GAP
            + WIDE_SSID_WIDTH;
        WIDE_INFO_LEFT_PAD + WIDE_QR_SIZE + WIDE_INFO_GAP + addresses + wifi + WIDE_INFO_RIGHT_PAD
    }

    #[test]
    fn wide_header_fits_the_panel_width() {
        let panel = wide_panel();
        #[expect(clippy::cast_precision_loss, reason = "panel sizes are small")]
        let panel_w = panel.width as f32;
        let width = wide_info_width(tier_for(&panel));
        assert!(
            width <= panel_w,
            "header content {width} overflows {panel_w} — the flex spacer \
             collapses and the WiFi block runs off the right edge"
        );
    }

    #[test]
    fn close_is_the_only_absolutely_positioned_canvas() {
        fn absolute_canvases<'t>(
            node: &'t TreeNode,
            out: &mut Vec<(&'t PropsData, Option<&'t str>)>,
        ) {
            if let TreeNode::Canvas {
                props, touch_key, ..
            } = node
                && (props.inset_top.is_finite() || props.inset_left.is_finite())
            {
                out.push((props, touch_key.as_deref()));
            }
            if let Some(kids) = children(node) {
                for k in kids {
                    absolute_canvases(k, out);
                }
            }
        }
        for panel in [wide_panel(), narrow_panel(), small_panel(), round_panel()] {
            let tree = build_with_controls(panel, all_controls());
            let mut absolute = Vec::new();
            absolute_canvases(&tree, &mut absolute);
            assert_eq!(
                absolute.len(),
                1,
                "{panel:?}: the close target is the only out-of-flow touchable"
            );
            assert_eq!(absolute[0].1, Some(CLOSE_KEY));

            let kids = children(&tree).expect("BUG: root must be a container");
            let last = kids.last().expect("BUG: root must have children");
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

    #[test]
    fn short_hostname_is_unchanged() {
        assert_eq!(
            fit_line("braiins-deck", ROUND_HOSTNAME_WIDTH, 24),
            "braiins-deck"
        );
    }

    #[test]
    fn long_hostname_is_truncated_with_ellipsis() {
        let long = "braiins-deck-extremely-long-hostname-xyz";
        let fitted = fit_line(long, ROUND_HOSTNAME_WIDTH, 24);
        assert!(fitted.ends_with('…'));
        assert!(fitted.chars().count() < long.chars().count());
    }

    #[test]
    fn smaller_text_fits_more_characters() {
        let s = "a-string-that-is-fairly-long-indeed";
        let at_24 = fit_line(s, 256.0, 24);
        let at_12 = fit_line(s, 256.0, 12);
        assert!(at_12.chars().count() > at_24.chars().count());
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

    #[test]
    fn round_close_target_stays_inside_the_disc_and_above_the_controls() {
        // Round-panel chord safety: the far corner of the close target must
        // stay inside the disc.
        let panel = round_panel();
        let tier = tier_for(&panel);
        let (left, top) = close_origin(&panel, tier);
        let r = 240.0_f32;
        let far_x = left + CLOSE_TARGET - r;
        let far_y = top - r;
        let dist = (far_x * far_x + far_y * far_y).sqrt();
        assert!(
            dist < r,
            "close target far corner at {dist} must stay inside the 240px disc"
        );
        assert!(
            ROUND_CONTROLS_TOP >= top + CLOSE_TARGET,
            "round control rows start below the close target"
        );
    }
}
