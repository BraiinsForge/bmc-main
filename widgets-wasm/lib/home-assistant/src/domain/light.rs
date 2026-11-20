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

/// The colour temperatures a light takes, in kelvin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KelvinRange {
    pub warmest: f64,
    pub coolest: f64,
}

/// The range Home Assistant assumes for a light that reports none.
pub const DEFAULT_KELVIN_RANGE: KelvinRange = KelvinRange {
    warmest: 2000.0,
    coolest: 6535.0,
};

#[must_use]
pub fn dims(light: &Entity) -> bool {
    takes_colour(light) || supports_any(light, &DIMMING_MODES)
}

#[must_use]
pub fn takes_colour(light: &Entity) -> bool {
    supports_any(light, &COLOUR_MODES)
}

/// Its brightness in percent, as the frontend rounds it, though never below 1 %;
/// Home Assistant reports none while the light is off.
#[must_use]
pub fn brightness_percent(light: &Entity) -> Option<u8> {
    let brightness = light.number(BRIGHTNESS)?;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=100 first"
    )]
    let percent = (brightness * 100.0 / BRIGHTNESS_MAX)
        .round()
        .clamp(1.0, 100.0) as u8;
    Some(percent)
}

/// The colour temperatures it takes, where it takes one.
#[must_use]
pub fn kelvin_range(light: &Entity) -> Option<KelvinRange> {
    supports_any(light, &[COLOR_TEMP_MODE]).then(|| KelvinRange {
        warmest: light
            .number(MIN_COLOR_TEMP_KELVIN)
            .unwrap_or(DEFAULT_KELVIN_RANGE.warmest),
        coolest: light
            .number(MAX_COLOR_TEMP_KELVIN)
            .unwrap_or(DEFAULT_KELVIN_RANGE.coolest),
    })
}

/// Its colour temperature; none while it is off, or while it shows a colour instead.
#[must_use]
pub fn kelvin(light: &Entity) -> Option<f64> {
    light.number(COLOR_TEMP_KELVIN)
}

/// A colour as Home Assistant's `hs_color` tells it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HueSaturation {
    /// In degrees, from 0 to 360.
    pub hue: f64,
    /// In percent, from 0 to 100.
    pub saturation: f64,
}

/// Its colour; none while it is off or shows a colour temperature,
/// as Home Assistant then reports the colour it works out from the kelvin.
#[must_use]
pub fn colour(light: &Entity) -> Option<HueSaturation> {
    let mode = light.text(COLOR_MODE)?;
    if !COLOUR_MODES.contains(&mode) {
        return None;
    }
    match *light.numbers(HS_COLOR)? {
        [hue, saturation] => Some(HueSaturation { hue, saturation }),
        _ => None,
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
pub fn turn_on_at_percent(percent: u8) -> ServiceCall {
    turn_on_with(ServiceData::new().integer(BRIGHTNESS_PCT, i64::from(percent)))
}

#[must_use]
pub fn turn_on_at_kelvin(kelvin: u32) -> ServiceCall {
    turn_on_with(ServiceData::new().integer(COLOR_TEMP_KELVIN, i64::from(kelvin)))
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

    #[wasm_test]
    fn only_a_light_with_a_colour_mode_beyond_on_and_off_dims() {
        assert!(dims(&modes(&["brightness"])));
        assert!(dims(&modes(&["onoff", "color_temp"])));
        assert!(dims(&modes(&["xy"])));
        assert!(!dims(&modes(&["onoff"])));
        assert!(!dims(&light(vec![])), "a light without the attribute");
    }

    #[wasm_test]
    fn only_a_colour_mode_takes_a_colour() {
        assert!(takes_colour(&modes(&["color_temp", "rgbww"])));
        assert!(!takes_colour(&modes(&[
            "brightness",
            "color_temp",
            "white"
        ])));
    }

    #[wasm_test]
    fn brightness_reads_in_percent_as_the_frontend_rounds_it() {
        let at = |brightness| {
            brightness_percent(&light(vec![(
                BRIGHTNESS,
                AttributeValue::Number(brightness),
            )]))
        };
        assert_eq!(at(255.0), Some(100));
        assert_eq!(at(153.0), Some(60));
        assert_eq!(at(130.0), Some(51), "50.98 % rounds up");
        assert_eq!(at(1.0), Some(1), "a light that is on never reads 0 %");
        assert_eq!(brightness_percent(&light(vec![])), None);
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
            Some(KelvinRange {
                warmest: 2202.0,
                coolest: 6535.0
            })
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

    fn showing(mode: &str, hs: Vec<f64>) -> Entity {
        light(vec![
            (COLOR_MODE, AttributeValue::Text(mode.to_owned())),
            (HS_COLOR, AttributeValue::Numbers(hs)),
        ])
    }

    #[wasm_test]
    fn a_colour_is_the_hue_and_the_saturation_it_reports() {
        assert_eq!(
            colour(&showing("hs", vec![12.5, 80.0])),
            Some(HueSaturation {
                hue: 12.5,
                saturation: 80.0
            })
        );
        assert_eq!(colour(&light(vec![])), None);
    }

    #[wasm_test]
    fn a_light_showing_a_colour_temperature_has_no_colour() {
        assert_eq!(colour(&showing(COLOR_TEMP_MODE, vec![27.0, 19.0])), None);
    }

    #[wasm_test]
    fn each_setting_turns_the_light_on_with_its_own_data() {
        assert_eq!(
            turn_on_at_percent(40).data,
            ServiceData::new().integer("brightness_pct", 40)
        );
        assert_eq!(
            turn_on_at_kelvin(2700).data,
            ServiceData::new().integer("color_temp_kelvin", 2700)
        );
        assert_eq!(
            turn_on_at_hue(90.0, 100.0).data,
            ServiceData::new().decimals("hs_color", &[90.0, 100.0], 0)
        );
        assert_eq!(turn_on_at_kelvin(2700).service, "turn_on");
    }
}
