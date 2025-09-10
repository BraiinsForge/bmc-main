// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::generated::AnalogClockHands;
use chrono::{DateTime, FixedOffset, Timelike};
use slint::Image;
use svg::Document;
use svg::node::element::tag::Type;
use svg::node::element::{Group, Path};
use svg::parser::Event;
use tracing::error;

const SECOND_HAND_ROUND: &str =
    include_str!("../../bmc-display/ui/assets/images/clock/hand-second.svg");
const MINUTE_HAND_ROUND: &str =
    include_str!("../../bmc-display/ui/assets/images/clock/hand-minute.svg");
const HOUR_HAND_ROUND: &str =
    include_str!("../../bmc-display/ui/assets/images/clock/hand-hour.svg");
const SECOND_HAND_RECT: &str =
    include_str!("../../bmc-display/ui/assets/images/clock/hand-second-rect.svg");
const MINUTE_HAND_RECT: &str =
    include_str!("../../bmc-display/ui/assets/images/clock/hand-minute-rect.svg");
const HOUR_HAND_RECT: &str =
    include_str!("../../bmc-display/ui/assets/images/clock/hand-hour-rect.svg");

#[derive(Clone, Copy, Debug, Default)]
pub struct ClockData {
    now: DateTime<FixedOffset>,
}

impl ClockData {
    #[must_use]
    pub fn new(now: DateTime<FixedOffset>) -> Self {
        Self { now }
    }

    #[must_use]
    pub fn into_clock_hand_images(self) -> AnalogClockHands {
        let second = self.now.second();
        let minute = self.now.minute();
        let hour = self.now.hour();

        let rotation_angle_sec = i32::try_from(second).unwrap_or_default() * 6;
        let rotation_angle_min = i32::try_from(minute).unwrap_or_default() * 6;
        #[expect(clippy::integer_division)]
        let rotation_angle_hour =
            i32::try_from(hour).unwrap_or_default() * 30 + rotation_angle_min / 12;

        let second_hand_round = rotate_svg(SECOND_HAND_ROUND, rotation_angle_sec, 2, 198);
        let second_hand_rect = rotate_svg(SECOND_HAND_RECT, rotation_angle_sec, 2, 239);

        let minute_hand_round = rotate_svg(MINUTE_HAND_ROUND, rotation_angle_min, 25, 200);
        let hour_hand_round = rotate_svg(HOUR_HAND_ROUND, rotation_angle_hour, 31, 121);
        let minute_hand_rect = rotate_svg(MINUTE_HAND_RECT, rotation_angle_min, 27, 242);
        let hour_hand_rect = rotate_svg(HOUR_HAND_RECT, rotation_angle_hour, 27, 138);

        AnalogClockHands {
            second_hand_round,
            minute_hand_round,
            hour_hand_round,
            second_hand_rect,
            minute_hand_rect,
            hour_hand_rect,
        }
    }
}

// FIXME: Move to graph_utils
fn rotate_svg(svg_data: &str, rotation_angle: i32, rot_origin_x: u32, rot_origin_y: u32) -> Image {
    let parser = match svg::read(svg_data) {
        Ok(parser) => parser,
        Err(e) => {
            error!("Reading svg failed: {}", e);
            return Image::default();
        }
    };
    let rotate = format!("rotate({rotation_angle}, {rot_origin_x}, {rot_origin_y})");

    let mut document = Document::new();
    let mut group = Group::new();

    for event in parser {
        #[expect(clippy::wildcard_enum_match_arm)]
        match event {
            Event::Tag("svg", Type::Start, el_attributes) => {
                for (key, val) in &el_attributes {
                    document = document.set(key, val.clone());
                }
            }
            Event::Tag("path", _, el_attributes) => {
                let mut path = Path::new();
                for (key, val) in &el_attributes {
                    path = path.set(key, val.clone());
                }
                path = path.set("transform", rotate.clone());
                group = group.add(path);
            }
            _ => {}
        }
    }
    document = document.add(group);

    let mut svg_image: Vec<u8> = vec![];
    if let Err(e) = svg::write(&mut svg_image, &document) {
        error!("Writing svg document failed: {}", e);
        return Image::default();
    }

    Image::load_from_svg_data(&svg_image).unwrap_or_default()
}
