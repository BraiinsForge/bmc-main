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

//! The layouts as views over a [`ViewData`]: size dispatch,
//! the launch panels, and the loading/error states.

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

use crate::model::{LaunchData, State};
use crate::screens::parts::{ValueLayout, detail_table, launch_info_table, rocket_panel};

/// What the widget holds, the viewport it is drawn into,
/// and the moment the countdown is counted from.
#[derive(Clone, Debug)]
pub struct ViewData {
    pub viewport: WidgetViewport,
    pub state: State,
    pub now_secs: i64,
}

#[must_use]
pub fn launch_view(view: &ViewData) -> Node {
    match &view.state {
        State::Loaded(data) => {
            let size = WidgetSize::from_dimensions(view.viewport.width, view.viewport.height);
            current_view(data, size, view.now_secs)
        }
        State::Loading => loading_view(),
        State::NoLaunch => empty_view(),
        State::Error(msg) => error_view(msg),
    }
}

/// Dispatch the loaded view by size.
/// The countdown is computed per render, so the timer keeps ticking
/// between nexus refreshes; once the net time passes, the status reads `Launched`.
fn current_view(data: &LaunchData, size: WidgetSize, now_secs: i64) -> Node {
    let remaining = data.launch_unix - now_secs;
    let countdown = format_duration(remaining, true);
    let status = if remaining > 0 {
        data.status.as_str()
    } else {
        "Launched"
    };
    match size.variant {
        SizeVariant::Full => render_full(size.height, data, &countdown, status),
        // The large view stacks both tables and needs the height it was drawn for.
        // A shorter viewport still classifies as Large (BMM101 at 480x320 does),
        // and the stack then runs past the bottom edge, so anything short falls
        // to the side-by-side view that fits a shallow frame.
        SizeVariant::Large if size.height >= SizeVariant::Large.height() => {
            render_large(data, &countdown, status)
        }
        // Narrower than the Deck slot the side-by-side view was drawn for,
        // so its values wrap where they used to fit. Stacking each under
        // its label spends the height this frame has spare to buy back that width.
        SizeVariant::Large => render_medium(data, &countdown, status, ValueLayout::Stacked),
        SizeVariant::Medium => render_medium(data, &countdown, status, ValueLayout::Inline),
        SizeVariant::Small => render_small(data, &countdown, status),
    }
}

/// Centered loading message.
fn loading_view() -> Node {
    col(
        props!(padding: 32.0, background: BLACK),
        [text("Loading\u{2026}", style!(size: 24, color: GRAY_30))],
    )
}

/// Plain "no upcoming launches" message (valid empty reply, not an error).
fn empty_view() -> Node {
    col(
        props!(padding: 32.0, gap: 16.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 24, color: GRAY_30)),
                    text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
                ],
            ),
            text("No upcoming launches", style!(size: 24, color: GRAY_30)),
        ],
    )
}

/// Header plus an error banner with the failure detail.
fn error_view(detail: &str) -> Node {
    col(
        props!(padding: 32.0, gap: 16.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 24, color: GRAY_30)),
                    text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
                ],
            ),
            notification(
                NotificationKind::Error,
                "Failed to load launch data",
                detail,
            ),
        ],
    )
}

/// Full (1280×480): header + mission + two tables + rocket panel.
fn render_full(height: u32, data: &LaunchData, countdown: &str, status: &str) -> Node {
    row(
        props!(background: BLACK),
        [
            col(
                props!(padding: 32.0, flex: 1.0, gap: 12.0),
                [
                    // Header
                    row(
                        props!(gap: 8.0),
                        [
                            text("Space X", style!(size: 24, color: GRAY_30)),
                            text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
                        ],
                    ),
                    // Mission title
                    text(
                        &data.mission_name,
                        style!(size: 32, weight: FontWeight::BOLD),
                    ),
                    text("Mission name", style!(size: 24, color: GRAY_30)),
                    // Distribute space around tables
                    spacer(1.0),
                    // Two data tables side by side
                    row(
                        props!(gap: 40.0),
                        [
                            launch_info_table(
                                24,
                                10.0,
                                data,
                                countdown,
                                status,
                                ValueLayout::Inline,
                            ),
                            detail_table(24, 10.0, data, ValueLayout::Inline),
                        ],
                    ),
                    spacer(0.3),
                ],
            ),
            rocket_panel(&data.rocket, height as f32),
        ],
    )
}

/// Large (638×480): header + mission + two tables (stacked), no rocket.
fn render_large(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            // Header
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 22, color: GRAY_30)),
                    text("Next Launch", style!(size: 22, weight: FontWeight::BOLD)),
                ],
            ),
            // Mission title
            col(
                props!(gap: 4.0),
                [
                    text(
                        &data.mission_name,
                        style!(size: 28, weight: FontWeight::BOLD),
                    ),
                    text("Mission name", style!(size: 22, color: GRAY_30)),
                ],
            ),
            spacer(1.0),
            col(
                props!(gap: 32.0),
                [
                    launch_info_table(18, 6.0, data, countdown, status, ValueLayout::Inline),
                    detail_table(18, 6.0, data, ValueLayout::Inline),
                ],
            ),
        ],
    )
}

/// Medium (638×238): mission in header, two tables side by side.
fn render_medium(data: &LaunchData, countdown: &str, status: &str, layout: ValueLayout) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text("Space X", style!(size: 20, color: GRAY_30)),
                    text(
                        &data.mission_name,
                        style!(size: 20, weight: FontWeight::BOLD),
                    ),
                ],
            ),
            spacer(1.0),
            // Smaller font keeps a long detail value on one line at 638×238.
            row(
                props!(gap: 20.0),
                [
                    launch_info_table(16, 6.0, data, countdown, status, layout),
                    detail_table(16, 6.0, data, layout),
                ],
            ),
        ],
    )
}

/// Small (317×238): mission as title, single table.
fn render_small(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            text(
                &data.mission_name,
                style!(size: 20, weight: FontWeight::BOLD),
            ),
            spacer(1.0),
            launch_info_table(20, 8.0, data, countdown, status, ValueLayout::Inline),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SizeBucket;
    use crate::screens::fixtures;

    /// Every string the tree would draw, in tree order.
    fn texts(node: &Node) -> Vec<String> {
        let mut out = Vec::new();
        collect_texts(node, &mut out);
        out
    }

    fn collect_texts(node: &Node, out: &mut Vec<String>) {
        match node {
            Node::Column(_, children) | Node::Row(_, children) | Node::Center(_, children) => {
                for child in children {
                    collect_texts(child, out);
                }
            }
            Node::Paragraph { spans, .. } => {
                out.push(spans.iter().map(|span| span.text.as_str()).collect());
            }
            _ => {}
        }
    }

    #[test]
    fn a_passed_launch_reads_launched_at_t_zero() {
        let view = fixtures::launched(fixtures::at_bucket(SizeBucket::Small));
        let texts = texts(&launch_view(&view));
        assert!(
            texts.contains(&"T-0".to_owned()) && texts.contains(&"Launched".to_owned()),
            "{texts:?}"
        );
    }
}
