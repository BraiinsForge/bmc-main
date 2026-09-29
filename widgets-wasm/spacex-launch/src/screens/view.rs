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

use crate::model::{Frame, LaunchData, SizeBucket, State};
use crate::screens::bmm101;
use crate::screens::parts::{self, BRAND, detail_table, launch_info_table, rocket_panel};

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
    let frame = Frame::of(view.viewport);
    let bucket = frame.bucket;
    match &view.state {
        State::Loaded(data) => loaded_view(data, frame, view.now_secs),
        State::Loading => loading_view(state_type(bucket)),
        State::NoLaunch => empty_view(state_type(bucket), state_header(bucket)),
        State::Error(detail) => error_view(detail, state_type(bucket), state_header(bucket)),
    }
}

/// Dispatch the loaded view by size.
fn loaded_view(data: &LaunchData, frame: Frame, now_secs: i64) -> Node {
    let (countdown, status) = parts::countdown(data, now_secs);
    match frame.bucket {
        SizeBucket::Full => render_full(frame.size.height, data, &countdown, status),
        SizeBucket::Large => render_large(data, &countdown, status),
        SizeBucket::Medium => render_medium(data, &countdown, status),
        SizeBucket::Small => render_small(data, &countdown, status),
        SizeBucket::Bmm101 => bmm101::launch(data, &countdown, status),
    }
}

/// The views without a launch share one design everywhere,
/// set in the device's type under its own header.
#[derive(Clone, Copy)]
struct StateType {
    padding: f32,
    body: u32,
}

fn state_type(bucket: SizeBucket) -> StateType {
    match bucket {
        SizeBucket::Full | SizeBucket::Large | SizeBucket::Medium | SizeBucket::Small => {
            StateType {
                padding: 32.0,
                body: 24,
            }
        }
        SizeBucket::Bmm101 => StateType {
            padding: bmm101::EDGE,
            body: bmm101::BODY_SIZE,
        },
    }
}

fn state_header(bucket: SizeBucket) -> Node {
    match bucket {
        SizeBucket::Full | SizeBucket::Large | SizeBucket::Medium | SizeBucket::Small => row(
            props!(gap: 8.0),
            [
                text(BRAND, style!(size: 24, color: GRAY_30)),
                text("Next Launch", style!(size: 24, weight: FontWeight::BOLD)),
            ],
        ),
        SizeBucket::Bmm101 => bmm101::header(),
    }
}

fn loading_view(set: StateType) -> Node {
    col(
        props!(padding: set.padding, background: BLACK),
        [text(
            "Loading\u{2026}",
            style!(size: set.body, color: GRAY_30),
        )],
    )
}

/// A valid reply with nothing upcoming, not an error.
fn empty_view(set: StateType, header: Node) -> Node {
    col(
        props!(padding: set.padding, gap: 16.0, background: BLACK),
        [
            header,
            text(
                "No upcoming launches",
                style!(size: set.body, color: GRAY_30),
            ),
        ],
    )
}

fn error_view(detail: &str, set: StateType, header: Node) -> Node {
    col(
        props!(padding: set.padding, gap: 16.0, background: BLACK),
        [
            header,
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
                            text(BRAND, style!(size: 24, color: GRAY_30)),
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
                            launch_info_table(24, 10.0, data, countdown, status),
                            detail_table(24, 10.0, data),
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
                    text(BRAND, style!(size: 22, color: GRAY_30)),
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
                    launch_info_table(18, 6.0, data, countdown, status),
                    detail_table(18, 6.0, data),
                ],
            ),
        ],
    )
}

/// Medium (638×238): mission in header, two tables side by side.
fn render_medium(data: &LaunchData, countdown: &str, status: &str) -> Node {
    col(
        props!(padding: 24.0, gap: 8.0, background: BLACK),
        [
            row(
                props!(gap: 8.0),
                [
                    text(BRAND, style!(size: 20, color: GRAY_30)),
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
                    launch_info_table(16, 6.0, data, countdown, status),
                    detail_table(16, 6.0, data),
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
            // One line, or a long name pushes the table off the frame.
            text(
                &data.mission_name,
                style!(size: 20, weight: FontWeight::BOLD, text_overflow: TextOverflow::Ellipsis),
            ),
            spacer(1.0),
            launch_info_table(20, 8.0, data, countdown, status),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screens::fixtures::{self, StateFixture};
    use crate::screens::tree::{overflow_of, texts};

    fn at(bucket: SizeBucket, state: StateFixture) -> Vec<String> {
        texts(&launch_view(&state(fixtures::at_bucket(bucket))))
    }

    /// Each label followed by the value it reads, left column first.
    fn grid(values: [&str; 8]) -> Vec<String> {
        let labels = [
            "Scheduled",
            "Status",
            "Rocket",
            "Place",
            "Landing",
            "Booster",
            "Payload",
            "Spacecraft",
        ];
        labels
            .into_iter()
            .zip(values)
            .flat_map(|(label, value)| [label.to_owned(), value.to_owned()])
            .collect()
    }

    /// The header, then the hero over its caption.
    fn bmm101_top(mission: &str) -> Vec<String> {
        [BRAND, "Next Launch", mission, "Mission name"]
            .map(str::to_owned)
            .to_vec()
    }

    #[test]
    fn every_frame_keeps_the_brand_on_one_line() {
        assets::init_test_registrars();
        let branded = [
            (SizeBucket::Full, fixtures::healthy as StateFixture),
            (SizeBucket::Large, fixtures::healthy),
            (SizeBucket::Medium, fixtures::healthy),
            (SizeBucket::Bmm101, fixtures::healthy),
            (SizeBucket::Large, fixtures::no_launch),
            (SizeBucket::Large, fixtures::failed),
            (SizeBucket::Bmm101, fixtures::no_launch),
        ];
        let brand = typography::unbroken("Space X");
        for (bucket, state) in branded {
            let texts = at(bucket, state);
            assert!(texts.contains(&brand), "{bucket:?}: {texts:?}");
        }
    }

    #[test]
    fn the_small_layout_ends_a_long_mission_in_an_ellipsis() {
        for viewport in [
            fixtures::at_bucket(SizeBucket::Small),
            fixtures::rectangular(320, 240),
        ] {
            let view = fixtures::longest_mission(viewport);
            assert_eq!(
                overflow_of(&launch_view(&view), fixtures::LONGEST_MISSION),
                Some(TextOverflow::Ellipsis),
                "{viewport:?}"
            );
        }
    }

    #[test]
    fn every_frame_keeps_the_countdown_on_one_line() {
        assets::init_test_registrars();
        let countdown = typography::unbroken("0d 20h 28m 39s");
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let texts = at(bucket, fixtures::healthy);
            assert!(texts.contains(&countdown), "{bucket:?}: {texts:?}");
        }
    }

    #[test]
    fn a_launch_a_hundred_days_out_counts_down_without_seconds() {
        let texts = at(SizeBucket::Small, fixtures::widest_values);
        assert!(
            texts.contains(&typography::unbroken("888d 08h 08m")),
            "{texts:?}"
        );
    }

    #[test]
    fn a_passed_launch_reads_launched_at_t_zero() {
        for bucket in [SizeBucket::Small, SizeBucket::Bmm101] {
            let texts = at(bucket, fixtures::launched);
            assert!(
                texts.contains(&"T-0".to_owned()) && texts.contains(&"Launched".to_owned()),
                "{bucket:?}: {texts:?}"
            );
        }
    }

    #[test]
    fn the_bmm101_frame_reads_top_down_as_designed() {
        let countdown = typography::unbroken("0d 20h 28m 39s");
        let booster = fmt!("3{} flown", typography::TIMES);
        let mut expected = bmm101_top("NROL-179");
        expected.extend(grid([
            &countdown,
            "Go for Launch",
            "Falcon 9 Block 5",
            "VSFB SLC-4E",
            "RTLS",
            &booster,
            "Classified",
            "N/A",
        ]));
        assert_eq!(at(SizeBucket::Bmm101, fixtures::healthy), expected);
    }

    #[test]
    fn every_frame_reads_loading_the_same_way() {
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            assert_eq!(
                at(bucket, fixtures::loading),
                ["Loading\u{2026}"],
                "{bucket:?}"
            );
        }
    }

    #[test]
    fn the_deck_and_bmm101_report_failures_the_same_way() {
        for bucket in [SizeBucket::Large, SizeBucket::Bmm101] {
            assert_eq!(
                at(bucket, fixtures::failed),
                [
                    BRAND,
                    "Next Launch",
                    "Failed to load launch data",
                    "API request failed (503)"
                ],
                "{bucket:?}"
            );
            assert_eq!(
                at(bucket, fixtures::no_launch),
                [BRAND, "Next Launch", "No upcoming launches"],
                "{bucket:?}"
            );
        }
    }
}
