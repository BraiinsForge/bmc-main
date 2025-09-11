// Copyright (C) 2025  Braiins Systems s.r.o.

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
    #[inline]
    pub fn hour_hand_round(&self) -> Image {
        let rotation_angle = self.hour_hand_angle();

        rotate_svg(HOUR_HAND_ROUND, rotation_angle, 31, 121)
    }

    #[must_use]
    #[inline]
    pub fn hour_hand_rect(&self) -> Image {
        let rotation_angle = self.hour_hand_angle();

        rotate_svg(HOUR_HAND_RECT, rotation_angle, 27, 138)
    }

    #[must_use]
    #[inline]
    pub fn minute_hand_round(&self) -> Image {
        let rotation_angle = self.minute_hand_angle();

        rotate_svg(MINUTE_HAND_ROUND, rotation_angle, 25, 200)
    }

    #[must_use]
    #[inline]
    pub fn minute_hand_rect(&self) -> Image {
        let rotation_angle = self.minute_hand_angle();

        rotate_svg(MINUTE_HAND_RECT, rotation_angle, 27, 242)
    }

    #[must_use]
    #[inline]
    pub fn second_hand_round(&self) -> Image {
        let rotation_angle = self.second_hand_angle();

        rotate_svg(SECOND_HAND_ROUND, rotation_angle, 2, 198)
    }

    #[must_use]
    #[inline]
    pub fn second_hand_rect(&self) -> Image {
        let rotation_angle = self.second_hand_angle();

        rotate_svg(SECOND_HAND_RECT, rotation_angle, 2, 239)
    }

    #[must_use]
    #[inline]
    fn hour_hand_angle(&self) -> i32 {
        let hour = self.now.hour();

        let rotation_angle_min = self.minute_hand_angle();
        #[expect(clippy::integer_division)]
        let rotation_angle_hour =
            i32::try_from(hour).unwrap_or_default() * 30 + rotation_angle_min / 12;

        rotation_angle_hour
    }

    #[must_use]
    #[inline]
    fn minute_hand_angle(&self) -> i32 {
        let minute = self.now.minute();

        i32::try_from(minute).unwrap_or_default() * 6
    }

    #[must_use]
    #[inline]
    fn second_hand_angle(&self) -> i32 {
        let second = self.now.second();

        i32::try_from(second).unwrap_or_default() * 6
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
