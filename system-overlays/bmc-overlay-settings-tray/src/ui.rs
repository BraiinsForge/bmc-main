// Copyright (C) 2026  Braiins Systems s.r.o.

//! Pure UI-tree construction for the settings overlay. Builds a
//! `bmc_render::tree::TreeNode` the GPU renderer lays out and paints. Kept free
//! of host/GL imports so it compiles and unit-tests on the host.

use bmc_platform::DisplayShape;
use bmc_render::tree::{DrawCommand, PropsData, SpanData, TextStyle, TreeNode};
use bmc_wasm_protocol::colors::{BLACK, GRAY_50, GRAY_80, GREEN_50, TRANSPARENT, WHITE};
use bmc_wasm_protocol::{Color, CrossAlign, FontWeight, SvgId, TextAlign};

/// Stable touch key for the brightness slider drag.
pub const BRIGHTNESS_SLIDER_KEY: &str = "brightness";

/// Stable touch key for the WiFi reconfiguration hold button.
pub const WIFI_RECONFIG_KEY: &str = "wifi_reconfig";

/// Stable touch key for the bare WiFi reconnect hold button.
pub const WIFI_RECONNECT_KEY: &str = "wifi_reconnect";

/// `ButtonStyle::Secondary` wire value (see `bmc_wasm_protocol::ButtonStyle`).
const BUTTON_STYLE_SECONDARY: u8 = 1;
/// `ButtonSize::Normal` wire value (see `bmc_wasm_protocol::ButtonSize`).
const BUTTON_SIZE_NORMAL: u8 = 1;

/// What to show in the WiFi/reconfig area of the overlay.
#[derive(Debug, Clone, Copy)]
pub enum WifiView<'a> {
    /// Normal mode: station info row plus the hold-to-confirm button below it.
    /// `label` is the button caption (changes to convey hold progress).
    Idle { label: &'a str },
    /// Setup mode: compact row with a SETUP badge and the AP SSID, no button.
    Setup { ap_ssid: &'a str },
}

/// Display panel the overlay is laid out for.
#[derive(Debug, Clone, Copy)]
pub struct Panel {
    pub shape: DisplayShape,
    pub width: u32,
    pub height: u32,
    /// Whether to render the WiFi reconfigure/reconnect button row. WiFi
    /// reconfiguration is only supported on boards whose AP runs over the
    /// mac80211 radio (BMC100, BFM100); the BMM boards drive their ESP32 AP
    /// through a separate firmware path the overlay does not implement, so the
    /// buttons are hidden there.
    pub wifi_buttons: bool,
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
        match dbm {
            None | Some(0) => self.problem,
            Some(level) if level >= -60 => self.strong,
            Some(level) if level >= -75 => self.fair,
            Some(_) => self.low,
        }
    }
}

const MIN_BRIGHTNESS: u8 = 10;
const MAX_BRIGHTNESS: u8 = 100;

/// Aspect ratio (width / height) at or above which the Wi-Fi info collapses
/// into a single row; below it the SSID and IP stack vertically.
const WIDE_ASPECT: f32 = 2.0;

/// Gap kept below the Wi-Fi info on round panels, so it clears the curved
/// bottom edge instead of sitting flush against it.
const ROUND_BOTTOM_GAP: f32 = 48.0;

/// Gap kept above the hostname on round panels, mirroring the bottom gap so
/// the first row clears the curved top edge.
const ROUND_TOP_GAP: f32 = 48.0;

/// Horizontal inset on round panels, keeping content inside the inscribed
/// circle where the usable width is narrower than the full panel.
const ROUND_H_PAD: f32 = 48.0;

/// Symmetric padding inside rectangular panels.
const RECT_PADDING: f32 = 24.0;

/// Fixed width of the reconfigure + reconnect button row. Capped so the row
/// fits the round panel's usable chord (~384px inside the inscribed circle)
/// while keeping the two buttons compact instead of stretching the full panel.
const BUTTON_ROW_WIDTH: f32 = 360.0;

/// Usable hostname width (px) on round panels. Near the top curve the
/// inscribed circle's chord is narrower than the full width, so the centered
/// hostname is budgeted against this chord rather than the panel width.
const ROUND_HOSTNAME_WIDTH: f32 = 256.0;

/// Conservative per-glyph advance (px) for the 24px bold hostname font,
/// rounded up from the measured Braiins Sans bold width so the character
/// budget never under-counts and lets a hostname overflow its row. The
/// renderer has no single-line ellipsis, so the fit is enforced on the
/// string in [`fit_hostname`] instead.
const HOSTNAME_CHAR_W: f32 = 16.0;

/// Truncate `hostname` with a trailing ellipsis so it fits on one line within
/// `width_px`. Pure (no host calls) so it stays host-testable.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "max_chars is a small non-negative count"
)]
fn fit_hostname(hostname: &str, width_px: f32) -> String {
    let max_chars = (width_px / HOSTNAME_CHAR_W).floor().max(1.0) as usize;
    if hostname.chars().count() <= max_chars {
        return hostname.to_owned();
    }
    let kept: String = hostname.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// A `Column` carrying the given props and children.
fn col(props: PropsData, kids: Vec<TreeNode>) -> TreeNode {
    TreeNode::Column(props, kids)
}

/// A `Row` carrying the given props and children.
fn row(props: PropsData, kids: Vec<TreeNode>) -> TreeNode {
    TreeNode::Row(props, kids)
}

/// A single text node: a paragraph with one span.
fn text(s: impl Into<String>, style: TextStyle) -> TreeNode {
    TreeNode::Paragraph {
        props: PropsData::default(),
        base_style: style,
        spans: vec![SpanData {
            text: s.into(),
            weight: None,
            color: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }],
    }
}

/// A flexible spacer that takes the leftover main-axis space.
fn spacer(flex: f32) -> TreeNode {
    TreeNode::Spacer { flex }
}

/// A `Secondary`/`Normal` hold-to-confirm button keyed by `key`.
fn button(key: &str, label: impl Into<String>) -> TreeNode {
    TreeNode::Button {
        id: key.to_owned(),
        label: label.into(),
        style: BUTTON_STYLE_SECONDARY,
        size: BUTTON_SIZE_NORMAL,
        icon_id: None,
        disabled: false,
        skin: None,
    }
}

fn text_style(size: u32, color: Color) -> TextStyle {
    TextStyle {
        size,
        color,
        ..TextStyle::default()
    }
}

fn fixed_height(height: f32) -> TreeNode {
    col(
        PropsData {
            height,
            ..PropsData::default()
        },
        Vec::new(),
    )
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

/// Centered hostname header row. The string is pre-fitted by [`fit_hostname`],
/// so it is laid out on a single line without wrapping. The centering column
/// is what actually centers the text node; a bare paragraph with
/// `align: Center` does not center under a stretching parent.
fn hostname_row(hostname: &str) -> TreeNode {
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![text(
            hostname,
            TextStyle {
                size: 24,
                weight: FontWeight::BOLD,
                color: WHITE,
                align: TextAlign::Center,
                ..TextStyle::default()
            },
        )],
    )
}

fn wifi_icon(icons: WifiIcons, wifi_signal: Option<i32>) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: 32.0,
            height: 32.0,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Svg {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
            color: TRANSPARENT,
            icon_id: icons.for_signal(wifi_signal),
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// Brightness section: an optional "Brightness" label above a draggable slider
/// and its percentage readout. The label is dropped on short panels (narrow
/// rectangular) where the extra row would push the Wi-Fi info off the bottom.
fn brightness_section(brightness: u8, with_label: bool) -> TreeNode {
    let frac = f32::from(brightness.saturating_sub(MIN_BRIGHTNESS))
        / f32::from(MAX_BRIGHTNESS - MIN_BRIGHTNESS);
    let slider = row(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: 16.0,
            ..PropsData::default()
        },
        vec![
            col(
                PropsData {
                    flex: 1.0,
                    ..PropsData::default()
                },
                vec![TreeNode::ProgressBar {
                    touch_key: Some(BRIGHTNESS_SLIDER_KEY.to_owned()),
                    track_h: 8.0,
                    mode: 0,
                    fraction: frac,
                    active: true,
                    fill_color: GREEN_50,
                    track_color: GRAY_80,
                    bg_color: TRANSPARENT,
                    skin: None,
                }],
            ),
            text(
                format!("{brightness}%"),
                TextStyle {
                    size: 24,
                    weight: FontWeight::BOLD,
                    color: WHITE,
                    ..TextStyle::default()
                },
            ),
        ],
    );
    let mut children = Vec::with_capacity(2);
    if with_label {
        children.push(text("Brightness", text_style(24, WHITE)));
    }
    children.push(slider);
    col(
        PropsData {
            gap: 8.0,
            ..PropsData::default()
        },
        children,
    )
}

/// Rectangular overlay column: hostname header, brightness, then the Wi-Fi
/// `info` block pushed to the bottom edge. `with_brightness_label` is dropped on
/// short panels to reclaim the vertical space.
fn rect_overlay(
    hostname_str: &str,
    brightness: u8,
    with_brightness_label: bool,
    info: TreeNode,
) -> TreeNode {
    col(
        PropsData {
            background: BLACK,
            padding: RECT_PADDING,
            gap: 16.0,
            ..PropsData::default()
        },
        vec![
            hostname_row(hostname_str),
            brightness_section(brightness, with_brightness_label),
            spacer(1.0),
            info,
        ],
    )
}

/// Hold-to-confirm WiFi reconfigure button. The caller swaps `label` to convey
/// hold progress; the press is detected via the `WIFI_RECONFIG_KEY` hit region.
fn reconfig_button(label: &str) -> TreeNode {
    button(WIFI_RECONFIG_KEY, label)
}

/// Hold-to-confirm bare WiFi reconnect button. Its press is detected via the
/// `WIFI_RECONNECT_KEY` hit region; the caller spawns the reconnect sequence
/// once the hold completes. The label is static — hold progress shows only as
/// the pressed background.
fn reconnect_button() -> TreeNode {
    button(WIFI_RECONNECT_KEY, "Reconnect WiFi")
}

/// Side-by-side hold buttons: reconfigure (left) and reconnect (right). Each is
/// wrapped in a `flex: 1.0` column so they split the fixed-width row into equal,
/// label-independent halves. The row is centered and capped at
/// [`BUTTON_ROW_WIDTH`] rather than stretching the full panel, so the buttons
/// stay compact on wide displays and still fit the round panel's narrower
/// usable width.
fn button_row(reconfig_label: &str) -> TreeNode {
    let half = |node: TreeNode| {
        col(
            PropsData {
                flex: 1.0,
                ..PropsData::default()
            },
            vec![node],
        )
    };
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        vec![row(
            PropsData {
                gap: 16.0,
                width: BUTTON_ROW_WIDTH,
                ..PropsData::default()
            },
            vec![
                half(reconfig_button(reconfig_label)),
                half(reconnect_button()),
            ],
        )],
    )
}

/// Setup-mode row: a SETUP badge and the AP SSID to join, plus a one-line hint.
fn setup_row(icons: WifiIcons, ap_ssid: &str) -> TreeNode {
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
                    wifi_icon(icons, None),
                    text(
                        "SETUP",
                        TextStyle {
                            size: 14,
                            weight: FontWeight::BOLD,
                            color: GREEN_50,
                            ..TextStyle::default()
                        },
                    ),
                    flex_text(22, WHITE, ap_ssid, TextAlign::Left),
                ],
            ),
            text(
                "Join this network from your phone to reconfigure WiFi.",
                text_style(16, GRAY_50),
            ),
        ],
    )
}

/// The trailing WiFi/reconfig section used by every layout branch. `info` is the
/// existing station icon+SSID+IP node for normal mode.
fn wifi_section(info: TreeNode, icons: WifiIcons, view: WifiView<'_>, buttons: bool) -> TreeNode {
    match view {
        WifiView::Setup { ap_ssid } => setup_row(icons, ap_ssid),
        WifiView::Idle { label } if buttons => col(
            PropsData {
                gap: 16.0,
                ..PropsData::default()
            },
            vec![button_row(label), info],
        ),
        WifiView::Idle { .. } => info,
    }
}

/// A text node whose paragraph flexes to share row width with its siblings.
fn flex_text(size: u32, color: Color, s: &str, align: TextAlign) -> TreeNode {
    TreeNode::Paragraph {
        props: PropsData {
            flex: 1.0,
            ..PropsData::default()
        },
        base_style: TextStyle {
            size,
            color,
            align,
            ..TextStyle::default()
        },
        spans: vec![SpanData {
            text: s.to_owned(),
            weight: None,
            color: None,
            italic: false,
            underline: false,
            strikethrough: false,
        }],
    }
}

/// Build the overlay UI tree for the current state.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "display sizes are small")]
#[expect(
    clippy::too_many_arguments,
    reason = "overlay state is a flat set of display fields"
)]
pub fn build_tree(
    brightness: u8,
    hostname: Option<&str>,
    ip: Option<&str>,
    wifi_signal: Option<i32>,
    ssid: Option<&str>,
    icons: WifiIcons,
    panel: Panel,
    wifi_view: WifiView<'_>,
) -> TreeNode {
    let Panel {
        shape,
        width,
        height,
        wifi_buttons,
    } = panel;

    // Fit the hostname to the row's usable width up front: round panels are
    // capped by the inscribed circle's chord, rectangular ones by the panel
    // width minus padding.
    let hostname_budget = match shape {
        DisplayShape::Round => ROUND_HOSTNAME_WIDTH,
        DisplayShape::Rectangular => (width as f32) - 2.0 * RECT_PADDING,
    };
    let hostname_str = fit_hostname(hostname.unwrap_or("N/A"), hostname_budget);
    let ssid_str = ssid.unwrap_or("Not configured");
    let ip_str = format!("IP {}", ip.unwrap_or("---"));

    let wide = matches!(shape, DisplayShape::Rectangular)
        && (width as f32) / (height as f32) >= WIDE_ASPECT;

    match shape {
        // Round panels clip the corners: inset content horizontally, drop the
        // brightness into the wide middle band, center the Wi-Fi rows, and keep
        // the hostname and Wi-Fi clear of the curved top and bottom edges.
        DisplayShape::Round => {
            let info = round_info(icons, wifi_signal, ssid_str, &ip_str);
            col(
                PropsData {
                    background: BLACK,
                    gap: 16.0,
                    ..PropsData::default()
                },
                vec![
                    fixed_height(ROUND_TOP_GAP),
                    hostname_row(&hostname_str),
                    spacer(1.0),
                    pad_horizontal(brightness_section(brightness, true), ROUND_H_PAD),
                    spacer(1.0),
                    pad_horizontal(
                        wifi_section(info, icons, wifi_view, wifi_buttons),
                        ROUND_H_PAD,
                    ),
                    fixed_height(ROUND_BOTTOM_GAP),
                ],
            )
        }
        DisplayShape::Rectangular if wide => {
            let info = wide_info(icons, wifi_signal, ssid_str, &ip_str);
            rect_overlay(
                &hostname_str,
                brightness,
                true,
                wifi_section(info, icons, wifi_view, wifi_buttons),
            )
        }
        DisplayShape::Rectangular => {
            let info = narrow_info(icons, wifi_signal, ssid_str, &ip_str);
            // Narrow panel: drop the "Brightness" label so the Wi-Fi info and IP
            // are not pushed off the bottom edge.
            rect_overlay(
                &hostname_str,
                brightness,
                false,
                wifi_section(info, icons, wifi_view, wifi_buttons),
            )
        }
    }
}

/// Round-panel station info: a centered icon+SSID row above a centered IP line.
fn round_info(icons: WifiIcons, wifi_signal: Option<i32>, ssid: &str, ip: &str) -> TreeNode {
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: 8.0,
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
                    wifi_icon(icons, wifi_signal),
                    text(ssid, text_style(22, WHITE)),
                ],
            ),
            text(
                ip,
                TextStyle {
                    size: 22,
                    color: GRAY_50,
                    align: TextAlign::Center,
                    ..TextStyle::default()
                },
            ),
        ],
    )
}

/// Wide-panel station info: icon, SSID, and right-aligned IP all on one row.
/// Both SSID and IP flex so the IP right-aligns within its own share and never
/// spills past the right edge when either string is long; a bare content-width
/// IP node would push out instead.
fn wide_info(icons: WifiIcons, wifi_signal: Option<i32>, ssid: &str, ip: &str) -> TreeNode {
    row(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: 16.0,
            ..PropsData::default()
        },
        vec![
            wifi_icon(icons, wifi_signal),
            flex_text(22, WHITE, ssid, TextAlign::Left),
            flex_text(22, GRAY_50, ip, TextAlign::Right),
        ],
    )
}

/// Narrow-panel station info: icon+SSID row above the IP line.
fn narrow_info(icons: WifiIcons, wifi_signal: Option<i32>, ssid: &str, ip: &str) -> TreeNode {
    col(
        PropsData {
            gap: 8.0,
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
                    wifi_icon(icons, wifi_signal),
                    flex_text(22, WHITE, ssid, TextAlign::Left),
                ],
            ),
            text(ip, text_style(22, GRAY_50)),
        ],
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

    fn build(panel: Panel, view: WifiView<'_>) -> TreeNode {
        build_tree(
            55,
            Some("braiins-deck"),
            Some("10.0.0.2"),
            Some(-55),
            Some("MyWifi"),
            WifiIcons::default(),
            panel,
            view,
        )
    }

    /// Direct children of a container node, if any.
    fn children(node: &TreeNode) -> Option<&[TreeNode]> {
        match node {
            TreeNode::Column(_, kids)
            | TreeNode::Row(_, kids)
            | TreeNode::Center(_, kids)
            | TreeNode::Scroll { children: kids, .. } => Some(kids),
            TreeNode::Paragraph { .. }
            | TreeNode::Button { .. }
            | TreeNode::Spacer { .. }
            | TreeNode::Canvas { .. }
            | TreeNode::Notification { .. }
            | TreeNode::Modal { .. }
            | TreeNode::ProgressBar { .. } => None,
        }
    }

    /// Recursively collect every `ProgressBar` fraction in the tree.
    fn slider_fractions(node: &TreeNode, out: &mut Vec<f32>) {
        if let TreeNode::ProgressBar { fraction, .. } = node {
            out.push(*fraction);
        }
        if let Some(kids) = children(node) {
            for k in kids {
                slider_fractions(k, out);
            }
        }
    }

    /// Recursively collect every `Button` id in the tree.
    fn button_ids(node: &TreeNode, out: &mut Vec<String>) {
        if let TreeNode::Button { id, .. } = node {
            out.push(id.clone());
        }
        if let Some(kids) = children(node) {
            for k in kids {
                button_ids(k, out);
            }
        }
    }

    #[test]
    fn brightness_fraction_is_b_minus_ten_over_ninety() {
        // 55 -> (55-10)/90 = 0.5; the slider must carry exactly that.
        let mut fracs = Vec::new();
        slider_fractions(
            &build(wide_panel(), WifiView::Idle { label: "x" }),
            &mut fracs,
        );
        assert_eq!(fracs.len(), 1, "exactly one brightness slider");
        assert!((fracs[0] - 0.5).abs() < 1e-6, "got {}", fracs[0]);
    }

    #[test]
    fn round_panel_roots_a_column() {
        assert!(matches!(
            build(round_panel(), WifiView::Idle { label: "x" }),
            TreeNode::Column(..)
        ));
    }

    #[test]
    fn wide_and_narrow_both_root_a_column() {
        assert!(matches!(
            build(wide_panel(), WifiView::Idle { label: "x" }),
            TreeNode::Column(..)
        ));
        assert!(matches!(
            build(narrow_panel(), WifiView::Idle { label: "x" }),
            TreeNode::Column(..)
        ));
    }

    #[test]
    fn wifi_buttons_flag_gates_the_button_row() {
        let with = |wifi_buttons| {
            let mut panel = wide_panel();
            panel.wifi_buttons = wifi_buttons;
            let mut ids = Vec::new();
            button_ids(&build(panel, WifiView::Idle { label: "x" }), &mut ids);
            ids
        };
        let enabled = with(true);
        assert!(enabled.iter().any(|id| id == WIFI_RECONFIG_KEY));
        assert!(enabled.iter().any(|id| id == WIFI_RECONNECT_KEY));

        assert!(with(false).is_empty());
    }

    #[test]
    fn setup_view_has_no_buttons() {
        let mut ids = Vec::new();
        button_ids(
            &build(
                wide_panel(),
                WifiView::Setup {
                    ap_ssid: "Deck ABCD",
                },
            ),
            &mut ids,
        );
        assert!(ids.is_empty());
    }

    #[test]
    fn short_hostname_is_unchanged() {
        assert_eq!(
            fit_hostname("braiins-deck", ROUND_HOSTNAME_WIDTH),
            "braiins-deck"
        );
    }

    #[test]
    fn long_hostname_is_truncated_with_ellipsis() {
        let long = "braiins-deck-extremely-long-hostname-xyz";
        let fitted = fit_hostname(long, ROUND_HOSTNAME_WIDTH);
        assert!(fitted.ends_with('…'));
        assert!(fitted.chars().count() < long.chars().count());
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
}
