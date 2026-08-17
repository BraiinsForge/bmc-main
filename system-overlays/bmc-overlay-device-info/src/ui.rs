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

//! Screen composition for the device-info overlay: a pure view enum plus tree
//! builders, so every screen is renderable from a plain value (the gallery
//! renders the same trees the device does).

use std::net::Ipv4Addr;
use std::time::Instant;

use bmc_render::colors::{BLACK, GRAY_40, VIOLET_50, WHITE};
use bmc_render::renderer::Renderer;
use bmc_render::tree::{
    DrawCommand, FontFamily, FontWeight, TextAlign, TextStyle, TreeNode, col, fixed_height, row,
    text,
};
use bmc_system_overlay::{AccessPoint, TreeUi};
use bmc_wasm_protocol::{CrossAlign, Fill, GRAY_60, Justify, PropsData, TRANSPARENT};

use crate::icons::{DeviceInfoIcons, Icon};

/// What the overlay shows, derived from the FSM. Pure data so the gallery can
/// render every screen without a compositor or prober.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceInfoView {
    /// First-boot / reconfiguration AP screen; `None` while the AP is still
    /// coming up.
    SetupStart {
        ap: Option<AccessPoint>,
    },
    SetupConnecting {
        ssid: Option<String>,
    },
    SetupConnected {
        ssid: Option<String>,
    },
    /// Setup connect-info: the device address as text and QR. `ip` is `None`
    /// while the station address is still being assigned, and the screen falls
    /// back to the connect progress for `ssid`.
    SetupConnectInfo {
        ip: Option<Ipv4Addr>,
        ssid: Option<String>,
    },
    SetupCompleted,
    SetupError,
    /// Unrecoverable setup error; bmc recovers on its own (reset or reboot).
    SetupFatal,
    /// Operational-boot connect progress.
    Connecting {
        ssid: Option<String>,
    },
    /// Operational-boot connect info.
    Success {
        ip: Ipv4Addr,
    },
    Failed {
        ssid: Option<String>,
    },
    /// Renders nothing (unmapped).
    Done,
}

/// Persistent render caches for the overlay's tree UI.
pub struct DeviceInfoRenderState {
    tree: TreeUi,
    /// Registered lazily on the first render (or eagerly by `prewarm`),
    /// so the gallery needs no separate warm-up path.
    icons: Option<DeviceInfoIcons>,
    last_render: Instant,
}

impl std::fmt::Debug for DeviceInfoRenderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceInfoRenderState")
            .field("icons", &self.icons)
            .field("last_render", &self.last_render)
            .finish_non_exhaustive()
    }
}

impl DeviceInfoRenderState {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            tree: TreeUi::default(),
            icons: None,
            last_render: now,
        }
    }

    pub fn ensure_icons(&mut self, renderer: &mut dyn Renderer) -> DeviceInfoIcons {
        *self
            .icons
            .get_or_insert_with(|| crate::icons::register_icons(renderer))
    }
}

const EYEBROW: &str = "Initial Setup";
const SETUP_START_TITLE: &str = "Connect to Braiins Deck WiFi";
const SETUP_AP_PENDING_TITLE: &str = "Starting setup WiFi...";
/// Shared by the setup flow's two connect-progress screens and the operational
/// one, so the wording cannot drift between the flows.
const CONNECTING_TITLE: &str = "Connecting to WiFi...";
const QR_SIZE: f32 = 336.0;
/// The QR in a column that carries its own text: small enough that the headline
/// above it and the address below still fit the panel's height.
const QR_SIZE_COLUMN: f32 = 224.0;
/// Modules of white border around the QR (the legacy white plate).
const QR_QUIET_ZONE: u8 = 4;
const DEV_DECK_ICON_MARGIN: f32 = 30.0;
/// Breathing room above the first line of a screen, and between its blocks.
const SCREEN_TOP_INSET: f32 = 40.0;
const SCREEN_GAP: f32 = 20.0;
/// Gap between a label and the value it introduces, tighter than the gap
/// separating the surrounding blocks.
const LABEL_GAP: f32 = 8.0;
/// Gap between a split screen's columns and the rule between them.
const COLUMN_GAP: f32 = 40.0;
/// Widest artwork a split screen's column holds: half the panel, less the rule
/// and the gaps around it.
const COLUMN_ICON_WIDTH: f32 = 520.0;
/// Width of the QR column, so the code and its text keep their proportions
/// whatever the column beside them holds.
const QR_COLUMN_WIDTH: f32 = 480.0;
/// Gap between a QR column's two headline lines, tight enough that the pair
/// reads as one sentence rather than two blocks.
const HEADLINE_GAP: f32 = 4.0;
/// Height of the rule between two columns: most of the panel, not a tick.
const SEPARATOR_HEIGHT: f32 = 336.0;

fn style(
    size: u32,
    color: bmc_render::colors::Color,
    weight: FontWeight,
    align: TextAlign,
) -> TextStyle {
    TextStyle {
        size,
        color,
        weight,
        align,
        family: FontFamily::DeckSans,
        line_height: 1.2,
        ..TextStyle::default()
    }
}

fn eyebrow() -> TreeNode {
    text(
        EYEBROW,
        style(24, GRAY_40, FontWeight::REGULAR, TextAlign::Center),
    )
}

fn title(t: &str, align: TextAlign) -> TreeNode {
    text(t, style(40, WHITE, FontWeight::SEMIBOLD, align))
}

/// A title one step down the scale, for a heading that sits beside a screen's
/// own without competing with it.
fn title_small(t: &str, align: TextAlign) -> TreeNode {
    text(t, style(30, WHITE, FontWeight::SEMIBOLD, align))
}

fn content(t: &str, align: TextAlign) -> TreeNode {
    text(t, style(24, GRAY_40, FontWeight::REGULAR, align))
}

fn subtitle(t: &str, align: TextAlign) -> TreeNode {
    text(t, style(36, VIOLET_50, FontWeight::SEMIBOLD, align))
}

/// Draws an icon at the size it was authored at: the assets are drawn
/// at display scale, and the host stretches whatever box it is given.
fn icon(icon: Icon) -> TreeNode {
    icon_within(icon, icon.size.0)
}

/// `icon` shrunk to `max_width` when the artwork is wider than its column.
/// The host scales the axes independently, so the box carries the aspect ratio.
fn icon_within(icon: Icon, max_width: f32) -> TreeNode {
    let (authored_width, authored_height) = icon.size;
    let scale = (max_width / authored_width).min(1.0);
    let (width, height) = (authored_width * scale, authored_height * scale);
    TreeNode::Canvas {
        props: PropsData {
            width,
            height,
            margin: 12.0,
            ..PropsData::default()
        },
        touch_key: None,
        // TRANSPARENT keeps the SVG's own colors instead of tinting it.
        draws: vec![DrawCommand::Svg {
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            color: TRANSPARENT,
            icon_id: icon.id,
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// A QR code on its white plate: the quiet zone is painted in the light color,
/// which reproduces the legacy white square without a separate rect.
fn qr(payload: &str, size: f32) -> TreeNode {
    TreeNode::Canvas {
        props: PropsData {
            width: size,
            height: size,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Qr {
            x: 0.0,
            y: 0.0,
            size,
            dark: BLACK,
            light: WHITE,
            quiet_zone: QR_QUIET_ZONE,
            text: payload.to_owned(),
        }],
    }
}

fn spacer() -> TreeNode {
    col(
        PropsData {
            flex: 1.0,
            ..PropsData::default()
        },
        [],
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

/// The black backdrop a screen is laid out on, filled by its columns.
fn screen(columns: Vec<TreeNode>) -> TreeNode {
    row(
        PropsData {
            background: BLACK,
            gap: COLUMN_GAP,
            ..PropsData::default()
        },
        columns,
    )
}

/// One full-height column of a screen: its blocks packed under the top inset,
/// or centered in the full height.
fn screen_column(justify: Justify, children: Vec<TreeNode>) -> TreeNode {
    let mut nodes = match justify {
        Justify::Start => vec![fixed_height(SCREEN_TOP_INSET)],
        Justify::Center | Justify::End | Justify::SpaceBetween => Vec::new(),
    };
    nodes.extend(children);
    col(
        PropsData {
            flex: 1.0,
            cross_align: CrossAlign::Center,
            justify_content: justify,
            gap: SCREEN_GAP,
            ..PropsData::default()
        },
        nodes,
    )
}

/// A label stacked tightly above the value it introduces.
fn labeled(label: &str, value: TreeNode) -> TreeNode {
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: LABEL_GAP,
            ..PropsData::default()
        },
        [content(label, TextAlign::Center), value],
    )
}

fn ssid_line(ssid: &str) -> TreeNode {
    labeled(
        "WiFi SSID",
        text(
            ssid,
            style(36, VIOLET_50, FontWeight::REGULAR, TextAlign::Center),
        ),
    )
}

fn ssid_lines(ssid: Option<&str>) -> Vec<TreeNode> {
    match ssid {
        Some(ssid) => vec![ssid_line(ssid)],
        None => vec![content("Waiting for WiFi connection", TextAlign::Center)],
    }
}

/// The icon + text stack both templates are built from.
fn template_children(
    show_eyebrow: bool,
    icon_node: TreeNode,
    title_text: &str,
    lines: Vec<TreeNode>,
) -> Vec<TreeNode> {
    let mut children = Vec::new();
    if show_eyebrow {
        children.push(eyebrow());
    }
    children.push(icon_node);
    children.push(title(title_text, TextAlign::Center));
    children.extend(lines);
    children
}

/// The centered icon + text template shared by the simple screens.
fn template_tree(
    justify: Justify,
    show_eyebrow: bool,
    icon_id: Icon,
    title_text: &str,
    lines: Vec<TreeNode>,
) -> TreeNode {
    screen(vec![screen_column(
        justify,
        template_children(show_eyebrow, icon(icon_id), title_text, lines),
    )])
}

/// What the QR column beside a template's text says.
#[derive(Debug, Clone, Copy)]
struct QrColumn<'a> {
    headline: [&'a str; 2],
    /// Encoded in the code, and spelled out under it.
    url: &'a str,
}

/// `template_tree` with a second column to its right, on the far side of a rule:
/// the same icon + text stack, and a QR under its own headline beside it.
fn template_tree_with_qr(
    justify: Justify,
    show_eyebrow: bool,
    icon_id: Icon,
    title_text: &str,
    lines: Vec<TreeNode>,
    column: QrColumn<'_>,
) -> TreeNode {
    /// Keeps the outer columns off the panel edges, on top of `COLUMN_GAP`.
    const EDGE_INSET: f32 = 8.0;

    screen(vec![
        fixed_width(EDGE_INSET),
        screen_column(
            justify,
            template_children(
                show_eyebrow,
                icon_within(icon_id, COLUMN_ICON_WIDTH),
                title_text,
                lines,
            ),
        ),
        vertical_separator(),
        qr_column(column),
        fixed_width(EDGE_INSET),
    ])
}

/// The QR column, `QR_COLUMN_WIDTH` wide: a headline, the code,
/// and the address it encodes.
/// Centered rather than packed, since its stack leaves no room for a top inset.
fn qr_column(column: QrColumn<'_>) -> TreeNode {
    let headline = col(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: HEADLINE_GAP,
            ..PropsData::default()
        },
        column
            .headline
            .map(|line| title_small(line, TextAlign::Center)),
    );
    row(
        PropsData {
            width: QR_COLUMN_WIDTH,
            ..PropsData::default()
        },
        [screen_column(
            Justify::Center,
            vec![
                headline,
                qr(column.url, QR_SIZE_COLUMN),
                labeled(
                    "Or open directly in browser:",
                    content(column.url, TextAlign::Center),
                ),
            ],
        )],
    )
}

/// The rule between the two columns of a split screen, centered on the panel.
fn vertical_separator() -> TreeNode {
    col(
        PropsData {
            justify_content: Justify::Center,
            ..PropsData::default()
        },
        [TreeNode::Canvas {
            props: PropsData {
                width: 1.0,
                height: SEPARATOR_HEIGHT,
                ..PropsData::default()
            },
            touch_key: None,
            draws: vec![DrawCommand::Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: SEPARATOR_HEIGHT,
                fill: Fill::Solid(GRAY_60),
            }],
        }],
    )
}

fn connected_info_tree(icon_id: Icon, title_text: &str, ip: Ipv4Addr) -> TreeNode {
    const HORIZONTAL_SPACE: f32 = 120.0;

    let url = format!("http://{ip}/");

    let logo = col(
        PropsData {
            margin: DEV_DECK_ICON_MARGIN,
            ..PropsData::default()
        },
        [icon(icon_id)],
    );

    let text_section = col(
        PropsData::default(),
        [
            logo,
            title(title_text, TextAlign::Left),
            subtitle(&url, TextAlign::Left),
            title("or scan the QR code", TextAlign::Left),
        ],
    );

    let qr_section = col(PropsData::default(), [qr(&url, QR_SIZE)]);

    row(
        PropsData {
            background: BLACK,
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        [
            fixed_width(HORIZONTAL_SPACE),
            text_section,
            spacer(),
            qr_section,
            fixed_width(HORIZONTAL_SPACE),
        ],
    )
}

/// The setup flow's connect progress: shown while joining the chosen network,
/// and again while its station address is still pending.
fn setup_connecting_tree(wifi: Icon, ssid: Option<&str>) -> TreeNode {
    template_tree(
        Justify::Start,
        true,
        wifi,
        CONNECTING_TITLE,
        ssid_lines(ssid),
    )
}

#[must_use]
pub fn build_device_info_tree(view: &DeviceInfoView, icons: DeviceInfoIcons) -> Option<TreeNode> {
    let tree = match view {
        DeviceInfoView::SetupStart { ap } => match ap {
            Some(ap) => template_tree_with_qr(
                Justify::Start,
                true,
                icons.wifi_connect,
                SETUP_START_TITLE,
                vec![ssid_line(&ap.ssid)],
                QrColumn {
                    headline: ["Connected but nothing happens?", "Scan the code!"],
                    url: &ap.setup_url,
                },
            ),
            None => template_tree(
                Justify::Start,
                true,
                icons.wifi_connect,
                SETUP_AP_PENDING_TITLE,
                Vec::new(),
            ),
        },
        DeviceInfoView::SetupConnecting { ssid } => {
            setup_connecting_tree(icons.wifi, ssid.as_deref())
        }
        DeviceInfoView::SetupConnected { ssid } => template_tree(
            Justify::Start,
            true,
            icons.wifi,
            "Your Braiins Deck is connected!",
            ssid_lines(ssid.as_deref()),
        ),
        DeviceInfoView::SetupConnectInfo { ip, ssid } => {
            if let Some(ip) = ip {
                connected_info_tree(icons.desktop_clock, "Complete the setup\nby accessing", *ip)
            } else {
                setup_connecting_tree(icons.wifi, ssid.as_deref())
            }
        }
        DeviceInfoView::SetupCompleted => template_tree(
            Justify::Start,
            true,
            icons.success,
            "Braiins Deck is ready!",
            vec![content("Login to continue.", TextAlign::Center)],
        ),
        DeviceInfoView::SetupError => template_tree(
            Justify::Start,
            true,
            icons.wifi_error,
            "Could not connect. Please try again.",
            Vec::new(),
        ),
        DeviceInfoView::SetupFatal => template_tree(
            Justify::Center,
            false,
            icons.refresh,
            "Problem Occurred. Restarting Braiins Deck.",
            Vec::new(),
        ),
        DeviceInfoView::Connecting { ssid } => {
            let mut lines = ssid_lines(ssid.as_deref());
            lines.push(content("Waiting for IP address", TextAlign::Center));
            template_tree(Justify::Center, false, icons.wifi, CONNECTING_TITLE, lines)
        }
        DeviceInfoView::Success { ip } => {
            connected_info_tree(icons.desktop_clock, "Access the device at", *ip)
        }
        DeviceInfoView::Failed { ssid } => {
            let mut lines = ssid_lines(ssid.as_deref());
            lines.push(content("No IP address assigned", TextAlign::Center));
            template_tree(
                Justify::Center,
                false,
                icons.wifi_error,
                "Problem with connection",
                lines,
            )
        }
        DeviceInfoView::Done => return None,
    };
    Some(tree)
}

pub fn render_device_info(
    r: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut DeviceInfoRenderState,
    view: &DeviceInfoView,
) {
    let icons = state.ensure_icons(r);
    let now = Instant::now();
    let delta_ms = u32::try_from(now.saturating_duration_since(state.last_render).as_millis())
        .unwrap_or(u32::MAX);
    state.last_render = now;

    let Some(tree) = build_device_info_tree(view, icons) else {
        return;
    };
    if let Err(err) = state.tree.render(&tree, size, delta_ms, r) {
        tracing::error!("device-info tree render failed: {err}");
    }
}
