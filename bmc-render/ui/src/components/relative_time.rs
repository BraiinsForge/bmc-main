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

//! `RelativeTimeLive` node builder.

use crate::relative_time::{RelTimeClamp, RelTimeFormat};
use crate::text::TextStyle;
use crate::time::SystemTime;
use crate::tree::Node;

/// Self-updating relative-time label anchored at `anchor`; the host formats
/// `now - anchor` against its clock. `clamp` pins the direction (e.g.
/// `ElapsedOnly` keeps a "last refresh" pill from ever reading "in …").
#[must_use]
pub fn relative_time_live(
    anchor: SystemTime,
    format: RelTimeFormat,
    clamp: RelTimeClamp,
    style: TextStyle,
) -> Node {
    Node::RelTime {
        anchor: anchor.unix_secs,
        format,
        clamp,
        style,
    }
}

#[cfg(test)]
mod tests {
    use crate::relative_time::{RelTimeClamp, RelTimeFormat, RelTimeLength, RelTimeSegments};
    use crate::text::TextStyle;
    use crate::time::SystemTime;
    use crate::tree::Node;

    const FMT: RelTimeFormat = RelTimeFormat {
        length: RelTimeLength::Short,
        segments: RelTimeSegments::Single,
    };

    #[test]
    fn builder_carries_anchor_seconds_and_requested_clamp() {
        let node = super::relative_time_live(
            SystemTime { unix_secs: 42 },
            FMT,
            RelTimeClamp::ElapsedOnly,
            TextStyle::default(),
        );
        assert!(matches!(
            node,
            Node::RelTime {
                anchor: 42,
                clamp: RelTimeClamp::ElapsedOnly,
                ..
            }
        ));
    }
}
