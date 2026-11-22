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

//! Lights: on or off, and where a light supports them, its brightness, colour temperature and colour.

use crate::{Entity, ServiceCall, ServiceData};

pub(super) const DOMAIN: &str = "light";
const BRIGHTNESS: &str = "brightness";
const SUPPORTED_COLOR_MODES: &str = "supported_color_modes";
const COLOR_MODE: &str = "color_mode";
const MIN_COLOR_TEMP_KELVIN: &str = "min_color_temp_kelvin";
const MAX_COLOR_TEMP_KELVIN: &str = "max_color_temp_kelvin";
const COLOR_TEMP_KELVIN: &str = "color_temp_kelvin";
const HS_COLOR: &str = "hs_color";
pub const ATTRIBUTES: &[&str] = &[
    BRIGHTNESS,
    SUPPORTED_COLOR_MODES,
    COLOR_MODE,
    MIN_COLOR_TEMP_KELVIN,
    MAX_COLOR_TEMP_KELVIN,
    COLOR_TEMP_KELVIN,
    HS_COLOR,
];
const BRIGHTNESS_PCT: &str = "brightness_pct";
const BRIGHTNESS_MAX: f64 = 255.0;
/// The frontend's `lightSupportsColor`.
const COLOUR_MODES: [&str; 5] = ["hs", "xy", "rgb", "rgbw", "rgbww"];
/// With [`COLOUR_MODES`], the frontend's `lightSupportsBrightness`.
const DIMMING_MODES: [&str; 3] = ["brightness", "color_temp", "white"];
const COLOR_TEMP_MODE: &str = "color_temp";
const PLAIN_MODES: [&str; 3] = ["onoff", "brightness", "white"];

/// A light's colour temperature, which Home Assistant tells in kelvin.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct ColorTemperature(f32);

impl ColorTemperature {
    #[must_use]
    pub const fn from_kelvin(kelvin: f32) -> Self {
        Self(kelvin)
    }

    #[must_use]
    pub const fn as_kelvin(self) -> f32 {
        self.0
    }

    fn read(kelvin: f64) -> Self {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a colour temperature of a few thousand kelvin"
        )]
        Self(kelvin as f32)
    }
}

/// A brightness in whole percent, from 1 to 100: a light that is on is never at 0 %.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Brightness(u8);

impl Brightness {
    pub const FULL: Self = Self(100);

    /// `percent` clamped to 1–100.
    #[must_use]
    pub const fn from_percent(percent: u8) -> Self {
        Self(if percent < 1 {
            1
        } else if percent > 100 {
            100
        } else {
            percent
        })
    }

    #[must_use]
    pub const fn as_percent(self) -> u8 {
        self.0
    }

    /// As a share of full, from 0.01 to 1.
    #[must_use]
    pub fn share(self) -> f32 {
        f32::from(self.0) / 100.0
    }
}

/// The colour temperatures a light takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KelvinRange {
    warmest: ColorTemperature,
    coolest: ColorTemperature,
}

/// The range Home Assistant assumes for a light that reports none.
pub const DEFAULT_KELVIN_RANGE: KelvinRange = KelvinRange {
    warmest: ColorTemperature(2000.0),
    coolest: ColorTemperature(6535.0),
};

impl KelvinRange {
    /// The range between `one` and `other`, whichever way round an integration reports them.
    #[must_use]
    pub fn new(one: ColorTemperature, other: ColorTemperature) -> Self {
        if one <= other {
            Self {
                warmest: one,
                coolest: other,
            }
        } else {
            Self {
                warmest: other,
                coolest: one,
            }
        }
    }

    #[must_use]
    pub const fn warmest(self) -> ColorTemperature {
        self.warmest
    }

    #[must_use]
    pub const fn coolest(self) -> ColorTemperature {
        self.coolest
    }

    /// How far `temperature` lies from the warmest (0) to the coolest (1);
    /// the middle for a light that takes one temperature only.
    #[must_use]
    pub fn share_of(self, temperature: ColorTemperature) -> f32 {
        let span = self.coolest.0 - self.warmest.0;
        if span > 0.0 {
            ((temperature.0 - self.warmest.0) / span).clamp(0.0, 1.0)
        } else {
            0.5
        }
    }

    /// The colour temperature `share` of the way from the warmest to the coolest, in whole kelvin.
    #[must_use]
    pub fn at(self, share: f32) -> ColorTemperature {
        let span = self.coolest.0 - self.warmest.0;
        ColorTemperature((self.warmest.0 + share.clamp(0.0, 1.0) * span).round())
    }
}

/// What a light takes beyond on and off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimming {
    /// Its range, where it takes a colour temperature.
    pub warmth: Option<KelvinRange>,
    pub colour: bool,
}

/// None for a light that only switches.
#[must_use]
pub fn dimming(light: &Entity) -> Option<Dimming> {
    let colour = supports_any(light, &COLOUR_MODES);
    let dims = colour || supports_any(light, &DIMMING_MODES);
    dims.then(|| Dimming {
        warmth: kelvin_range(light),
        colour,
    })
}

/// Its brightness as the frontend rounds it; Home Assistant reports none while the light is off.
#[must_use]
pub fn brightness(light: &Entity) -> Option<Brightness> {
    let brightness = light.number(BRIGHTNESS)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100 first"
    )]
    let percent = (brightness * 100.0 / BRIGHTNESS_MAX)
        .round()
        .clamp(0.0, 100.0) as u8;
    Some(Brightness::from_percent(percent))
}

fn kelvin_range(light: &Entity) -> Option<KelvinRange> {
    let reported = |name, default: ColorTemperature| {
        light.number(name).map_or(default, ColorTemperature::read)
    };
    supports_any(light, &[COLOR_TEMP_MODE]).then(|| {
        KelvinRange::new(
            reported(MIN_COLOR_TEMP_KELVIN, DEFAULT_KELVIN_RANGE.warmest),
            reported(MAX_COLOR_TEMP_KELVIN, DEFAULT_KELVIN_RANGE.coolest),
        )
    })
}

/// A colour as Home Assistant's `hs_color` tells it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueSaturation {
    /// In degrees, from 0 to 360.
    pub hue: f64,
    /// In percent, from 0 to 100.
    pub saturation: f64,
}

/// What a light shows: Home Assistant reports a colour temperature or a colour, never both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Look {
    /// Off, in the `unknown` colour mode, or in a colour mode reported without its value.
    Unknown,
    /// A white of no stated temperature: a light that only switches or dims, or one in the `white` mode.
    Plain,
    White(ColorTemperature),
    Colour(HueSaturation),
}

/// What it shows, by its colour mode: in the colour temperature mode,
/// Home Assistant also reports the colour it works out from the kelvin.
#[must_use]
pub fn look(light: &Entity) -> Look {
    match light.text(COLOR_MODE) {
        Some(COLOR_TEMP_MODE) => light
            .number(COLOR_TEMP_KELVIN)
            .map_or(Look::Unknown, |kelvin| {
                Look::White(ColorTemperature::read(kelvin))
            }),
        Some(mode) if COLOUR_MODES.contains(&mode) => match light.numbers(HS_COLOR) {
            Some(&[hue, saturation]) => Look::Colour(HueSaturation { hue, saturation }),
            Some(_) | None => Look::Unknown,
        },
        Some(mode) if PLAIN_MODES.contains(&mode) => Look::Plain,
        Some(_) | None => Look::Unknown,
    }
}

fn supports_any(light: &Entity, modes: &[&str]) -> bool {
    light
        .list(SUPPORTED_COLOR_MODES)
        .is_some_and(|supported| supported.iter().any(|mode| modes.contains(&mode.as_str())))
}

/// Comes back at the brightness and colour the light had.
#[must_use]
pub fn turn_on() -> ServiceCall {
    ServiceCall::new(DOMAIN, "turn_on")
}

#[must_use]
pub fn turn_off() -> ServiceCall {
    ServiceCall::new(DOMAIN, "turn_off")
}

#[must_use]
pub fn turn_on_at_percent(brightness: Brightness) -> ServiceCall {
    turn_on_with(ServiceData::new().integer(BRIGHTNESS_PCT, i64::from(brightness.0)))
}

/// At `temperature` rounded to whole kelvin, as Home Assistant takes it.
#[must_use]
pub fn turn_on_at_kelvin(temperature: ColorTemperature) -> ServiceCall {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a colour temperature of a few thousand kelvin"
    )]
    let kelvin = temperature.0.round() as i64;
    turn_on_with(ServiceData::new().integer(COLOR_TEMP_KELVIN, kelvin))
}

/// `hue` in degrees and `saturation` in percent.
#[must_use]
pub fn turn_on_at_hue(hue: f64, saturation: f64) -> ServiceCall {
    turn_on_with(ServiceData::new().decimals(HS_COLOR, &[hue, saturation], 0))
}

fn turn_on_with(data: ServiceData) -> ServiceCall {
    ServiceCall { data, ..turn_on() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AttributeValue;
    use bmc_wasm_sdk::wasm_test;

    fn light(attributes: Vec<(&str, AttributeValue)>) -> Entity {
        Entity::tracked("on", attributes, ATTRIBUTES)
    }

    fn modes(modes: &[&str]) -> Entity {
        light(vec![(
            SUPPORTED_COLOR_MODES,
            AttributeValue::List(modes.iter().map(|mode| (*mode).to_owned()).collect()),
        )])
    }

    fn kelvin(kelvin: f32) -> ColorTemperature {
        ColorTemperature::from_kelvin(kelvin)
    }

    #[wasm_test]
    fn only_a_light_with_a_colour_mode_beyond_on_and_off_dims() {
        assert!(dimming(&modes(&["brightness"])).is_some());
        assert!(dimming(&modes(&["onoff", "color_temp"])).is_some());
        assert!(dimming(&modes(&["xy"])).is_some());
        assert_eq!(dimming(&modes(&["onoff"])), None);
        assert_eq!(
            dimming(&light(vec![])),
            None,
            "a light without the attribute"
        );
    }

    #[wasm_test]
    fn only_a_colour_mode_takes_a_colour() {
        let colour = |supported: &[&str]| dimming(&modes(supported)).map(|dimming| dimming.colour);
        assert_eq!(colour(&["color_temp", "rgbww"]), Some(true));
        assert_eq!(colour(&["brightness", "color_temp", "white"]), Some(false));
    }

    #[wasm_test]
    fn brightness_reads_in_percent_as_the_frontend_rounds_it() {
        let at = |brightness| {
            super::brightness(&light(vec![(
                BRIGHTNESS,
                AttributeValue::Number(brightness),
            )]))
            .map(Brightness::as_percent)
        };
        assert_eq!(at(255.0), Some(100));
        assert_eq!(at(153.0), Some(60));
        assert_eq!(at(130.0), Some(51), "50.98 % rounds up");
        assert_eq!(at(1.0), Some(1), "a light that is on never reads 0 %");
        assert_eq!(super::brightness(&light(vec![])), None);
    }

    #[wasm_test]
    fn a_brightness_stays_within_1_and_100_percent() {
        assert_eq!(Brightness::from_percent(0), Brightness::from_percent(1));
        assert_eq!(Brightness::from_percent(255), Brightness::FULL);
    }

    #[wasm_test]
    fn a_colour_temperature_range_is_the_one_the_light_reports() {
        let light = light(vec![
            (
                SUPPORTED_COLOR_MODES,
                AttributeValue::List(vec![COLOR_TEMP_MODE.to_owned()]),
            ),
            (MIN_COLOR_TEMP_KELVIN, AttributeValue::Number(2202.0)),
            (MAX_COLOR_TEMP_KELVIN, AttributeValue::Number(6535.0)),
        ]);
        assert_eq!(
            kelvin_range(&light),
            Some(KelvinRange::new(kelvin(2202.0), kelvin(6535.0)))
        );
    }

    #[wasm_test]
    fn a_colour_temperature_range_falls_back_to_home_assistants() {
        assert_eq!(
            kelvin_range(&modes(&["color_temp"])),
            Some(DEFAULT_KELVIN_RANGE)
        );
        assert_eq!(kelvin_range(&modes(&["hs"])), None);
    }

    #[wasm_test]
    fn a_range_reported_coolest_first_runs_from_its_warmest() {
        let range = KelvinRange::new(kelvin(6500.0), kelvin(2000.0));
        assert_eq!(
            (range.warmest(), range.coolest()),
            (kelvin(2000.0), kelvin(6500.0))
        );
    }

    #[wasm_test]
    fn a_share_of_the_range_is_a_whole_kelvin_and_back() {
        let range = KelvinRange::new(kelvin(2000.0), kelvin(6000.0));
        assert_eq!(range.at(0.37), kelvin(3480.0));
        assert!((range.share_of(kelvin(3000.0)) - 0.25).abs() < f32::EPSILON);
    }

    #[wasm_test]
    fn a_light_of_one_temperature_sits_in_the_middle() {
        let range = KelvinRange::new(kelvin(2700.0), kelvin(2700.0));
        assert!((range.share_of(kelvin(2700.0)) - 0.5).abs() < f32::EPSILON);
    }

    fn showing(mode: &str, attributes: Vec<(&str, AttributeValue)>) -> Entity {
        let mut all = vec![(COLOR_MODE, AttributeValue::Text(mode.to_owned()))];
        all.extend(attributes);
        light(all)
    }

    #[wasm_test]
    fn a_colour_is_the_hue_and_the_saturation_it_reports() {
        let hs = (HS_COLOR, AttributeValue::Numbers(vec![12.5, 80.0]));
        assert_eq!(
            look(&showing("hs", vec![hs])),
            Look::Colour(HueSaturation {
                hue: 12.5,
                saturation: 80.0
            })
        );
    }

    #[wasm_test]
    fn a_light_showing_a_colour_temperature_shows_a_white_not_its_colour() {
        let attributes = vec![
            (COLOR_TEMP_KELVIN, AttributeValue::Number(2700.0)),
            (HS_COLOR, AttributeValue::Numbers(vec![27.0, 19.0])),
        ];
        assert_eq!(
            look(&showing(COLOR_TEMP_MODE, attributes)),
            Look::White(kelvin(2700.0))
        );
    }

    #[wasm_test]
    fn a_light_that_switches_dims_or_shows_its_white_channel_shows_a_plain_white() {
        for mode in ["onoff", "brightness", "white"] {
            assert_eq!(look(&showing(mode, vec![])), Look::Plain, "{mode}");
        }
    }

    #[wasm_test]
    fn a_light_that_is_off_or_tells_no_value_for_its_mode_shows_no_known_look() {
        assert_eq!(look(&light(vec![])), Look::Unknown, "off: no colour mode");
        assert_eq!(look(&showing("unknown", vec![])), Look::Unknown);
        assert_eq!(
            look(&showing("hs", vec![])),
            Look::Unknown,
            "hs without hs_color"
        );
        assert_eq!(
            look(&showing(COLOR_TEMP_MODE, vec![])),
            Look::Unknown,
            "color_temp without color_temp_kelvin"
        );
    }

    #[wasm_test]
    fn each_setting_turns_the_light_on_with_its_own_data() {
        assert_eq!(
            turn_on_at_percent(Brightness::from_percent(40)).data,
            ServiceData::new().integer("brightness_pct", 40)
        );
        assert_eq!(
            turn_on_at_kelvin(kelvin(2700.4)).data,
            ServiceData::new().integer("color_temp_kelvin", 2700)
        );
        assert_eq!(
            turn_on_at_hue(90.0, 100.0).data,
            ServiceData::new().decimals("hs_color", &[90.0, 100.0], 0)
        );
        assert_eq!(turn_on_at_kelvin(kelvin(2700.0)).service, "turn_on");
    }
}
