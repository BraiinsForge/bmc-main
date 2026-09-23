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

//! The layouts as views over a [`ViewData`] and the system snapshot: a node tree out.
//!
//! Temperatures, times and wind format against `system::current()`,
//! so whoever installs the snapshot — the host, the gallery, a test —
//! sets the units, the clock and the timezone once for every layout.

#[expect(
    clippy::wildcard_imports,
    reason = "screen code uses the SDK's tree builders, macros, and tokens throughout"
)]
use bmc_wasm_sdk::*;

use crate::display;
use crate::manifest_params::Params;
use crate::model::{Frame, Location, State, Weather};
use crate::screens::{common, full, large, medium, small};

/// What the widget holds, the viewport it is drawn into, the operator's params,
/// and the last good load while a refresh keeps failing past its grace.
#[derive(Clone, Debug)]
pub struct ViewData {
    pub viewport: WidgetViewport,
    pub params: Params,
    pub state: State,
    pub stale_since: Option<SystemTime>,
}

#[must_use]
pub fn weather_view(view: &ViewData) -> Node {
    if view.params.location.trim().is_empty() {
        return message_view(display::ENTER_LOCATION);
    }
    let frame = Frame::of(view.viewport);
    match &view.state {
        State::Loaded(weather) => {
            let root = layout(weather, &view.params, frame);
            match view.stale_since {
                Some(anchor) => with_stale_overlay(root, anchor, view.viewport.shape),
                None => root,
            }
        }
        State::Loading => layout(&not_loaded(), &view.params, frame),
        State::BadLocation => message_view("Location not found"),
        State::Error => message_view(display::CANNOT_LOAD),
    }
}

/// Nothing fetched yet, so every value the layout draws reads `--`.
fn not_loaded() -> Weather {
    Weather {
        location: Location {
            display_name: display::NOT_AVAILABLE.to_owned(),
            timezone: String::new(),
        },
        current: None,
        hourly: None,
        daily: None,
    }
}

fn layout(weather: &Weather, params: &Params, frame: Frame) -> Node {
    let size = frame.size;
    match size.variant {
        SizeVariant::Full => full::full(weather, params, size),
        SizeVariant::Large => large::large(weather, params, size),
        SizeVariant::Medium => medium::medium(weather, params, size),
        SizeVariant::Small => small::small(weather, params, size),
    }
}

fn message_view(message: &str) -> Node {
    col(
        props!(background: BLACK),
        [center(
            props!(flex: 1.0),
            [common::txt(
                message.to_string(),
                32,
                FontWeight::REGULAR,
                GRAY_30,
            )],
        )],
    )
}

#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::system::{SnapshotBuilder, TimeFormat};

    use super::*;
    use crate::manifest_params::TimeZone;
    use crate::model::SizeBucket;
    use crate::screens::fixtures;

    fn install(timezone: &str) {
        assets::init_test_registrars();
        system::set_current(
            SnapshotBuilder::new()
                .timezone(timezone)
                .time_format(TimeFormat::Hour24)
                .build(),
        );
    }

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

    fn at(bucket: SizeBucket, state: fixtures::StateFixture, params: Params) -> Vec<String> {
        texts(&weather_view(&state(fixtures::at_bucket(bucket), params)))
    }

    #[test]
    fn the_time_zone_param_picks_the_zone_the_sun_is_timed_in() {
        install("Asia/Kolkata");
        let sun = |time_zone| {
            let params = Params {
                time_zone,
                ..fixtures::default_params()
            };
            at(SizeBucket::Large, fixtures::healthy, params)
        };
        let in_prague = sun(TimeZone::Location);
        assert!(
            in_prague.contains(&"04:51".to_owned()) && in_prague.contains(&"21:08".to_owned()),
            "{in_prague:?}"
        );
        let in_kolkata = sun(TimeZone::System);
        assert!(
            in_kolkata.contains(&"08:21".to_owned()) && in_kolkata.contains(&"00:38".to_owned()),
            "{in_kolkata:?}"
        );
    }

    #[test]
    fn every_size_names_the_location() {
        install("Europe/Prague");
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let texts = at(bucket, fixtures::healthy, fixtures::default_params());
            assert!(
                texts.contains(&"Prague, Czech Republic".to_owned()),
                "{bucket:?}: {texts:?}"
            );
        }
    }

    #[test]
    fn loading_draws_every_layout_with_each_value_as_a_dash() {
        install("Europe/Prague");
        for bucket in [
            SizeBucket::Full,
            SizeBucket::Large,
            SizeBucket::Medium,
            SizeBucket::Small,
            SizeBucket::Bmm101,
        ] {
            let texts = at(bucket, fixtures::loading, fixtures::default_params());
            let dashes = texts
                .iter()
                .filter(|text| *text == display::NOT_AVAILABLE)
                .count();
            assert!(
                dashes >= 3,
                "location, temperature and condition at least, {bucket:?}: {texts:?}"
            );
        }
    }

    #[test]
    fn the_stat_labels_stay_while_their_values_load() {
        install("Europe/Prague");
        let texts = at(
            SizeBucket::Large,
            fixtures::loading,
            fixtures::default_params(),
        );
        for label in ["Low T.", "High T.", "Sunrise", "Sunset"] {
            assert!(texts.contains(&label.to_owned()), "{label}: {texts:?}");
        }
    }

    #[test]
    fn a_blank_location_asks_for_one() {
        install("Europe/Prague");
        assert_eq!(
            at(
                SizeBucket::Large,
                fixtures::no_location,
                fixtures::default_params()
            ),
            [display::ENTER_LOCATION]
        );
    }

    #[test]
    fn a_location_miss_says_so() {
        install("Europe/Prague");
        assert_eq!(
            at(
                SizeBucket::Large,
                fixtures::bad_location,
                fixtures::default_params()
            ),
            ["Location not found"]
        );
    }

    #[test]
    fn a_stale_forecast_floats_its_tag_over_the_layout() {
        install("Europe/Prague");
        let children = |state: fixtures::StateFixture| {
            let view = state(
                fixtures::at_bucket(SizeBucket::Large),
                fixtures::default_params(),
            );
            let Node::Column(_, children) = weather_view(&view) else {
                panic!("BUG: every layout is a column");
            };
            children.len()
        };
        assert_eq!(children(fixtures::stale), children(fixtures::healthy) + 1);
    }
}
