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

//! The faces as views over a [`ViewData`] and the system snapshot: a node tree out.
//!
//! The deck-wide snapshot — formats, timezone, night mode, the next alarm —
//! is read from `system::current()` rather than carried in the `ViewData`,
//! so whoever installs it (the host, the gallery, a test) sets it once
//! for every part that formats against it.

mod analog;
mod digital;
pub mod fixtures;
mod parts;

use bmc_wasm_sdk::{Node, SystemTime, Tz, WidgetViewport, system};

use crate::manifest_params::{ClockStyle, Params};
use crate::model::{ClockHandTransition, Frame, face};

/// A moment, the viewport it is drawn into, the operator's params
/// and whether the hands sweep to the moment or snap.
#[derive(Clone, Debug)]
pub struct ViewData {
    pub now: SystemTime,
    pub viewport: WidgetViewport,
    pub params: Params,
    pub hand_transition: ClockHandTransition,
}

/// The face `view`'s viewport draws, at `view`'s moment.
#[must_use]
pub fn clock_view(view: &ViewData) -> Node {
    let frame = Frame::of(view.viewport);
    let tz = view
        .params
        .timezone_override
        .as_deref()
        .map(Tz::from_runtime);
    let palette = parts::clock_palette(system::current().night_mode().unwrap_or(false));
    match face(view.params.clock_style, view.viewport.shape) {
        ClockStyle::AnalogRound => analog::round::render(
            view.now,
            &view.params,
            frame,
            tz.as_ref(),
            &palette,
            view.hand_transition,
        ),
        ClockStyle::AnalogRect => analog::rect::render(
            view.now,
            &view.params,
            frame,
            tz.as_ref(),
            &palette,
            view.hand_transition,
        ),
        ClockStyle::Digital => {
            digital::render(view.now, &view.params, frame, tz.as_ref(), &palette)
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::system::{SnapshotBuilder, TimeFormat};
    use bmc_wasm_sdk::{Draw, assets};

    use super::*;
    use crate::model::SizeBucket;
    use crate::screens::digital::BMM101_CAPTION_LINE_HEIGHT;
    use crate::screens::fixtures::{self, SEPTEMBER_NOON};

    /// 04:30 UTC on the 15th of September 2026 — 06:30 in Prague.
    const ALARM_UTC_MS: i64 = 1_789_446_600_000;

    fn install_prague(next_alarm: bool) {
        assets::init_test_registrars();
        let mut snapshot = SnapshotBuilder::new()
            .timezone("Europe/Prague")
            .time_format(TimeFormat::Hour24);
        if next_alarm {
            snapshot = snapshot.next_alarm_some(ALARM_UTC_MS, "Wake up");
        }
        system::set_current(snapshot.build());
    }

    /// Every string the tree would draw, paragraphs and canvas text alike.
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
            Node::Canvas { draws, .. } => {
                for draw in draws {
                    if let Draw::Text { text, .. } = draw {
                        out.push(text.clone());
                    }
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_digital_face_reads_the_moment_in_the_system_zone() {
        install_prague(false);
        let view =
            fixtures::at_bucket(SizeBucket::Full, SEPTEMBER_NOON, fixtures::default_params());
        let texts = texts(&clock_view(&view));
        assert!(texts.contains(&"12:30:00".to_owned()), "{texts:?}");
        assert!(
            texts.contains(&"Mon 14 September 2026".to_owned()),
            "{texts:?}"
        );
        assert!(texts.contains(&"Prague (+2)".to_owned()), "{texts:?}");
    }

    #[test]
    fn an_override_zone_moves_the_digits_and_the_caption() {
        install_prague(false);
        let params = Params {
            timezone_override: Some("Europe/Helsinki".to_owned()),
            ..fixtures::default_params()
        };
        let view = fixtures::at_bucket(SizeBucket::Large, SEPTEMBER_NOON, params);
        let texts = texts(&clock_view(&view));
        assert!(texts.contains(&"13:30:00".to_owned()), "{texts:?}");
        assert!(texts.contains(&"Helsinki (+3)".to_owned()), "{texts:?}");
    }

    #[test]
    fn the_alarm_row_shows_on_the_full_digital_face_only() {
        install_prague(true);
        let full =
            fixtures::at_bucket(SizeBucket::Full, SEPTEMBER_NOON, fixtures::default_params());
        assert!(texts(&clock_view(&full)).contains(&"06:30".to_owned()));
        let large = fixtures::at_bucket(
            SizeBucket::Large,
            SEPTEMBER_NOON,
            fixtures::default_params(),
        );
        assert!(!texts(&clock_view(&large)).contains(&"06:30".to_owned()));
    }

    /// The frame BMM101 was designed for: the zone above the digits,
    /// the numeric date below, `AM` beside them — and no weekday, no alarm.
    #[test]
    fn the_bmm101_face_splits_the_captions_around_the_digits() {
        assets::init_test_registrars();
        system::set_current(
            SnapshotBuilder::new()
                .timezone("Europe/Prague")
                .time_format(TimeFormat::Hour12)
                .next_alarm_some(ALARM_UTC_MS, "Wake up")
                .build(),
        );
        let view = fixtures::at_bucket(
            SizeBucket::Bmm101,
            fixtures::DESIGN_MOMENT,
            fixtures::default_params(),
        );
        let root = clock_view(&view);
        let Node::Column(_, slots) = &root else {
            panic!("BUG: the digital face is a column of slots");
        };
        let caption_slot = |slot: &Node| {
            let Node::Column(props, lines) = slot else {
                panic!("BUG: a caption slot is a fixed-height column");
            };
            assert_eq!((props.height, lines.len()), (BMM101_CAPTION_LINE_HEIGHT, 1));
            lines.iter().flat_map(texts).collect::<Vec<_>>()
        };
        assert_eq!(caption_slot(&slots[1]), ["Prague (+1)"]);
        assert_eq!(caption_slot(&slots[5]), ["24.12.2025"]);
        assert_eq!(texts(&slots[3]), ["12:39:30", "AM"]);
        assert!(!texts(&root).contains(&"06:30".to_owned()), "no alarm row");
    }

    /// A hidden readout leaves its slot in place, so the digits do not move.
    #[test]
    fn a_hidden_bmm101_readout_keeps_its_slot() {
        install_prague(false);
        let params = Params {
            show_timezone: false,
            show_date: false,
            ..fixtures::default_params()
        };
        let view = fixtures::at_bucket(SizeBucket::Bmm101, fixtures::DESIGN_MOMENT, params);
        let Node::Column(_, slots) = clock_view(&view) else {
            panic!("BUG: the digital face is a column of slots");
        };
        for slot in [&slots[1], &slots[5]] {
            let Node::Column(props, lines) = slot else {
                panic!("BUG: a caption slot is a fixed-height column");
            };
            assert_eq!((props.height, lines.len()), (BMM101_CAPTION_LINE_HEIGHT, 0));
        }
    }

    #[test]
    fn the_rectangular_dial_numbers_its_quarters() {
        install_prague(false);
        let params = Params {
            clock_style: ClockStyle::AnalogRect,
            ..fixtures::default_params()
        };
        let view = fixtures::at_bucket(SizeBucket::Full, SEPTEMBER_NOON, params);
        let texts = texts(&clock_view(&view));
        for numeral in ["12", "3", "6", "9"] {
            assert!(texts.contains(&numeral.to_owned()), "{texts:?}");
        }
        assert!(texts.contains(&"Mon 14 Sep".to_owned()), "{texts:?}");
    }

    #[test]
    fn a_round_viewport_draws_the_round_dial_whatever_the_style() {
        install_prague(false);
        let view = fixtures::round(480, SEPTEMBER_NOON, fixtures::default_params());
        let root = clock_view(&view);
        let Node::Center(_, layers) = &root else {
            panic!("BUG: an analog face is its dial and hands, centred");
        };
        assert!(
            matches!(
                layers.as_slice(),
                [Node::Canvas { .. }, Node::Canvas { .. }]
            ),
            "the dial and the hands are two canvases"
        );
        let texts = texts(&root);
        assert!(texts.contains(&"Prague".to_owned()), "{texts:?}");
        assert!(texts.contains(&"+2".to_owned()), "{texts:?}");
        assert!(
            !texts.contains(&"12".to_owned()),
            "no numerals on the round dial"
        );
    }
}
