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

//! When a test that needs headless GL may skip.

/// Set by the Nix `ci` profile, which supplies Mesa:
/// a failed GL init there is a broken profile, and skipping would hide it.
pub const REQUIRE_EGL: &str = "BMC_REQUIRE_HEADLESS_EGL";

/// What a headless GL `init` returned, or `None` to skip the test when it failed
/// outside the `ci` profile.
///
/// # Panics
///
/// Panics when `init` failed where [`REQUIRE_EGL`] demands GL.
pub fn or_skip<T>(init: anyhow::Result<T>) -> Option<T> {
    match init {
        Ok(gl) => Some(gl),
        Err(error) => {
            assert!(
                std::env::var_os(REQUIRE_EGL).is_none(),
                "{REQUIRE_EGL} is set, so skipping would report tests that never ran: {error:#}"
            );
            eprintln!(
                "skipping: headless GL is unavailable; the `ci` dev shell supplies Mesa: {error:#}"
            );
            None
        }
    }
}
