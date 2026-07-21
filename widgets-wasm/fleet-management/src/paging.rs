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

/// The page actually shown: the stored page clamped into the page count, so
/// a fleet shrinking under the operator can never strand the view.
#[must_use]
pub fn effective_page(page: usize, count: usize) -> usize {
    page.min(count.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_page_clamps_to_the_last_page() {
        assert_eq!(effective_page(0, 3), 0);
        assert_eq!(effective_page(2, 3), 2);
        assert_eq!(effective_page(5, 3), 2);
        assert_eq!(effective_page(5, 1), 0);
    }
}
