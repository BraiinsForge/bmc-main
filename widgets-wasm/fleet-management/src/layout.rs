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

/// Truncate to `max_chars` characters, ending in `…` when cut. The render
/// engine has no text ellipsis, so overlong labels must be cut in code to
/// keep table rows from wrapping.
#[must_use]
pub fn truncate_label(label: &str, max_chars: usize) -> String {
    if label.chars().count() <= max_chars {
        return label.to_owned();
    }
    let mut out: String = label.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_labels_pass_through_untruncated() {
        assert_eq!(truncate_label("BMM 101", 12), "BMM 101");
        assert_eq!(truncate_label("Twelve chars", 12), "Twelve chars");
    }

    #[test]
    fn long_labels_cut_to_the_budget_with_an_ellipsis() {
        let cut = truncate_label("Bitaxe Gamma 601", 12);
        assert_eq!(cut, "Bitaxe Gamm\u{2026}");
        assert_eq!(cut.chars().count(), 12);
    }
}
