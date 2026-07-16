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

use crate::weather_code::IconId;

#[expect(
    clippy::wildcard_imports,
    reason = "widget render uses many SDK exports"
)]
use bmc_wasm_sdk::*;

pub const TEMP_HIGH: Svg = include_svg!("assets/temp-high.svg");
pub const TEMP_LOW: Svg = include_svg!("assets/temp-low.svg");
pub const SUNRISE: Svg = include_svg!("assets/time-sunrise.svg");
pub const SUNSET: Svg = include_svg!("assets/time-sunset.svg");

const CLEAR_DAY: Svg = include_svg!("assets/clear-day.svg");
const CLEAR_NIGHT: Svg = include_svg!("assets/clear-night.svg");
const PARTLY_CLOUDY_DAY: Svg = include_svg!("assets/partly-cloudy-day.svg");
const PARTLY_CLOUDY_NIGHT: Svg = include_svg!("assets/partly-cloudy-night.svg");
const CLOUDY: Svg = include_svg!("assets/cloudy.svg");
const FOG: Svg = include_svg!("assets/fog.svg");
const DRIZZLE: Svg = include_svg!("assets/drizzle.svg");
const FREEZING_RAIN: Svg = include_svg!("assets/freezing-rain.svg");
const RAIN: Svg = include_svg!("assets/rain.svg");
const SNOW: Svg = include_svg!("assets/snow.svg");
const RAIN_SHOWERS: Svg = include_svg!("assets/rain-showers.svg");
const THUNDERSTORM: Svg = include_svg!("assets/thunderstorm.svg");
const THUNDERSTORM_HAIL: Svg = include_svg!("assets/thunderstorm-hail.svg");

#[must_use]
pub fn icon_svg(id: IconId) -> &'static Svg {
    match id {
        IconId::ClearDay => &CLEAR_DAY,
        IconId::ClearNight => &CLEAR_NIGHT,
        IconId::PartlyCloudyDay => &PARTLY_CLOUDY_DAY,
        IconId::PartlyCloudyNight => &PARTLY_CLOUDY_NIGHT,
        IconId::Cloudy | IconId::Unknown => &CLOUDY,
        IconId::Fog => &FOG,
        IconId::Drizzle => &DRIZZLE,
        IconId::FreezingRain => &FREEZING_RAIN,
        IconId::Rain => &RAIN,
        IconId::Snow => &SNOW,
        IconId::RainShowers => &RAIN_SHOWERS,
        IconId::Thunderstorm => &THUNDERSTORM,
        IconId::ThunderstormHail => &THUNDERSTORM_HAIL,
    }
}
