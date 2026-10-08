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

//! Warnings logged once per widget runtime.
//!
//! One host process runs every widget,
//! so a `static` latch would let the first misbehaving widget silence the rest.
//! Each runtime keeps its own record in `HostState`, dropped with the runtime.

use std::collections::HashSet;

/// Call-site and key pairs a runtime has already warned about.
#[derive(Debug, Default)]
pub(crate) struct LoggedSites(HashSet<(&'static str, &'static str)>);

impl LoggedSites {
    /// Record `key` at `site`, returning whether the pair is new.
    pub(crate) fn first(&mut self, site: &'static str, key: &'static str) -> bool {
        self.0.insert((site, key))
    }
}

/// `tracing::warn!` the first time a runtime reaches this call site,
/// `tracing::debug!` after that.
///
/// A helper shared by several callers adds a `key: …;`,
/// so each caller still gets its one warning from the shared call site:
///
/// ```ignore
/// warn_once!(state.logged_sites, "timer refused");
/// warn_once!(state.logged_sites, key: import_name; "`{import_name}` ignored");
/// ```
macro_rules! warn_once {
    ($sites:expr, key: $key:expr; $($arg:tt)+) => {
        if $sites.first(concat!(file!(), ":", line!(), ":", column!()), $key) {
            ::tracing::warn!($($arg)+)
        } else {
            ::tracing::debug!($($arg)+)
        }
    };
    ($sites:expr, $($arg:tt)+) => {
        $crate::log_once::warn_once!($sites, key: ""; $($arg)+)
    };
}
pub(crate) use warn_once;

#[cfg(test)]
mod tests {
    use super::LoggedSites;

    #[test]
    fn a_key_is_first_only_once() {
        let mut sites = LoggedSites::default();

        assert!(sites.first("site", "host_request_frame"));
        assert!(!sites.first("site", "host_request_frame"));
    }

    #[test]
    fn runtimes_keep_separate_records() {
        let mut first_widget = LoggedSites::default();
        let mut second_widget = LoggedSites::default();

        assert!(first_widget.first("site", "host_request_frame"));
        assert!(second_widget.first("site", "host_request_frame"));
    }

    #[test]
    fn each_call_site_is_its_own_key() {
        let mut sites = LoggedSites::default();

        for _ in 0..3 {
            warn_once!(sites, "first site");
            warn_once!(sites, "second site");
        }

        assert_eq!(sites.0.len(), 2);
    }

    #[test]
    fn a_shared_key_does_not_cross_call_sites() {
        let mut sites = LoggedSites::default();

        for _ in 0..3 {
            warn_once!(sites, key: "host_request_frame"; "first guard");
            warn_once!(sites, key: "host_request_frame"; "second guard");
        }

        assert_eq!(sites.0.len(), 2);
    }
}
