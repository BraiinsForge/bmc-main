// Copyright (C) 2026  Braiins Systems s.r.o.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IconId {
    ClearDay,
    ClearNight,
    PartlyCloudyDay,
    PartlyCloudyNight,
    Cloudy,
    Fog,
    Drizzle,
    FreezingRain,
    Rain,
    Snow,
    RainShowers,
    Thunderstorm,
    ThunderstormHail,
    Unknown,
}

#[must_use]
pub fn icon_id(code: i64, is_day: bool) -> IconId {
    match code {
        0 if is_day => IconId::ClearDay,
        0 => IconId::ClearNight,
        1 | 2 if is_day => IconId::PartlyCloudyDay,
        1 | 2 => IconId::PartlyCloudyNight,
        3 => IconId::Cloudy,
        45..=48 => IconId::Fog,
        51..=55 => IconId::Drizzle,
        56..=57 | 66..=67 => IconId::FreezingRain,
        61..=65 => IconId::Rain,
        71..=77 | 85..=86 => IconId::Snow,
        80..=82 => IconId::RainShowers,
        95 => IconId::Thunderstorm,
        96..=99 => IconId::ThunderstormHail,
        _ => IconId::Unknown,
    }
}

#[must_use]
pub fn description(code: i64) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mainly Clear",
        2 => "Partly Cloudy",
        3 => "Overcast",
        45..=48 => "Foggy",
        51..=55 => "Drizzle",
        56..=57 => "Freezing Drizzle",
        61..=65 => "Rain",
        66..=67 => "Freezing Rain",
        71..=75 => "Snow",
        77 => "Snow Grains",
        80..=82 => "Rain Showers",
        85..=86 => "Snow Showers",
        95 => "Thunderstorm",
        96..=99 => "Thunderstorm with Hail",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_picks_day_or_night_icon() {
        assert_eq!(icon_id(0, true), IconId::ClearDay);
        assert_eq!(icon_id(0, false), IconId::ClearNight);
        assert_eq!(icon_id(1, false), IconId::PartlyCloudyNight);
        assert_eq!(icon_id(2, false), IconId::PartlyCloudyNight);
    }

    #[test]
    fn ranges_map_to_expected_icons() {
        assert_eq!(icon_id(2, true), IconId::PartlyCloudyDay);
        assert_eq!(icon_id(3, true), IconId::Cloudy);
        assert_eq!(icon_id(48, true), IconId::Fog);
        assert_eq!(icon_id(55, true), IconId::Drizzle);
        assert_eq!(icon_id(57, true), IconId::FreezingRain);
        assert_eq!(icon_id(63, true), IconId::Rain);
        assert_eq!(icon_id(67, true), IconId::FreezingRain);
        assert_eq!(icon_id(75, true), IconId::Snow);
        assert_eq!(icon_id(82, true), IconId::RainShowers);
        assert_eq!(icon_id(86, true), IconId::Snow);
        assert_eq!(icon_id(95, true), IconId::Thunderstorm);
        assert_eq!(icon_id(99, true), IconId::ThunderstormHail);
    }

    #[test]
    fn unmatched_codes_are_unknown_not_hail() {
        assert_eq!(icon_id(10, true), IconId::Unknown);
        assert_eq!(icon_id(50, true), IconId::Unknown);
        assert_eq!(description(10), "Unknown");
    }

    #[test]
    fn descriptions_match_deckfeeder() {
        assert_eq!(description(2), "Partly Cloudy");
        assert_eq!(description(3), "Overcast");
        assert_eq!(description(77), "Snow Grains");
        assert_eq!(description(96), "Thunderstorm with Hail");
    }
}
