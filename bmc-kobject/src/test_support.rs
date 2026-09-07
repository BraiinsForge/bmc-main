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

//! Netlink packets captured from real devices, for tests that parse or route them.

/// `ACTION=pressed BUTTON=BTN_0` as gpio-button-hotplug sent it on a BMM board:
/// the kernel names the ip-report button after its keycode, not its device-tree label.
pub const BMM_BTN_0_PRESSED: &[u8] = &[
    112, 114, 101, 115, 115, 101, 100, 64, 0, 72, 79, 77, 69, 61, 47, 0, 80, 65, 84, 72, 61, 47,
    115, 98, 105, 110, 58, 47, 98, 105, 110, 58, 47, 117, 115, 114, 47, 115, 98, 105, 110, 58, 47,
    117, 115, 114, 47, 98, 105, 110, 0, 83, 85, 66, 83, 89, 83, 84, 69, 77, 61, 98, 117, 116, 116,
    111, 110, 0, 65, 67, 84, 73, 79, 78, 61, 112, 114, 101, 115, 115, 101, 100, 0, 66, 85, 84, 84,
    79, 78, 61, 66, 84, 78, 95, 48, 0, 83, 69, 69, 78, 61, 49, 51, 48, 0, 83, 69, 81, 78, 85, 77,
    61, 49, 50, 48, 53, 0,
];
