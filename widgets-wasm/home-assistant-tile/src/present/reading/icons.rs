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

//! The frontend's icon for a sensor or a binary sensor, by its device class and its state:
//! Home Assistant core's `sensor/icons.json` and `binary_sensor/icons.json`.

use bmc_wasm_sdk::{Svg, include_svg};
use home_assistant::domain::binary_sensor::DeviceClass;

use crate::view::icons::{ARROW_LEFT, ARROW_RIGHT, POWER_PLUG, POWER_PLUG_OFF};
use crate::view::style::Icon;

const AIR_FILTER: Svg = include_svg!("assets/mdi/air-filter.svg");
const ALERT_CIRCLE: Svg = include_svg!("assets/mdi/alert-circle.svg");
const ANGLE_ACUTE: Svg = include_svg!("assets/mdi/angle-acute.svg");
const ARROW_BOTTOM_LEFT: Svg = include_svg!("assets/mdi/arrow-bottom-left.svg");
const ARROW_BOTTOM_RIGHT: Svg = include_svg!("assets/mdi/arrow-bottom-right.svg");
const ARROW_DOWN: Svg = include_svg!("assets/mdi/arrow-down.svg");
const ARROW_LEFT_RIGHT: Svg = include_svg!("assets/mdi/arrow-left-right.svg");
const ARROW_TOP_LEFT: Svg = include_svg!("assets/mdi/arrow-top-left.svg");
const ARROW_TOP_RIGHT: Svg = include_svg!("assets/mdi/arrow-top-right.svg");
const ARROW_UP: Svg = include_svg!("assets/mdi/arrow-up.svg");
const BATTERY: Svg = include_svg!("assets/mdi/battery.svg");
const BATTERY_10: Svg = include_svg!("assets/mdi/battery-10.svg");
const BATTERY_20: Svg = include_svg!("assets/mdi/battery-20.svg");
const BATTERY_30: Svg = include_svg!("assets/mdi/battery-30.svg");
const BATTERY_40: Svg = include_svg!("assets/mdi/battery-40.svg");
const BATTERY_50: Svg = include_svg!("assets/mdi/battery-50.svg");
const BATTERY_60: Svg = include_svg!("assets/mdi/battery-60.svg");
const BATTERY_70: Svg = include_svg!("assets/mdi/battery-70.svg");
const BATTERY_80: Svg = include_svg!("assets/mdi/battery-80.svg");
const BATTERY_90: Svg = include_svg!("assets/mdi/battery-90.svg");
const BATTERY_ALERT: Svg = include_svg!("assets/mdi/battery-alert.svg");
const BATTERY_CHARGING: Svg = include_svg!("assets/mdi/battery-charging.svg");
const BATTERY_OUTLINE: Svg = include_svg!("assets/mdi/battery-outline.svg");
const BATTERY_UNKNOWN: Svg = include_svg!("assets/mdi/battery-unknown.svg");
const BRIGHTNESS_5: Svg = include_svg!("assets/mdi/brightness-5.svg");
const BRIGHTNESS_7: Svg = include_svg!("assets/mdi/brightness-7.svg");
const CALENDAR: Svg = include_svg!("assets/mdi/calendar.svg");
const CAR_BATTERY: Svg = include_svg!("assets/mdi/car-battery.svg");
const CAR_COOLANT_LEVEL: Svg = include_svg!("assets/mdi/car-coolant-level.svg");
const CASH: Svg = include_svg!("assets/mdi/cash.svg");
const CHECK_CIRCLE: Svg = include_svg!("assets/mdi/check-circle.svg");
const CHECK_NETWORK_OUTLINE: Svg = include_svg!("assets/mdi/check-network-outline.svg");
const CHECKBOX_MARKED_CIRCLE: Svg = include_svg!("assets/mdi/checkbox-marked-circle.svg");
const CLOCK: Svg = include_svg!("assets/mdi/clock.svg");
const CLOCK_START: Svg = include_svg!("assets/mdi/clock-start.svg");
const CLOSE_NETWORK_OUTLINE: Svg = include_svg!("assets/mdi/close-network-outline.svg");
const COMPASS_ROSE: Svg = include_svg!("assets/mdi/compass-rose.svg");
const CROP_PORTRAIT: Svg = include_svg!("assets/mdi/crop-portrait.svg");
const CURRENT_AC: Svg = include_svg!("assets/mdi/current-ac.svg");
const DATABASE: Svg = include_svg!("assets/mdi/database.svg");
const DOOR_CLOSED: Svg = include_svg!("assets/mdi/door-closed.svg");
const DOOR_OPEN: Svg = include_svg!("assets/mdi/door-open.svg");
const EAR_HEARING: Svg = include_svg!("assets/mdi/ear-hearing.svg");
const EYE: Svg = include_svg!("assets/mdi/eye.svg");
const FIRE: Svg = include_svg!("assets/mdi/fire.svg");
const FLASH: Svg = include_svg!("assets/mdi/flash.svg");
const GARAGE: Svg = include_svg!("assets/mdi/garage.svg");
const GARAGE_OPEN: Svg = include_svg!("assets/mdi/garage-open.svg");
const GAUGE: Svg = include_svg!("assets/mdi/gauge.svg");
const GLASS_FRAGILE: Svg = include_svg!("assets/mdi/glass-fragile.svg");
const HOME: Svg = include_svg!("assets/mdi/home.svg");
const HOME_OUTLINE: Svg = include_svg!("assets/mdi/home-outline.svg");
const LIGHTNING_BOLT: Svg = include_svg!("assets/mdi/lightning-bolt.svg");
const LOCK: Svg = include_svg!("assets/mdi/lock.svg");
const LOCK_OPEN: Svg = include_svg!("assets/mdi/lock-open.svg");
const METER_GAS: Svg = include_svg!("assets/mdi/meter-gas.svg");
const MOLECULE: Svg = include_svg!("assets/mdi/molecule.svg");
const MOLECULE_CO: Svg = include_svg!("assets/mdi/molecule-co.svg");
const MOLECULE_CO2: Svg = include_svg!("assets/mdi/molecule-co2.svg");
const MOTION_SENSOR: Svg = include_svg!("assets/mdi/motion-sensor.svg");
const MOTION_SENSOR_OFF: Svg = include_svg!("assets/mdi/motion-sensor-off.svg");
const MUSIC_NOTE: Svg = include_svg!("assets/mdi/music-note.svg");
const MUSIC_NOTE_OFF: Svg = include_svg!("assets/mdi/music-note-off.svg");
const OCTAGON: Svg = include_svg!("assets/mdi/octagon.svg");
const PACKAGE: Svg = include_svg!("assets/mdi/package.svg");
const PACKAGE_UP: Svg = include_svg!("assets/mdi/package-up.svg");
const PH: Svg = include_svg!("assets/mdi/ph.svg");
const PIPE_VALVE: Svg = include_svg!("assets/mdi/pipe-valve.svg");
const PLAY: Svg = include_svg!("assets/mdi/play.svg");
const PROGRESS_CLOCK: Svg = include_svg!("assets/mdi/progress-clock.svg");
const RADIOACTIVE: Svg = include_svg!("assets/mdi/radioactive.svg");
const RADIOBOX_BLANK: Svg = include_svg!("assets/mdi/radiobox-blank.svg");
const SINE_WAVE: Svg = include_svg!("assets/mdi/sine-wave.svg");
const SMOKE_DETECTOR: Svg = include_svg!("assets/mdi/smoke-detector.svg");
const SMOKE_DETECTOR_ALERT: Svg = include_svg!("assets/mdi/smoke-detector-alert.svg");
const SMOKE_DETECTOR_VARIANT: Svg = include_svg!("assets/mdi/smoke-detector-variant.svg");
const SMOKE_DETECTOR_VARIANT_ALERT: Svg =
    include_svg!("assets/mdi/smoke-detector-variant-alert.svg");
const SNOWFLAKE: Svg = include_svg!("assets/mdi/snowflake.svg");
const SPEEDOMETER: Svg = include_svg!("assets/mdi/speedometer.svg");
const SPOON_SUGAR: Svg = include_svg!("assets/mdi/spoon-sugar.svg");
const SPROUT_OUTLINE: Svg = include_svg!("assets/mdi/sprout-outline.svg");
const SQUARE: Svg = include_svg!("assets/mdi/square.svg");
const SQUARE_OUTLINE: Svg = include_svg!("assets/mdi/square-outline.svg");
const STOP: Svg = include_svg!("assets/mdi/stop.svg");
const STORAGE_TANK: Svg = include_svg!("assets/mdi/storage-tank.svg");
const SUN_WIRELESS: Svg = include_svg!("assets/mdi/sun-wireless.svg");
const TEXTURE_BOX: Svg = include_svg!("assets/mdi/texture-box.svg");
const THERMOMETER: Svg = include_svg!("assets/mdi/thermometer.svg");
const THERMOMETER_LINES: Svg = include_svg!("assets/mdi/thermometer-lines.svg");
const TRANSMISSION_TOWER: Svg = include_svg!("assets/mdi/transmission-tower.svg");
const VIBRATE: Svg = include_svg!("assets/mdi/vibrate.svg");
const WATER: Svg = include_svg!("assets/mdi/water.svg");
const WATER_OFF: Svg = include_svg!("assets/mdi/water-off.svg");
const WATER_OPACITY: Svg = include_svg!("assets/mdi/water-opacity.svg");
const WATER_PERCENT: Svg = include_svg!("assets/mdi/water-percent.svg");
const WEATHER_POURING: Svg = include_svg!("assets/mdi/weather-pouring.svg");
const WEATHER_RAINY: Svg = include_svg!("assets/mdi/weather-rainy.svg");
const WEATHER_WINDY: Svg = include_svg!("assets/mdi/weather-windy.svg");
const WEIGHT: Svg = include_svg!("assets/mdi/weight.svg");
const WIFI: Svg = include_svg!("assets/mdi/wifi.svg");
const WINDOW_CLOSED_VARIANT: Svg = include_svg!("assets/mdi/window-closed-variant.svg");
const WINDOW_OPEN_VARIANT: Svg = include_svg!("assets/mdi/window-open-variant.svg");

/// `battery`'s icons from the lowest value each is drawn from.
const BATTERY_RANGE: [(f64, &Svg); 11] = [
    (0.0, &BATTERY_ALERT),
    (10.0, &BATTERY_10),
    (20.0, &BATTERY_20),
    (30.0, &BATTERY_30),
    (40.0, &BATTERY_40),
    (50.0, &BATTERY_50),
    (60.0, &BATTERY_60),
    (70.0, &BATTERY_70),
    (80.0, &BATTERY_80),
    (90.0, &BATTERY_90),
    (100.0, &BATTERY),
];

/// `wind_direction`'s icons from the lowest value each is drawn from.
const WIND_DIRECTION_RANGE: [(f64, &Svg); 9] = [
    (0.0, &ARROW_DOWN),
    (22.5, &ARROW_BOTTOM_LEFT),
    (67.5, &ARROW_LEFT),
    (112.5, &ARROW_TOP_LEFT),
    (157.5, &ARROW_UP),
    (202.5, &ARROW_TOP_RIGHT),
    (247.5, &ARROW_RIGHT),
    (292.5, &ARROW_BOTTOM_RIGHT),
    (337.5, &ARROW_DOWN),
];

/// A sensor's icon. `value` picks one where its device class draws a range,
/// such as a battery's charge: the one drawn from the highest value at or under it.
#[must_use]
pub fn sensor(device_class: Option<&str>, value: Option<f64>) -> Icon {
    let range: &[(f64, &Svg)] = match device_class {
        Some("battery") => &BATTERY_RANGE,
        Some("wind_direction") => &WIND_DIRECTION_RANGE,
        _ => &[],
    };
    let ranged = value.and_then(|value| {
        range
            .iter()
            .take_while(|(from, _)| *from <= value)
            .last()
            .map(|(_, svg)| *svg)
    });
    Icon(ranged.unwrap_or_else(|| sensor_default(device_class)))
}

/// A class drawn with the domain's default, such as `enum`, falls through to it.
fn sensor_default(device_class: Option<&str>) -> &'static Svg {
    match device_class {
        Some("absolute_humidity") => &WATER_OPACITY,
        Some("apparent_power" | "power" | "reactive_power") => &FLASH,
        Some("aqi") => &AIR_FILTER,
        Some("area") => &TEXTURE_BOX,
        Some("atmospheric_pressure") => &THERMOMETER_LINES,
        Some("battery") => &BATTERY_UNKNOWN,
        Some("blood_glucose_concentration") => &SPOON_SUGAR,
        Some("carbon_dioxide") => &MOLECULE_CO2,
        Some("carbon_monoxide") => &MOLECULE_CO,
        Some("conductivity") => &SPROUT_OUTLINE,
        Some("current") => &CURRENT_AC,
        Some("data_rate") => &TRANSMISSION_TOWER,
        Some("data_size") => &DATABASE,
        Some("date") => &CALENDAR,
        Some("distance") => &ARROW_LEFT_RIGHT,
        Some("duration") => &PROGRESS_CLOCK,
        Some("energy" | "reactive_energy") => &LIGHTNING_BOLT,
        Some("energy_storage") => &CAR_BATTERY,
        Some("frequency" | "voltage") => &SINE_WAVE,
        Some("gas") => &METER_GAS,
        Some("humidity" | "moisture") => &WATER_PERCENT,
        Some("illuminance") => &BRIGHTNESS_5,
        Some("irradiance") => &SUN_WIRELESS,
        Some("monetary") => &CASH,
        Some(
            "nitrogen_dioxide"
            | "nitrogen_monoxide"
            | "nitrous_oxide"
            | "ozone"
            | "pm1"
            | "pm10"
            | "pm25"
            | "pm4"
            | "sulphur_dioxide"
            | "volatile_organic_compounds"
            | "volatile_organic_compounds_parts",
        ) => &MOLECULE,
        Some("ph") => &PH,
        Some("power_factor") => &ANGLE_ACUTE,
        Some("precipitation") => &WEATHER_RAINY,
        Some("precipitation_intensity") => &WEATHER_POURING,
        Some("pressure") => &GAUGE,
        Some("radon") => &RADIOACTIVE,
        Some("signal_strength") => &WIFI,
        Some("sound_pressure") => &EAR_HEARING,
        Some("speed") => &SPEEDOMETER,
        Some("temperature" | "temperature_delta") => &THERMOMETER,
        Some("timestamp") => &CLOCK,
        Some("uptime") => &CLOCK_START,
        Some("volume") => &CAR_COOLANT_LEVEL,
        Some("volume_flow_rate") => &PIPE_VALVE,
        Some("volume_storage") => &STORAGE_TANK,
        Some("water") => &WATER,
        Some("weight") => &WEIGHT,
        Some("wind_direction") => &COMPASS_ROSE,
        Some("wind_speed") => &WEATHER_WINDY,
        _ => &EYE,
    }
}

/// A binary sensor's icon while it is `on`, or else.
#[must_use]
pub fn binary_sensor(device_class: Option<DeviceClass>, on: bool) -> Icon {
    let (off_icon, on_icon): (&Svg, &Svg) = match device_class {
        Some(DeviceClass::Battery) => (&BATTERY, &BATTERY_OUTLINE),
        Some(DeviceClass::BatteryCharging) => (&BATTERY, &BATTERY_CHARGING),
        Some(DeviceClass::CarbonMonoxide) => (&SMOKE_DETECTOR, &SMOKE_DETECTOR_ALERT),
        Some(DeviceClass::Cold) => (&THERMOMETER, &SNOWFLAKE),
        Some(DeviceClass::Connectivity) => (&CLOSE_NETWORK_OUTLINE, &CHECK_NETWORK_OUTLINE),
        Some(DeviceClass::Door) => (&DOOR_CLOSED, &DOOR_OPEN),
        Some(DeviceClass::GarageDoor) => (&GARAGE, &GARAGE_OPEN),
        Some(
            DeviceClass::Gas | DeviceClass::Problem | DeviceClass::Safety | DeviceClass::Tamper,
        ) => (&CHECK_CIRCLE, &ALERT_CIRCLE),
        Some(DeviceClass::GlassBreak) => (&GLASS_FRAGILE, &GLASS_FRAGILE),
        Some(DeviceClass::Heat) => (&THERMOMETER, &FIRE),
        Some(DeviceClass::Light) => (&BRIGHTNESS_5, &BRIGHTNESS_7),
        Some(DeviceClass::Lock) => (&LOCK, &LOCK_OPEN),
        Some(DeviceClass::Moisture) => (&WATER_OFF, &WATER),
        Some(DeviceClass::Motion) => (&MOTION_SENSOR_OFF, &MOTION_SENSOR),
        Some(DeviceClass::Moving) => (&OCTAGON, &ARROW_RIGHT),
        Some(DeviceClass::Occupancy | DeviceClass::Presence) => (&HOME_OUTLINE, &HOME),
        Some(DeviceClass::Opening) => (&SQUARE, &SQUARE_OUTLINE),
        Some(DeviceClass::Plug | DeviceClass::Power) => (&POWER_PLUG_OFF, &POWER_PLUG),
        Some(DeviceClass::Running) => (&STOP, &PLAY),
        Some(DeviceClass::Smoke) => (&SMOKE_DETECTOR_VARIANT, &SMOKE_DETECTOR_VARIANT_ALERT),
        Some(DeviceClass::Sound) => (&MUSIC_NOTE_OFF, &MUSIC_NOTE),
        Some(DeviceClass::Update) => (&PACKAGE, &PACKAGE_UP),
        Some(DeviceClass::Vibration) => (&CROP_PORTRAIT, &VIBRATE),
        Some(DeviceClass::Window) => (&WINDOW_CLOSED_VARIANT, &WINDOW_OPEN_VARIANT),
        None => (&RADIOBOX_BLANK, &CHECKBOX_MARKED_CIRCLE),
    };
    Icon(if on { on_icon } else { off_icon })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_sensor_wears_its_device_classs_icon_or_the_default() {
        assert_eq!(sensor(Some("temperature"), Some(21.5)), Icon(&THERMOMETER));
        assert_eq!(sensor(Some("teleporter"), None), Icon(&EYE));
        assert_eq!(sensor(None, Some(3.0)), Icon(&EYE));
    }

    #[wasm_test]
    fn a_battery_shows_its_charge_in_steps_of_ten() {
        let cases = [
            (Some(0.0), &BATTERY_ALERT),
            (Some(9.9), &BATTERY_ALERT),
            (Some(10.0), &BATTERY_10),
            (Some(55.0), &BATTERY_50),
            (Some(100.0), &BATTERY),
            (Some(-1.0), &BATTERY_UNKNOWN),
            (None, &BATTERY_UNKNOWN),
        ];
        for (charge, icon) in cases {
            assert_eq!(sensor(Some("battery"), charge), Icon(icon), "{charge:?}");
        }
    }

    #[wasm_test]
    fn a_wind_direction_points_where_the_wind_blows_to() {
        let cases = [
            (0.0, &ARROW_DOWN),
            (90.0, &ARROW_LEFT),
            (180.0, &ARROW_UP),
            (350.0, &ARROW_DOWN),
        ];
        for (degrees, icon) in cases {
            assert_eq!(
                sensor(Some("wind_direction"), Some(degrees)),
                Icon(icon),
                "{degrees}"
            );
        }
    }

    #[wasm_test]
    fn a_binary_sensor_shows_whether_it_is_on() {
        assert_eq!(
            binary_sensor(Some(DeviceClass::Door), true),
            Icon(&DOOR_OPEN)
        );
        assert_eq!(
            binary_sensor(Some(DeviceClass::Door), false),
            Icon(&DOOR_CLOSED)
        );
        assert_eq!(
            binary_sensor(Some(DeviceClass::GlassBreak), true),
            Icon(&GLASS_FRAGILE),
            "a class without an icon of its own for on keeps its default"
        );
        assert_eq!(binary_sensor(None, true), Icon(&CHECKBOX_MARKED_CIRCLE));
        assert_eq!(binary_sensor(None, false), Icon(&RADIOBOX_BLANK));
    }
}
