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

use crate::device::DeviceFamily;

/// A family's enable-toggle manifest key, or `None` when it always runs.
#[must_use]
pub fn family_enabled_key(family: DeviceFamily) -> Option<&'static str> {
    match family {
        DeviceFamily::Bitaxe => Some("axeos_enabled"),
        DeviceFamily::Bos | DeviceFamily::Ubos => None,
    }
}

/// The render-side family filter (the driver gates polling separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filters {
    pub axeos_enabled: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            axeos_enabled: true,
        }
    }
}

impl Filters {
    #[must_use]
    pub fn is_visible(&self, family: DeviceFamily) -> bool {
        family != DeviceFamily::Bitaxe || self.axeos_enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_axeos_carries_an_enable_key() {
        assert_eq!(
            family_enabled_key(DeviceFamily::Bitaxe),
            Some("axeos_enabled")
        );
        assert_eq!(family_enabled_key(DeviceFamily::Bos), None);
        assert_eq!(family_enabled_key(DeviceFamily::Ubos), None);
    }

    #[test]
    fn axeos_hides_only_when_disabled_others_always_show() {
        let on = Filters::default();
        assert!(on.is_visible(DeviceFamily::Bitaxe));
        assert!(on.is_visible(DeviceFamily::Bos));

        let off = Filters {
            axeos_enabled: false,
        };
        assert!(!off.is_visible(DeviceFamily::Bitaxe), "axeos hidden");
        assert!(off.is_visible(DeviceFamily::Bos), "bos always shown");
        assert!(off.is_visible(DeviceFamily::Ubos), "ubos always shown");
    }
}
