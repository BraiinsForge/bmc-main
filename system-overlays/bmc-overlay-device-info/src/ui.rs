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

/// Which uplinks the board offers, as far as the screens care.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Uplinks {
    pub wifi: bool,
    pub ethernet: bool,
}

impl Uplinks {
    /// The Deck: what the screens were written for,
    /// and what a board that cannot say what it has is taken to be.
    pub const WIFI_ONLY: Self = Self {
        wifi: true,
        ethernet: false,
    };
}

/// The connection a screen waits on or reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A Wi-Fi join, in flight or configured; the SSID once it is known.
    Wifi { ssid: Option<String> },
    /// No Wi-Fi join is under way: the device waits on its ethernet port.
    Cable,
}

/// What the overlay shows, derived from the FSM. Pure data so the gallery can
/// render every screen without a compositor or prober.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceInfoView {
    /// First-boot / reconfiguration AP screen; `None` while the AP is still
    /// coming up, when `uplinks` decides what the user is told to do instead.
    SetupStart {
        ap: Option<AccessPoint>,
        uplinks: Uplinks,
    },
    SetupConnecting {
        link: Link,
    },
    TurningApOff,
    SetupConnected {
        ssid: Option<String>,
    },
    /// Setup connect-info: the device address as text and QR. `ip` is `None`
    /// while the address is still being assigned, and the screen falls
    /// back to the connect progress for `link`.
    SetupConnectInfo {
        ip: Option<Ipv4Addr>,
        link: Link,
    },
    SetupCompleted,
    SetupError,
    /// Setup failure the overlay cannot clear on its own.
    /// `restarting` says whether bmc is restarting the device,
    /// i.e. whether the screen waits it out or asks the user to act.
    /// `dismissible` says whether the setup is done and the screen may close;
    /// it decides the close glyph.
    SetupFatal {
        restarting: bool,
        dismissible: bool,
    },
    /// Post-firmware-upgrade success, the operational flow's opening screen.
    UpgradeSuccess,
    /// Operational-boot connect progress.
    Connecting {
        link: Link,
    },
    /// Operational-boot connect info.
    Success {
        ip: Ipv4Addr,
    },
    Failed {
        link: Link,
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
const SETUP_AP_PENDING_TITLE: &str = "Starting setup Wi-Fi...";
/// Shared by the setup flow's two connect-progress screens and the operational
/// one, so the wording cannot drift between the flows.
const CONNECTING_TITLE: &str = "Connecting to Wi-Fi...";
/// The same screens where no Wi-Fi join is under way.
const CONNECTING_CABLE_TITLE: &str = "Connecting to network...";
const WAITING_FOR_IP: &str = "Waiting for IP address";
/// What a board with an ethernet port is told while its setup AP is not up.
const CABLE_HINT: &str = "or connect the Ethernet cable";
const CONNECT_CABLE_TITLE: &str = "Connect the Ethernet cable";
const NO_NETWORK_TITLE: &str = "No network connection";
const CHECK_CABLE: &str = "Check the Ethernet cable";
const UPGRADE_SUCCESS_TITLE: &str = "Update Finished";
/// Shared by both setup-failure screens, which differ only in what follows.
const SETUP_FATAL_TITLE: &str = "Problem Occurred";
/// Modules of white border around the QR (the legacy white plate).
const QR_QUIET_ZONE: u8 = 4;
/// Tap target around the close glyph, and the glyph inside it.
/// Both match the settings tray, which is the other overlay a touch closes.
const CLOSE_TARGET: f32 = 48.0;
const CLOSE_GLYPH: f32 = 24.0;
/// Keeps the corner affordances off the panel's edges.
const CORNER_INSET: f32 = 24.0;
/// Legacy `connect_info` spacing between the tray hint's parts.
const TRAY_HINT_GAP: f32 = 8.0;
/// Gap between a QR column's two headline lines, tight enough that the pair
/// reads as one sentence rather than two blocks.
const HEADLINE_GAP: f32 = 4.0;

/// Widest artwork a screen's leading icon may draw, one field per group of screens that share a size.
/// The groups follow what a screen puts around the icon, not the panel:
/// a column sharing the panel with a QR takes less than a screen holding the panel on its own.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IconWidths {
    /// The setup screen once the AP is up, whose icon column stands beside the
    /// QR column and so has the least room of any screen's.
    ap_up: f32,
    /// The address-and-QR screens: setup over a cable, the setup connect info
    /// and the operational connect info.
    connect_info: f32,
    /// The same setup screen while the AP is still coming up, which holds the
    /// panel on its own and so fits far more.
    ap_pending: f32,
    /// Every other screen, all of them the icon + text template.
    other: f32,
}

/// Which registered artwork the setup screen leads with once the AP is up.
/// [`IconWidths::ap_up`] says how wide it draws. The pending variant of the
/// same screen makes no such choice: it holds the panel on its own, so the
/// authored illustration fits whatever the panel.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SetupApIcon {
    /// The Wi-Fi join illustration the screen was authored with.
    WifiConnect,
    /// The plain Wi-Fi glyph the connect-progress screens lead with.
    Wifi,
}

impl SetupApIcon {
    fn pick(self, icons: DeviceInfoIcons) -> Icon {
        match self {
            Self::WifiConnect => icons.wifi_connect,
            Self::Wifi => icons.wifi,
        }
    }
}

/// Size-dependent metrics. The screens were authored for the Deck's 1280x480
/// panel; the compact tier refits the same layouts to small panels such as
/// the BMM101's 480x320.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tier {
    eyebrow: u32,
    title: u32,
    title_small: u32,
    subtitle: u32,
    content: u32,
    /// The standalone QR beside the connect-info text.
    qr: f32,
    /// The QR in a column that carries its own text: small enough that the
    /// headline above it and the address below still fit the panel's height.
    qr_column: f32,
    /// Width of the QR column, so the code and its text keep their proportions
    /// whatever the column beside them holds.
    qr_column_width: f32,
    icon_width: IconWidths,
    /// The artwork the setup screen leads with once the AP is up,
    /// a per-panel choice like the sizes.
    ap_up_icon: SetupApIcon,
    icon_margin: f32,
    /// Breathing room above the first line of a screen, and between its blocks.
    top_inset: f32,
    gap: f32,
    /// Gap between a label and the value it introduces, tighter than the gap
    /// separating the surrounding blocks.
    label_gap: f32,
    /// Gap between a split screen's columns and the rule between them.
    column_gap: f32,
    /// Height of the rule between two columns: most of the panel, not a tick.
    separator_height: f32,
    /// Edge inset either side of the connect-info screen's content.
    horizontal_space: f32,
}

const TIER_WIDE: Tier = Tier {
    eyebrow: 24,
    title: 40,
    title_small: 30,
    subtitle: 36,
    content: 24,
    qr: 336.0,
    qr_column: 224.0,
    qr_column_width: 480.0,
    icon_width: IconWidths {
        ap_up: 520.0,
        connect_info: f32::MAX,
        ap_pending: f32::MAX,
        other: f32::MAX,
    },
    ap_up_icon: SetupApIcon::WifiConnect,
    icon_margin: 30.0,
    top_inset: 40.0,
    gap: 20.0,
    label_gap: 8.0,
    column_gap: 40.0,
    separator_height: 336.0,
    horizontal_space: 120.0,
};

const TIER_COMPACT: Tier = Tier {
    eyebrow: 14,
    title: 22,
    title_small: 16,
    subtitle: 20,
    content: 14,
    qr: 150.0,
    qr_column: 120.0,
    qr_column_width: 190.0,
    icon_width: IconWidths {
        ap_up: 30.0,
        connect_info: 150.0,
        ap_pending: 350.0,
        other: 80.0,
    },
    ap_up_icon: SetupApIcon::Wifi,
    icon_margin: 8.0,
    top_inset: 50.0,
    gap: 8.0,
    label_gap: 4.0,
    column_gap: 16.0,
    separator_height: 180.0,
    horizontal_space: 24.0,
};

/// The Deck's authored layout above the tray's wide threshold,
/// the compact refit below it.
pub(crate) fn tier_for(size: (u32, u32)) -> Tier {
    if size.0 >= 960 {
        TIER_WIDE
    } else {
        TIER_COMPACT
    }
}

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

fn eyebrow(tier: Tier) -> TreeNode {
    text(
        EYEBROW,
        style(
            tier.eyebrow,
            GRAY_40,
            FontWeight::REGULAR,
            TextAlign::Center,
        ),
    )
}

fn title(tier: Tier, t: &str, align: TextAlign) -> TreeNode {
    text(t, style(tier.title, WHITE, FontWeight::SEMIBOLD, align))
}

/// A title one step down the scale, for a heading that sits beside a screen's
/// own without competing with it.
fn title_small(tier: Tier, t: &str, align: TextAlign) -> TreeNode {
    text(
        t,
        style(tier.title_small, WHITE, FontWeight::SEMIBOLD, align),
    )
}

fn content(tier: Tier, t: &str, align: TextAlign) -> TreeNode {
    text(t, style(tier.content, GRAY_40, FontWeight::REGULAR, align))
}

fn subtitle(tier: Tier, t: &str, align: TextAlign) -> TreeNode {
    text(
        t,
        style(tier.subtitle, VIOLET_50, FontWeight::SEMIBOLD, align),
    )
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
fn screen(tier: Tier, columns: Vec<TreeNode>) -> TreeNode {
    row(
        PropsData {
            background: BLACK,
            gap: tier.column_gap,
            ..PropsData::default()
        },
        columns,
    )
}

/// The close glyph in the top-right, absolutely positioned so it sits outside
/// the column layout and shifts nothing.
///
/// It carries no touch key: the whole screen already dismisses, and a target
/// that swallowed the touch would make the rest of the screen do nothing.
/// It is here so the screen says it can be closed.
fn close_affordance(icon_id: Icon) -> TreeNode {
    let glyph_inset = (CLOSE_TARGET - CLOSE_GLYPH) / 2.0;
    TreeNode::Canvas {
        props: PropsData {
            width: CLOSE_TARGET,
            height: CLOSE_TARGET,
            inset_top: CORNER_INSET,
            inset_right: CORNER_INSET,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Svg {
            x: glyph_inset,
            y: glyph_inset,
            w: CLOSE_GLYPH,
            h: CLOSE_GLYPH,
            color: TRANSPARENT,
            icon_id: icon_id.id,
            anti_alias: true,
            fills: Vec::new(),
        }],
    }
}

/// The settings-tray hint in the bottom-right, absolutely positioned so it sits
/// outside the column layout and shifts nothing. Carried over from the legacy
/// `connect_info` screen, which said the same thing about a swipe up.
fn tray_hint(tier: Tier, icon_id: Icon) -> TreeNode {
    let (glyph_width, glyph_height) = icon_id.size;
    let glyph = TreeNode::Canvas {
        props: PropsData {
            width: glyph_width,
            height: glyph_height,
            ..PropsData::default()
        },
        touch_key: None,
        draws: vec![DrawCommand::Svg {
            x: 0.0,
            y: 0.0,
            w: glyph_width,
            h: glyph_height,
            color: TRANSPARENT,
            icon_id: icon_id.id,
            anti_alias: true,
            fills: Vec::new(),
        }],
    };
    row(
        PropsData {
            // The close target's own band, so the two corner affordances
            // sit on one line however tall the hint's own content is.
            height: CLOSE_TARGET,
            cross_align: CrossAlign::Center,
            gap: TRAY_HINT_GAP,
            inset_top: CORNER_INSET,
            inset_left: CORNER_INSET,
            ..PropsData::default()
        },
        [
            content(
                tier,
                "To access the controls, IP and Wi-Fi info",
                TextAlign::Right,
            ),
            text(
                "swipe down",
                style(tier.content, GRAY_40, FontWeight::BOLD, TextAlign::Right),
            ),
            glyph,
        ],
    )
}

/// Whether a touch anywhere on `view` closes it, which is what the close glyph
/// advertises.
///
/// The setup screens ignore touch, and the post-upgrade screen hands over
/// to the connect flow rather than closing, so on either an X would promise
/// something that does not happen. Kept beside the screens
/// rather than derived from the FSM so adding a view forces an answer here.
fn dismisses_on_touch(view: &DeviceInfoView) -> bool {
    match view {
        DeviceInfoView::Connecting { .. }
        | DeviceInfoView::Success { .. }
        | DeviceInfoView::Failed { .. } => true,
        // Only once the setup is done; the FSM decides, since the answer
        // turns on a lifecycle state no view carries.
        DeviceInfoView::SetupFatal { dismissible, .. } => *dismissible,
        DeviceInfoView::SetupStart { .. }
        | DeviceInfoView::TurningApOff
        | DeviceInfoView::SetupConnecting { .. }
        | DeviceInfoView::SetupConnected { .. }
        | DeviceInfoView::SetupConnectInfo { .. }
        | DeviceInfoView::SetupCompleted
        | DeviceInfoView::SetupError
        | DeviceInfoView::UpgradeSuccess
        | DeviceInfoView::Done => false,
    }
}

/// Put an absolutely-positioned node on a screen, outside its column layout.
fn overlaid(tree: TreeNode, node: TreeNode) -> TreeNode {
    let TreeNode::Row(props, mut columns) = tree else {
        panic!("BUG: every screen is rooted in a row");
    };
    columns.push(node);
    TreeNode::Row(props, columns)
}

/// Put the close glyph on a screen, outside its column layout.
fn dismissable(tree: TreeNode, icon_id: Icon) -> TreeNode {
    overlaid(tree, close_affordance(icon_id))
}

/// One full-height column of a screen: its blocks packed under the top inset,
/// or centered in the full height.
fn screen_column(tier: Tier, justify: Justify, children: Vec<TreeNode>) -> TreeNode {
    let mut nodes = match justify {
        Justify::Start => vec![fixed_height(tier.top_inset)],
        Justify::Center | Justify::End | Justify::SpaceBetween => Vec::new(),
    };
    nodes.extend(children);
    col(
        PropsData {
            flex: 1.0,
            cross_align: CrossAlign::Center,
            justify_content: justify,
            gap: tier.gap,
            ..PropsData::default()
        },
        nodes,
    )
}

/// A label stacked tightly above the value it introduces.
fn labeled(tier: Tier, label: &str, value: TreeNode) -> TreeNode {
    col(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: tier.label_gap,
            ..PropsData::default()
        },
        [content(tier, label, TextAlign::Center), value],
    )
}

fn ssid_line(tier: Tier, ssid: &str) -> TreeNode {
    labeled(
        tier,
        "Wi-Fi SSID",
        text(
            ssid,
            style(
                tier.subtitle,
                VIOLET_50,
                FontWeight::REGULAR,
                TextAlign::Center,
            ),
        ),
    )
}

fn ssid_lines(tier: Tier, ssid: Option<&str>) -> Vec<TreeNode> {
    match ssid {
        Some(ssid) => vec![ssid_line(tier, ssid)],
        None => vec![content(
            tier,
            "Waiting for Wi-Fi connection",
            TextAlign::Center,
        )],
    }
}

/// The icon + text stack both templates are built from.
fn template_children(
    tier: Tier,
    show_eyebrow: bool,
    icon_node: TreeNode,
    title_text: &str,
    lines: Vec<TreeNode>,
) -> Vec<TreeNode> {
    let mut children = Vec::new();
    if show_eyebrow {
        children.push(eyebrow(tier));
    }
    children.push(icon_node);
    children.push(title(tier, title_text, TextAlign::Center));
    children.extend(lines);
    children
}

/// The icon + text template shared by the simple screens.
fn template_tree(
    tier: Tier,
    justify: Justify,
    show_eyebrow: bool,
    icon_id: Icon,
    title_text: &str,
    lines: Vec<TreeNode>,
) -> TreeNode {
    template_tree_with_icon_width(
        tier,
        tier.icon_width.other,
        justify,
        show_eyebrow,
        icon_id,
        title_text,
        lines,
    )
}

/// `template_tree` for a screen that sizes its artwork on its own.
fn template_tree_with_icon_width(
    tier: Tier,
    icon_width: f32,
    justify: Justify,
    show_eyebrow: bool,
    icon_id: Icon,
    title_text: &str,
    lines: Vec<TreeNode>,
) -> TreeNode {
    screen(
        tier,
        vec![screen_column(
            tier,
            justify,
            template_children(
                tier,
                show_eyebrow,
                icon_within(icon_id, icon_width),
                title_text,
                lines,
            ),
        )],
    )
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
/// The setup screen with its AP up is the only screen laid out this way,
/// so it is that screen's icon width the stack draws within.
fn template_tree_with_qr(
    tier: Tier,
    justify: Justify,
    show_eyebrow: bool,
    icon_id: Icon,
    title_text: &str,
    lines: Vec<TreeNode>,
    column: QrColumn<'_>,
) -> TreeNode {
    /// Keeps the outer columns off the panel edges, on top of the column gap.
    const EDGE_INSET: f32 = 8.0;

    screen(
        tier,
        vec![
            fixed_width(EDGE_INSET),
            screen_column(
                tier,
                justify,
                template_children(
                    tier,
                    show_eyebrow,
                    icon_within(icon_id, tier.icon_width.ap_up),
                    title_text,
                    lines,
                ),
            ),
            vertical_separator(tier),
            qr_column(tier, column),
            fixed_width(EDGE_INSET),
        ],
    )
}

/// The QR column, `QR_COLUMN_WIDTH` wide: a headline, the code,
/// and the address it encodes.
/// Centered rather than packed, since its stack leaves no room for a top inset.
fn qr_column(tier: Tier, column: QrColumn<'_>) -> TreeNode {
    let headline = col(
        PropsData {
            cross_align: CrossAlign::Center,
            gap: HEADLINE_GAP,
            ..PropsData::default()
        },
        column
            .headline
            .map(|line| title_small(tier, line, TextAlign::Center)),
    );
    row(
        PropsData {
            width: tier.qr_column_width,
            ..PropsData::default()
        },
        [screen_column(
            tier,
            Justify::Center,
            vec![
                headline,
                qr(column.url, tier.qr_column),
                labeled(
                    tier,
                    "Or open directly in browser:",
                    content(tier, column.url, TextAlign::Center),
                ),
            ],
        )],
    )
}

/// The rule between the two columns of a split screen, centered on the panel.
fn vertical_separator(tier: Tier) -> TreeNode {
    col(
        PropsData {
            justify_content: Justify::Center,
            ..PropsData::default()
        },
        [TreeNode::Canvas {
            props: PropsData {
                width: 1.0,
                height: tier.separator_height,
                ..PropsData::default()
            },
            touch_key: None,
            draws: vec![DrawCommand::Rect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: tier.separator_height,
                fill: Fill::Solid(GRAY_60),
            }],
        }],
    )
}

fn connected_info_tree(tier: Tier, icon_id: Icon, title_text: &str, ip: Ipv4Addr) -> TreeNode {
    connected_info_url_tree(tier, icon_id, title_text, &format!("http://{ip}/"))
}

fn connected_info_url_tree(tier: Tier, icon_id: Icon, title_text: &str, url: &str) -> TreeNode {
    let logo = col(
        PropsData {
            margin: tier.icon_margin,
            ..PropsData::default()
        },
        [icon_within(icon_id, tier.icon_width.connect_info)],
    );

    let text_section = col(
        PropsData::default(),
        [
            logo,
            title(tier, title_text, TextAlign::Left),
            subtitle(tier, url, TextAlign::Left),
            title(tier, "or scan the QR code", TextAlign::Left),
        ],
    );

    let qr_section = col(PropsData::default(), [qr(url, tier.qr)]);

    row(
        PropsData {
            background: BLACK,
            cross_align: CrossAlign::Center,
            ..PropsData::default()
        },
        [
            fixed_width(tier.horizontal_space),
            text_section,
            spacer(),
            qr_section,
            fixed_width(tier.horizontal_space),
        ],
    )
}

/// Icon, title and detail lines of a connect-progress screen for `link`.
fn connecting_parts(
    tier: Tier,
    icons: DeviceInfoIcons,
    link: &Link,
) -> (Icon, &'static str, Vec<TreeNode>) {
    match link {
        Link::Wifi { ssid } => (
            icons.wifi,
            CONNECTING_TITLE,
            ssid_lines(tier, ssid.as_deref()),
        ),
        Link::Cable => (icons.ethernet, CONNECTING_CABLE_TITLE, Vec::new()),
    }
}

/// The setup flow's connect progress: shown while joining the chosen network,
/// and again while the uplink's address is still pending.
fn setup_connecting_tree(tier: Tier, icons: DeviceInfoIcons, link: &Link) -> TreeNode {
    let (icon_id, title_text, lines) = connecting_parts(tier, icons, link);
    template_tree(tier, Justify::Start, true, icon_id, title_text, lines)
}

/// The operational flow's connect progress: the setup screen's parts, centered
/// and saying what the device is still waiting for.
fn connecting_tree(tier: Tier, icons: DeviceInfoIcons, link: &Link) -> TreeNode {
    let (icon_id, title_text, mut lines) = connecting_parts(tier, icons, link);
    lines.push(content(tier, WAITING_FOR_IP, TextAlign::Center));
    template_tree(tier, Justify::Center, false, icon_id, title_text, lines)
}

/// First-boot / reconfiguration AP screen.
/// The pending variant shows while the AP is still coming up,
/// or asks for the cable where there is no AP to wait for.
fn setup_start_tree(
    tier: Tier,
    icons: DeviceInfoIcons,
    device_icon: Icon,
    device_name: &str,
    ap: Option<&AccessPoint>,
    uplinks: Uplinks,
) -> TreeNode {
    let cable_hint = || content(tier, CABLE_HINT, TextAlign::Center);
    match ap {
        // Setup over a wired uplink: no AP to join, so no WiFi column -
        // only the wizard address as text and QR code.
        Some(ap) if ap.ssid.is_empty() => {
            connected_info_url_tree(tier, device_icon, "Open the web browser at", &ap.setup_url)
        }
        Some(ap) => {
            let mut lines = vec![ssid_line(tier, &ap.ssid)];
            if uplinks.ethernet {
                lines.push(cable_hint());
            }
            template_tree_with_qr(
                tier,
                Justify::Start,
                true,
                tier.ap_up_icon.pick(icons),
                &format!("Connect to {device_name} Wi-Fi"),
                lines,
                QrColumn {
                    headline: ["Connected but nothing happens?", "Scan the code!"],
                    url: &ap.setup_url,
                },
            )
        }
        None if !uplinks.wifi => template_tree(
            tier,
            Justify::Start,
            true,
            device_icon,
            CONNECT_CABLE_TITLE,
            Vec::new(),
        ),
        // The authored illustration on every tier, unlike the screen above:
        // this variant holds the panel on its own, so there is nothing per-panel
        // to choose between.
        None => template_tree_with_icon_width(
            tier,
            tier.icon_width.ap_pending,
            Justify::Start,
            true,
            icons.wifi_connect,
            SETUP_AP_PENDING_TITLE,
            uplinks.ethernet.then(cable_hint).into_iter().collect(),
        ),
    }
}

/// Setup failure the overlay cannot clear: waiting out bmc's restart,
/// or asking the user for one.
fn setup_fatal_tree(
    tier: Tier,
    icons: DeviceInfoIcons,
    device_name: &str,
    restarting: bool,
) -> TreeNode {
    let (icon_id, line) = if restarting {
        (icons.refresh, format!("Restarting {device_name}..."))
    } else {
        (icons.error, format!("Restart {device_name} to try again"))
    };
    template_tree(
        tier,
        Justify::Center,
        false,
        icon_id,
        SETUP_FATAL_TITLE,
        vec![content(tier, &line, TextAlign::Center)],
    )
}

#[must_use]
pub fn build_device_info_tree(
    view: &DeviceInfoView,
    icons: DeviceInfoIcons,
    tier: Tier,
    device_name: &str,
    miner: bool,
) -> Option<TreeNode> {
    // The screens picture the device itself; a miner is not a desktop clock.
    let device_icon = if miner {
        icons.miner
    } else {
        icons.desktop_clock
    };
    let tree = match view {
        DeviceInfoView::SetupStart { ap, uplinks } => {
            setup_start_tree(tier, icons, device_icon, device_name, ap.as_ref(), *uplinks)
        }
        DeviceInfoView::TurningApOff => template_tree(
            tier,
            Justify::Start,
            true,
            icons.ethernet,
            "Your device is being set up...",
            Vec::new(),
        ),
        DeviceInfoView::SetupConnecting { link } => setup_connecting_tree(tier, icons, link),
        DeviceInfoView::SetupConnected { ssid } => template_tree(
            tier,
            Justify::Start,
            true,
            icons.wifi,
            &format!("Your {device_name} is connected!"),
            ssid_lines(tier, ssid.as_deref()),
        ),
        DeviceInfoView::SetupConnectInfo { ip, link } => {
            if let Some(ip) = ip {
                connected_info_tree(tier, device_icon, "Complete the setup\nby accessing", *ip)
            } else {
                setup_connecting_tree(tier, icons, link)
            }
        }
        DeviceInfoView::SetupCompleted => template_tree(
            tier,
            Justify::Start,
            true,
            icons.success,
            &format!("{device_name} is ready!"),
            vec![content(tier, "Login to continue", TextAlign::Center)],
        ),
        DeviceInfoView::SetupError => template_tree(
            tier,
            Justify::Start,
            true,
            icons.wifi_error,
            "Could not connect. Please try again.",
            Vec::new(),
        ),
        DeviceInfoView::SetupFatal { restarting, .. } => {
            setup_fatal_tree(tier, icons, device_name, *restarting)
        }
        DeviceInfoView::UpgradeSuccess => template_tree(
            tier,
            Justify::Center,
            false,
            icons.success,
            UPGRADE_SUCCESS_TITLE,
            Vec::new(),
        ),
        DeviceInfoView::Connecting { link } => connecting_tree(tier, icons, link),
        DeviceInfoView::Success { ip } => overlaid(
            connected_info_tree(tier, device_icon, "Access the device at", *ip),
            tray_hint(tier, icons.swipe_down),
        ),
        DeviceInfoView::Failed {
            link: Link::Wifi { ssid },
        } => {
            let mut lines = ssid_lines(tier, ssid.as_deref());
            lines.push(content(tier, "No IP address assigned", TextAlign::Center));
            template_tree(
                tier,
                Justify::Center,
                false,
                icons.wifi_error,
                "Problem with connection",
                lines,
            )
        }
        DeviceInfoView::Failed { link: Link::Cable } => template_tree(
            tier,
            Justify::Center,
            false,
            icons.ethernet_error,
            NO_NETWORK_TITLE,
            vec![content(tier, CHECK_CABLE, TextAlign::Center)],
        ),
        DeviceInfoView::Done => return None,
    };
    Some(if dismisses_on_touch(view) {
        dismissable(tree, icons.close)
    } else {
        tree
    })
}

pub fn render_device_info(
    r: &mut dyn Renderer,
    size: (u32, u32),
    state: &mut DeviceInfoRenderState,
    view: &DeviceInfoView,
    device_name: &str,
    miner: bool,
) {
    let icons = state.ensure_icons(r);
    let now = Instant::now();
    let delta_ms = u32::try_from(now.saturating_duration_since(state.last_render).as_millis())
        .unwrap_or(u32::MAX);
    state.last_render = now;

    let Some(tree) = build_device_info_tree(view, icons, tier_for(size), device_name, miner) else {
        return;
    };
    if let Err(err) = state.tree.render(&tree, size, delta_ms, r) {
        tracing::error!("device-info tree render failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccessPoint, DeviceInfoView, Link, SetupApIcon, TIER_WIDE, Tier, Uplinks,
        build_device_info_tree, dismisses_on_touch, tier_for,
    };

    /// The name and panel the screens were authored against.
    const DEVICE_NAME: &str = "Braiins Deck";
    const DECK_PANEL: (u32, u32) = (1_280, 480);
    use crate::icons::{DeviceInfoIcons, Icon};
    use bmc_render::tree::{DrawCommand, TreeNode};
    use bmc_wasm_protocol::SvgId;
    use std::net::Ipv4Addr;

    const IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 5);
    const SETUP_URL: &str = "http://10.0.0.21/";
    const SETUP_SSID: &str = "Deck setup";
    /// The BMM101 and the BMM100.
    const BOTH: Uplinks = Uplinks {
        wifi: true,
        ethernet: true,
    };
    const CABLE_ONLY: Uplinks = Uplinks {
        wifi: false,
        ethernet: true,
    };

    fn wifi(ssid: Option<&str>) -> Link {
        Link::Wifi {
            ssid: ssid.map(str::to_owned),
        }
    }

    fn setup_ap() -> AccessPoint {
        AccessPoint {
            ssid: SETUP_SSID.to_owned(),
            setup_url: SETUP_URL.to_owned(),
        }
    }

    /// Every view the gallery has a cell for, `Done` included.
    fn all_views() -> Vec<DeviceInfoView> {
        vec![
            DeviceInfoView::SetupStart {
                ap: None,
                uplinks: Uplinks::WIFI_ONLY,
            },
            DeviceInfoView::SetupStart {
                ap: None,
                uplinks: BOTH,
            },
            DeviceInfoView::SetupStart {
                ap: None,
                uplinks: CABLE_ONLY,
            },
            DeviceInfoView::SetupStart {
                ap: Some(setup_ap()),
                uplinks: Uplinks::WIFI_ONLY,
            },
            DeviceInfoView::SetupStart {
                ap: Some(setup_ap()),
                uplinks: BOTH,
            },
            DeviceInfoView::SetupStart {
                ap: Some(AccessPoint {
                    ssid: String::new(),
                    setup_url: SETUP_URL.to_owned(),
                }),
                uplinks: BOTH,
            },
            DeviceInfoView::SetupConnecting { link: wifi(None) },
            DeviceInfoView::SetupConnecting { link: Link::Cable },
            DeviceInfoView::TurningApOff,
            DeviceInfoView::SetupConnected { ssid: None },
            DeviceInfoView::SetupConnectInfo {
                ip: Some(IP),
                link: wifi(None),
            },
            DeviceInfoView::SetupConnectInfo {
                ip: None,
                link: wifi(None),
            },
            DeviceInfoView::SetupCompleted,
            DeviceInfoView::SetupError,
            DeviceInfoView::SetupFatal {
                restarting: true,
                dismissible: false,
            },
            DeviceInfoView::SetupFatal {
                restarting: false,
                dismissible: false,
            },
            DeviceInfoView::SetupFatal {
                restarting: false,
                dismissible: true,
            },
            DeviceInfoView::UpgradeSuccess,
            DeviceInfoView::Connecting { link: wifi(None) },
            DeviceInfoView::Connecting { link: Link::Cable },
            DeviceInfoView::Success { ip: IP },
            DeviceInfoView::Failed { link: wifi(None) },
            DeviceInfoView::Failed { link: Link::Cable },
            DeviceInfoView::Done,
        ]
    }

    /// The views a board waiting on its cable shows.
    fn cable_views() -> Vec<DeviceInfoView> {
        all_views().into_iter().filter(waits_on_a_cable).collect()
    }

    /// Whether `view` is one a board waiting on its cable shows.
    /// Every variant answers for itself rather than falling through a pattern,
    /// so a new screen cannot join `all_views` and quietly escape the check.
    fn waits_on_a_cable(view: &DeviceInfoView) -> bool {
        match view {
            DeviceInfoView::SetupStart { uplinks, .. } => !uplinks.wifi,
            DeviceInfoView::SetupConnecting { link }
            | DeviceInfoView::SetupConnectInfo { link, .. }
            | DeviceInfoView::Connecting { link }
            | DeviceInfoView::Failed { link } => *link == Link::Cable,
            // Nothing here names how the device is reached: the screens that
            // do name a network are reporting a Wi-Fi join that happened.
            DeviceInfoView::TurningApOff
            | DeviceInfoView::SetupConnected { .. }
            | DeviceInfoView::SetupCompleted
            | DeviceInfoView::SetupError
            | DeviceInfoView::SetupFatal { .. }
            | DeviceInfoView::UpgradeSuccess
            | DeviceInfoView::Success { .. }
            | DeviceInfoView::Done => false,
        }
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

    /// Every span the screen puts on the panel, in render order.
    fn texts(node: &TreeNode) -> Vec<String> {
        let mut out = Vec::new();
        if let TreeNode::Paragraph { spans, .. } = node {
            out.extend(spans.iter().map(|span| span.text.clone()));
        }
        for kid in children(node).unwrap_or_default() {
            out.extend(texts(kid));
        }
        out
    }

    /// Every draw the screen issues, from every canvas in the tree.
    fn draws(node: &TreeNode) -> Vec<&DrawCommand> {
        let mut out = Vec::new();
        if let TreeNode::Canvas { draws: own, .. } = node {
            out.extend(own.iter());
        }
        for kid in children(node).unwrap_or_default() {
            out.extend(draws(kid));
        }
        out
    }

    /// What the screen's QR codes encode.
    fn qr_payloads(tree: &TreeNode) -> Vec<&str> {
        draws(tree)
            .into_iter()
            .filter_map(|draw| {
                if let DrawCommand::Qr { text, .. } = draw {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Which registered icons the screen draws.
    fn icon_ids(tree: &TreeNode) -> Vec<SvgId> {
        draws(tree)
            .into_iter()
            .filter_map(|draw| {
                if let DrawCommand::Svg { icon_id, .. } = draw {
                    *icon_id
                } else {
                    None
                }
            })
            .collect()
    }

    /// Icons whose IDs all differ, so a screen reaching for the wrong one fails
    /// instead of matching the `None` every default icon shares.
    fn distinct_icons() -> DeviceInfoIcons {
        let mut next = 1;
        let mut id = || {
            let icon = Icon {
                id: SvgId::from_wire(next),
                size: (24.0, 24.0),
            };
            next += 1;
            icon
        };
        DeviceInfoIcons {
            wifi: id(),
            miner: id(),
            ethernet: id(),
            ethernet_error: id(),
            wifi_connect: id(),
            wifi_error: id(),
            success: id(),
            refresh: id(),
            desktop_clock: id(),
            error: id(),
            close: id(),
            swipe_down: id(),
        }
    }

    fn tree_for(view: &DeviceInfoView) -> TreeNode {
        build_device_info_tree(
            view,
            distinct_icons(),
            tier_for(DECK_PANEL),
            DEVICE_NAME,
            false,
        )
        .expect("BUG: every view but Done builds a tree")
    }

    /// The close glyph is the only absolutely positioned node on a screen.
    fn has_close(tree: &TreeNode) -> bool {
        let TreeNode::Row(_, columns) = tree else {
            panic!("BUG: every screen is rooted in a row");
        };
        columns.iter().any(
            |node| matches!(node, TreeNode::Canvas { props, .. } if props.inset_right.is_finite()),
        )
    }

    #[test]
    fn every_view_but_done_builds_a_tree() {
        for view in all_views() {
            let tree = build_device_info_tree(
                &view,
                DeviceInfoIcons::default(),
                tier_for(DECK_PANEL),
                DEVICE_NAME,
                false,
            );
            assert_eq!(
                tree.is_some(),
                view != DeviceInfoView::Done,
                "view {view:?}"
            );
        }
    }

    #[test]
    fn every_screen_renders_its_own_words() {
        let cases: Vec<(DeviceInfoView, &[&str])> = vec![
            (
                DeviceInfoView::SetupStart {
                    ap: None,
                    uplinks: Uplinks::WIFI_ONLY,
                },
                &["Initial Setup", "Starting setup Wi-Fi..."],
            ),
            (
                DeviceInfoView::SetupStart {
                    ap: Some(setup_ap()),
                    uplinks: Uplinks::WIFI_ONLY,
                },
                &[
                    "Connect to Braiins Deck Wi-Fi",
                    "Wi-Fi SSID",
                    SETUP_SSID,
                    "Scan the code!",
                    SETUP_URL,
                ],
            ),
            (
                DeviceInfoView::SetupConnecting { link: wifi(None) },
                &["Connecting to Wi-Fi...", "Waiting for Wi-Fi connection"],
            ),
            (
                DeviceInfoView::SetupConnected {
                    ssid: Some("home".to_owned()),
                },
                &["Your Braiins Deck is connected!", "Wi-Fi SSID", "home"],
            ),
            (
                DeviceInfoView::SetupConnectInfo {
                    ip: Some(IP),
                    link: wifi(None),
                },
                &["Complete the setup", "http://10.0.0.5/", "or scan the QR"],
            ),
            (
                DeviceInfoView::SetupConnectInfo {
                    ip: None,
                    link: wifi(Some("home")),
                },
                &["Connecting to Wi-Fi...", "home"],
            ),
            (
                DeviceInfoView::SetupCompleted,
                &["Braiins Deck is ready!", "Login to continue"],
            ),
            (
                DeviceInfoView::SetupError,
                &["Could not connect. Please try again."],
            ),
            (
                DeviceInfoView::SetupFatal {
                    restarting: true,
                    dismissible: false,
                },
                &["Problem Occurred", "Restarting Braiins Deck..."],
            ),
            (
                DeviceInfoView::SetupFatal {
                    restarting: false,
                    dismissible: false,
                },
                &["Problem Occurred", "Restart Braiins Deck to try again"],
            ),
            (DeviceInfoView::UpgradeSuccess, &["Update Finished"]),
            (
                DeviceInfoView::Connecting {
                    link: wifi(Some("home")),
                },
                &["Connecting to Wi-Fi...", "Wi-Fi SSID", "home"],
            ),
            (
                DeviceInfoView::Success { ip: IP },
                &[
                    "Access the device at",
                    "http://10.0.0.5/",
                    "To access the controls, IP and Wi-Fi info",
                    "swipe down",
                ],
            ),
            (
                DeviceInfoView::Failed { link: wifi(None) },
                &["Problem with connection", "No IP address assigned"],
            ),
        ];
        assert_screens_say(cases);
    }

    #[test]
    fn every_cable_screen_renders_its_own_words() {
        let cases: Vec<(DeviceInfoView, &[&str])> = vec![
            (
                DeviceInfoView::SetupStart {
                    ap: None,
                    uplinks: BOTH,
                },
                &["Starting setup Wi-Fi...", "or connect the Ethernet cable"],
            ),
            (
                DeviceInfoView::SetupStart {
                    ap: None,
                    uplinks: CABLE_ONLY,
                },
                &["Initial Setup", "Connect the Ethernet cable"],
            ),
            (
                DeviceInfoView::SetupStart {
                    ap: Some(setup_ap()),
                    uplinks: BOTH,
                },
                &["Wi-Fi SSID", SETUP_SSID, "or connect the Ethernet cable"],
            ),
            (
                DeviceInfoView::SetupConnecting { link: Link::Cable },
                &["Initial Setup", "Connecting to network..."],
            ),
            (
                DeviceInfoView::Connecting { link: Link::Cable },
                &["Connecting to network...", "Waiting for IP address"],
            ),
            (
                DeviceInfoView::Failed { link: Link::Cable },
                &["No network connection", "Check the Ethernet cable"],
            ),
        ];
        assert_screens_say(cases);
    }

    fn assert_screens_say(cases: Vec<(DeviceInfoView, &[&str])>) {
        for (view, wanted) in cases {
            let rendered = texts(&tree_for(&view));
            for want in wanted {
                assert!(
                    rendered.iter().any(|line| line.contains(want)),
                    "{view:?} never says {want:?}; it says {rendered:?}"
                );
            }
        }
    }

    #[test]
    fn a_board_waiting_on_its_cable_is_never_told_about_wifi() {
        for view in cable_views() {
            let rendered = texts(&tree_for(&view));
            assert!(
                !rendered.iter().any(|line| line.contains("Wi-Fi")),
                "{view:?} mentions Wi-Fi: {rendered:?}"
            );
        }
    }

    #[test]
    fn a_wifi_only_board_reads_as_it_always_did() {
        // The cable hint is for boards that have a port to plug into.
        for view in [
            DeviceInfoView::SetupStart {
                ap: None,
                uplinks: Uplinks::WIFI_ONLY,
            },
            DeviceInfoView::SetupStart {
                ap: Some(setup_ap()),
                uplinks: Uplinks::WIFI_ONLY,
            },
        ] {
            let rendered = texts(&tree_for(&view));
            assert!(
                !rendered.iter().any(|line| line.contains("Ethernet")),
                "{view:?} mentions a cable: {rendered:?}"
            );
        }
    }

    #[test]
    fn the_qr_codes_carry_the_address_printed_beside_them() {
        // The setup screen reads its label and its code from different fields,
        // so a swap between them is the one mismatch that is possible here.
        let tree = tree_for(&DeviceInfoView::SetupStart {
            ap: Some(setup_ap()),
            uplinks: Uplinks::WIFI_ONLY,
        });
        assert_eq!(
            qr_payloads(&tree),
            [SETUP_URL],
            "the setup code opens the wizard, not the SSID"
        );

        // Skip-WiFi switchover on a miner: a neutral progress screen.
        let tree = build_device_info_tree(
            &DeviceInfoView::TurningApOff,
            distinct_icons(),
            tier_for(DECK_PANEL),
            DEVICE_NAME,
            true,
        )
        .expect("BUG: view builds a tree");
        assert!(
            texts(&tree)
                .iter()
                .any(|line| line.contains("being set up")),
            "miner skip shows the switchover text"
        );

        // Wired-uplink setup (empty SSID): the wizard address alone,
        // whatever the board could otherwise offer.
        for uplinks in [Uplinks::WIFI_ONLY, BOTH, CABLE_ONLY] {
            let tree = tree_for(&DeviceInfoView::SetupStart {
                ap: Some(AccessPoint {
                    ssid: String::new(),
                    setup_url: SETUP_URL.to_owned(),
                }),
                uplinks,
            });
            assert_eq!(qr_payloads(&tree), [SETUP_URL]);
            let rendered = texts(&tree);
            assert!(
                rendered.iter().any(|line| line.contains(SETUP_URL)),
                "the wired setup screen prints the address it encodes"
            );
            assert!(
                !rendered.iter().any(|line| line.contains("Wi-Fi SSID")),
                "no AP to join, so no SSID line: {rendered:?}"
            );
        }

        let url = format!("http://{IP}/");
        for view in [
            DeviceInfoView::SetupConnectInfo {
                ip: Some(IP),
                link: wifi(None),
            },
            DeviceInfoView::Success { ip: IP },
        ] {
            let tree = tree_for(&view);
            assert_eq!(qr_payloads(&tree), [url.as_str()], "{view:?}");
            assert!(
                texts(&tree).iter().any(|line| line.contains(&url)),
                "{view:?} prints the address it encodes"
            );
        }
    }

    #[test]
    fn each_screen_leads_with_its_own_icon() {
        let icons = distinct_icons();
        for (view, wanted) in [
            (
                DeviceInfoView::SetupStart {
                    ap: None,
                    uplinks: Uplinks::WIFI_ONLY,
                },
                icons.wifi_connect,
            ),
            (
                DeviceInfoView::SetupStart {
                    ap: None,
                    uplinks: CABLE_ONLY,
                },
                icons.desktop_clock,
            ),
            (
                DeviceInfoView::SetupConnecting { link: wifi(None) },
                icons.wifi,
            ),
            (
                DeviceInfoView::SetupConnecting { link: Link::Cable },
                icons.ethernet,
            ),
            (DeviceInfoView::TurningApOff, icons.ethernet),
            (DeviceInfoView::SetupConnected { ssid: None }, icons.wifi),
            (
                DeviceInfoView::SetupConnectInfo {
                    ip: Some(IP),
                    link: wifi(None),
                },
                icons.desktop_clock,
            ),
            (DeviceInfoView::SetupCompleted, icons.success),
            (DeviceInfoView::SetupError, icons.wifi_error),
            (
                DeviceInfoView::SetupFatal {
                    restarting: true,
                    dismissible: false,
                },
                icons.refresh,
            ),
            (
                DeviceInfoView::SetupFatal {
                    restarting: false,
                    dismissible: false,
                },
                icons.error,
            ),
            (DeviceInfoView::UpgradeSuccess, icons.success),
            (DeviceInfoView::Connecting { link: wifi(None) }, icons.wifi),
            (
                DeviceInfoView::Connecting { link: Link::Cable },
                icons.ethernet,
            ),
            (DeviceInfoView::Success { ip: IP }, icons.desktop_clock),
            (
                DeviceInfoView::Failed { link: wifi(None) },
                icons.wifi_error,
            ),
            (
                DeviceInfoView::Failed { link: Link::Cable },
                icons.ethernet_error,
            ),
        ] {
            let drawn = icon_ids(&tree_for(&view));
            let wanted = wanted.id.expect("BUG: fixture icons carry an ID");
            assert!(
                drawn.contains(&wanted),
                "{view:?} draws {drawn:?}, which does not include its own icon {wanted:?}"
            );
        }
    }

    #[test]
    fn the_setup_screen_leads_with_the_icon_its_tier_names() {
        let icons = distinct_icons();
        for (choice, wanted) in [
            (SetupApIcon::WifiConnect, icons.wifi_connect),
            (SetupApIcon::Wifi, icons.wifi),
        ] {
            let tier = Tier {
                ap_up_icon: choice,
                ..TIER_WIDE
            };
            let tree = build_device_info_tree(
                &DeviceInfoView::SetupStart {
                    ap: Some(setup_ap()),
                    uplinks: Uplinks::WIFI_ONLY,
                },
                icons,
                tier,
                DEVICE_NAME,
                false,
            )
            .expect("BUG: the setup screen builds a tree");
            let wanted = wanted.id.expect("BUG: fixture icons carry an ID");
            let drawn = icon_ids(&tree);
            assert!(
                drawn.contains(&wanted),
                "{choice:?} draws {drawn:?}, which does not include {wanted:?}"
            );
        }
    }

    #[test]
    fn the_close_glyph_follows_dismissability() {
        for view in all_views() {
            let Some(tree) = build_device_info_tree(
                &view,
                DeviceInfoIcons::default(),
                tier_for(DECK_PANEL),
                DEVICE_NAME,
                false,
            ) else {
                continue;
            };
            assert_eq!(
                has_close(&tree),
                dismisses_on_touch(&view),
                "a screen offers a close exactly when a touch closes it: {view:?}"
            );
        }
    }

    #[test]
    fn the_post_upgrade_screen_offers_no_close() {
        // A touch there hands over to the connect flow instead of closing.
        assert!(!dismisses_on_touch(&DeviceInfoView::UpgradeSuccess));
    }

    #[test]
    fn the_setup_screens_offer_no_close() {
        // They hold until bmc moves them on,
        // so an X would be a button that does nothing.
        for view in [
            DeviceInfoView::SetupStart {
                ap: None,
                uplinks: Uplinks::WIFI_ONLY,
            },
            DeviceInfoView::SetupConnecting { link: wifi(None) },
            DeviceInfoView::TurningApOff,
            DeviceInfoView::SetupConnected { ssid: None },
            DeviceInfoView::SetupConnectInfo {
                ip: None,
                link: wifi(None),
            },
            DeviceInfoView::SetupCompleted,
            DeviceInfoView::SetupError,
            DeviceInfoView::SetupFatal {
                restarting: false,
                dismissible: false,
            },
        ] {
            assert!(!dismisses_on_touch(&view), "view {view:?}");
        }
    }

    #[test]
    fn only_a_dismissible_fatal_screen_offers_a_close() {
        // A pending restart is waited out whatever is behind the screen.
        for (restarting, dismissible, expected) in [
            (false, true, true),
            (false, false, false),
            (true, false, false),
        ] {
            let view = DeviceInfoView::SetupFatal {
                restarting,
                dismissible,
            };
            assert_eq!(dismisses_on_touch(&view), expected, "view {view:?}");
        }
    }
}
