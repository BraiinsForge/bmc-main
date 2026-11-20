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

//! The words a binary sensor's state reads as, by its device class.
//!
//! They follow the English labels of Home Assistant's `binary_sensor/strings.json`,
//! so the Deck says what the Home Assistant frontend says.

use home_assistant::domain::binary_sensor::DeviceClass;

/// The label of `on` for a binary sensor of `device_class`, _On_ or _Off_ without a known one.
#[must_use]
pub(super) fn label(device_class: Option<DeviceClass>, on: bool) -> &'static str {
    let (off_label, on_label) = match device_class {
        Some(DeviceClass::Battery) => ("Normal", "Low"),
        Some(DeviceClass::BatteryCharging) => ("Not charging", "Charging"),
        Some(
            DeviceClass::CarbonMonoxide
            | DeviceClass::Gas
            | DeviceClass::Motion
            | DeviceClass::Occupancy
            | DeviceClass::Smoke
            | DeviceClass::Sound
            | DeviceClass::Vibration,
        ) => ("Clear", "Detected"),
        Some(DeviceClass::Cold) => ("Normal", "Cold"),
        Some(DeviceClass::Connectivity) => ("Disconnected", "Connected"),
        Some(
            DeviceClass::Door
            | DeviceClass::GarageDoor
            | DeviceClass::Opening
            | DeviceClass::Window,
        ) => ("Closed", "Open"),
        Some(DeviceClass::GlassBreak) => ("Clear", "Glass break detected"),
        Some(DeviceClass::Heat) => ("Normal", "Hot"),
        Some(DeviceClass::Light) => ("No light", "Light detected"),
        Some(DeviceClass::Lock) => ("Locked", "Unlocked"),
        Some(DeviceClass::Moisture) => ("Dry", "Wet"),
        Some(DeviceClass::Moving) => ("Not moving", "Moving"),
        Some(DeviceClass::Plug) => ("Unplugged", "Plugged in"),
        Some(DeviceClass::Power) | None => ("Off", "On"),
        Some(DeviceClass::Presence) => ("Away", "Home"),
        Some(DeviceClass::Problem) => ("OK", "Problem"),
        Some(DeviceClass::Running) => ("Not running", "Running"),
        Some(DeviceClass::Safety) => ("Safe", "Unsafe"),
        Some(DeviceClass::Tamper) => ("Clear", "Tampering detected"),
        Some(DeviceClass::Update) => ("Up-to-date", "Update available"),
    };
    if on { on_label } else { off_label }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bmc_wasm_sdk::wasm_test;

    #[wasm_test]
    fn a_device_class_reads_home_assistants_words_for_it() {
        assert_eq!(label(Some(DeviceClass::Door), true), "Open");
        assert_eq!(label(Some(DeviceClass::Door), false), "Closed");
        assert_eq!(label(Some(DeviceClass::Tamper), true), "Tampering detected");
    }

    #[wasm_test]
    fn no_known_device_class_reads_on_or_off() {
        assert_eq!(label(None, true), "On");
        assert_eq!(label(None, false), "Off");
    }
}
