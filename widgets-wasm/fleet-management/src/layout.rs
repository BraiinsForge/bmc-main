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

/// Which fleet layout a viewport can fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Overview row plus the per-model breakdown table.
    Table,
    /// Compact summary-only screen for viewports too small for the table.
    Summary,
}

/// Choose a layout from the viewport size. The breakdown table needs the
/// `Large` box (638x480); a viewport smaller in either dimension cannot fit its
/// columns (width) or its rows (height), so it falls back to the summary.
#[must_use]
pub fn choose(width: u32, height: u32) -> Layout {
    if width >= TABLE_MIN_WIDTH && height >= TABLE_MIN_HEIGHT {
        Layout::Table
    } else {
        Layout::Summary
    }
}

const TABLE_MIN_WIDTH: u32 = 638;
const TABLE_MIN_HEIGHT: u32 = 480;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_and_full_boxes_get_the_table() {
        assert_eq!(choose(638, 480), Layout::Table, "Large box");
        assert_eq!(choose(1280, 480), Layout::Table, "Full box");
    }

    #[test]
    fn small_devices_get_the_summary() {
        assert_eq!(choose(480, 320), Layout::Summary, "BMM101");
        assert_eq!(choose(320, 240), Layout::Summary, "BMM100");
    }

    #[test]
    fn narrow_or_short_viewports_get_the_summary() {
        assert_eq!(choose(638, 238), Layout::Summary, "short 2x1 strip");
        assert_eq!(choose(480, 480), Layout::Summary, "narrow round BFM");
    }
}
