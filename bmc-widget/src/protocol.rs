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

pub(crate) mod to_protocol {
    use bmc_widget_protocol::client::deck_widget_surface_v1 as p;
    use bmc_widget_protocol::{LedEffect, LedScope};

    pub fn led_effect(effect: LedEffect) -> p::LedEffect {
        match effect {
            LedEffect::Chase => p::LedEffect::Chase,
            LedEffect::KnightRider => p::LedEffect::KnightRider,
            LedEffect::Scan => p::LedEffect::Scan,
            LedEffect::Snake => p::LedEffect::Snake,
            LedEffect::Breathe => p::LedEffect::Breathe,
            LedEffect::Solid => p::LedEffect::Solid,
        }
    }

    pub fn led_scope(scope: LedScope) -> p::LedScope {
        match scope {
            LedScope::Local => p::LedScope::Local,
            LedScope::Global => p::LedScope::Global,
        }
    }
}

pub(crate) mod from_protocol {
    use bmc_widget_protocol::client::deck_widget_surface_v1 as p;
    use bmc_widget_protocol::wayland_client::WEnum;
    use bmc_widget_protocol::{
        DateFormat, NumberFormat, TemperatureUnit, TimeSystem, UnitSystem, WeekDay,
    };

    pub fn night_mode(value: WEnum<p::NightModeState>) -> Option<bool> {
        match value.into_result().ok()? {
            p::NightModeState::Off => Some(false),
            p::NightModeState::On => Some(true),
            _ => None,
        }
    }

    pub fn date_format(value: WEnum<p::DateFormat>) -> Option<DateFormat> {
        match value.into_result().ok()? {
            p::DateFormat::DdMmYyyyDot => Some(DateFormat::DdMmYyyyDot),
            p::DateFormat::DdMmYyyySlash => Some(DateFormat::DdMmYyyySlash),
            p::DateFormat::DMYyyySlash => Some(DateFormat::DMYyyySlash),
            p::DateFormat::MDYyyySlash => Some(DateFormat::MDYyyySlash),
            p::DateFormat::DdMmYyyyDash => Some(DateFormat::DdMmYyyyDash),
            p::DateFormat::YyyyMDSlash => Some(DateFormat::YyyyMDSlash),
            p::DateFormat::YyyyMmDdDot => Some(DateFormat::YyyyMmDdDot),
            p::DateFormat::YyyyMmDdDash => Some(DateFormat::YyyyMmDdDash),
            _ => None,
        }
    }

    pub fn time_format(value: WEnum<p::TimeFormat>) -> Option<TimeSystem> {
        match value.into_result().ok()? {
            p::TimeFormat::Hour12 => Some(TimeSystem::Hour12),
            p::TimeFormat::Hour24 => Some(TimeSystem::Hour24),
            _ => None,
        }
    }

    pub fn number_format(value: WEnum<p::NumberFormat>) -> Option<NumberFormat> {
        match value.into_result().ok()? {
            p::NumberFormat::SpaceGroupCommaDecimal => Some(NumberFormat::SpaceGroupCommaDecimal),
            p::NumberFormat::CommaGroupDotDecimal => Some(NumberFormat::CommaGroupDotDecimal),
            p::NumberFormat::DotGroupCommaDecimal => Some(NumberFormat::DotGroupCommaDecimal),
            p::NumberFormat::SpaceGroupDotDecimal => Some(NumberFormat::SpaceGroupDotDecimal),
            _ => None,
        }
    }

    pub fn temperature_unit(value: WEnum<p::TemperatureUnit>) -> Option<TemperatureUnit> {
        match value.into_result().ok()? {
            p::TemperatureUnit::Celsius => Some(TemperatureUnit::Celsius),
            p::TemperatureUnit::Fahrenheit => Some(TemperatureUnit::Fahrenheit),
            _ => None,
        }
    }

    pub fn weekday(value: WEnum<p::Weekday>) -> Option<WeekDay> {
        value.into_result().ok().map(WeekDay::from)
    }

    pub fn unit_system(value: WEnum<p::UnitSystem>) -> Option<UnitSystem> {
        match value.into_result().ok()? {
            p::UnitSystem::Metric => Some(UnitSystem::Metric),
            p::UnitSystem::Imperial => Some(UnitSystem::Imperial),
            _ => None,
        }
    }

    pub fn presence(value: WEnum<p::Presence>) -> Option<bool> {
        match value.into_result().ok()? {
            p::Presence::Absent => Some(false),
            p::Presence::Present => Some(true),
            _ => None,
        }
    }

    pub fn lifecycle_state(value: WEnum<p::LifecycleState>) -> Option<p::LifecycleState> {
        match value.into_result() {
            Ok(state) => Some(state),
            Err(raw) => {
                tracing::warn!("Unknown deck_widget lifecycle_state value: {raw}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::from_protocol;
    use bmc_widget_protocol::client::deck_widget_surface_v1 as protocol;
    use bmc_widget_protocol::wayland_client::WEnum;

    #[test]
    fn known_lifecycle_state_is_preserved() {
        assert_eq!(
            from_protocol::lifecycle_state(WEnum::Value(protocol::LifecycleState::Visible)),
            Some(protocol::LifecycleState::Visible),
        );
    }
}
