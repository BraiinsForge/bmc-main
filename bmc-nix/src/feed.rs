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

//! Package feed (`nix-package-feed.v1.json`): the per-firmware release
//! catalog. Each entry names a BOS version, the init tarball that
//! bootstraps it, and optionally the package index that serves it.
//! Store init consumes the tarball fields; upgrade resolution follows
//! `index_url`. Pure validation and selection only — fetching and JSON
//! parsing stay with the callers.

use serde::{Deserialize, Serialize};

pub const PACKAGE_FEED_VERSION: u32 = 1;

/// Package feed document (`nix-package-feed.v1.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFeed {
    pub version: u32,
    pub entries: Vec<PackageFeedEntry>,
}

/// A single per-firmware feed entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFeedEntry {
    pub bos_version: String,
    pub download_url: String,
    pub profile_path: String,
    /// Exact URL of this firmware's package index. Absent entries are
    /// valid for store init but cannot serve upgrade resolution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_url: Option<String>,
    /// Nix-style `name:base64` Ed25519 signature of the init tarball
    /// (see [`crate::signature`]). Only verification-enabled init
    /// consumes it — and hard-fails when it is absent; upgrade
    /// resolution and unsigned development feeds parse without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Failure to validate a package feed or select an entry from it.
#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    #[error(
        "unsupported package feed version {version} at {url} (expected {PACKAGE_FEED_VERSION})"
    )]
    UnsupportedVersion { url: String, version: u32 },
    #[error("duplicate package feed entries for BOS version '{bos_version}' at {url}")]
    DuplicateEntry { url: String, bos_version: String },
    #[error("no package feed entry for BOS version '{bos_version}' at {url}")]
    MissingEntry { url: String, bos_version: String },
    #[error("package feed entry for BOS version '{bos_version}' at {url} has no index_url")]
    MissingIndexUrl { url: String, bos_version: String },
}

/// Validate a parsed feed: supported version, no duplicate
/// `bos_version` entries (a duplicate would make publication order
/// semantically significant). `url` is diagnostic context only.
///
/// # Errors
///
/// Returns [`FeedError`] on an unsupported version or duplicate entry.
pub fn validate_feed(url: &str, feed: &PackageFeed) -> Result<(), FeedError> {
    if feed.version != PACKAGE_FEED_VERSION {
        return Err(FeedError::UnsupportedVersion {
            url: url.to_owned(),
            version: feed.version,
        });
    }
    let mut seen = std::collections::HashSet::new();
    for entry in &feed.entries {
        if !seen.insert(entry.bos_version.as_str()) {
            return Err(FeedError::DuplicateEntry {
                url: url.to_owned(),
                bos_version: entry.bos_version.clone(),
            });
        }
    }
    Ok(())
}

/// Select the entry with `bos_version == target`. Callers run
/// [`validate_feed`] first.
///
/// # Errors
///
/// Returns [`FeedError::MissingEntry`] when no entry matches.
pub fn select_entry<'a>(
    url: &str,
    feed: &'a PackageFeed,
    bos_version: &str,
) -> Result<&'a PackageFeedEntry, FeedError> {
    feed.entries
        .iter()
        .find(|entry| entry.bos_version == bos_version)
        .ok_or_else(|| FeedError::MissingEntry {
            url: url.to_owned(),
            bos_version: bos_version.to_owned(),
        })
}

/// Require the selected entry's `index_url` — upgrade resolution only;
/// store init tolerates its absence.
///
/// # Errors
///
/// Returns [`FeedError::MissingIndexUrl`] when the entry has none.
pub fn require_index_url<'a>(url: &str, entry: &'a PackageFeedEntry) -> Result<&'a str, FeedError> {
    entry
        .index_url
        .as_deref()
        .ok_or_else(|| FeedError::MissingIndexUrl {
            url: url.to_owned(),
            bos_version: entry.bos_version.clone(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(v: &str, index_url: Option<&str>) -> PackageFeedEntry {
        PackageFeedEntry {
            bos_version: v.to_owned(),
            download_url: format!("https://example.com/{v}.tar.gz"),
            profile_path: "/nix/var/nix/gcroots/profiles/bmc".to_owned(),
            index_url: index_url.map(str::to_owned),
            signature: None,
        }
    }

    #[test]
    fn validate_accepts_current_version_and_unique_entries() {
        let feed = PackageFeed {
            version: PACKAGE_FEED_VERSION,
            entries: vec![entry("a", None), entry("b", None)],
        };
        assert!(validate_feed("u", &feed).is_ok());
    }

    #[test]
    fn validate_rejects_unsupported_version() {
        let feed = PackageFeed {
            version: 99,
            entries: vec![],
        };
        assert!(matches!(
            validate_feed("u", &feed),
            Err(FeedError::UnsupportedVersion { version: 99, .. })
        ));
    }

    #[test]
    fn validate_rejects_duplicate_bos_version() {
        let feed = PackageFeed {
            version: PACKAGE_FEED_VERSION,
            entries: vec![entry("a", None), entry("a", None)],
        };
        assert!(matches!(
            validate_feed("u", &feed),
            Err(FeedError::DuplicateEntry { .. })
        ));
    }

    #[test]
    fn select_finds_exact_target_and_rejects_missing() {
        let feed = PackageFeed {
            version: PACKAGE_FEED_VERSION,
            entries: vec![entry("a", None)],
        };
        assert_eq!(
            select_entry("u", &feed, "a")
                .expect("BUG: entry exists")
                .bos_version,
            "a"
        );
        assert!(matches!(
            select_entry("u", &feed, "x"),
            Err(FeedError::MissingEntry { .. })
        ));
    }

    #[test]
    fn require_index_url_distinguishes_present_and_absent() {
        assert_eq!(
            require_index_url("u", &entry("a", Some("https://i"))).expect("BUG: present"),
            "https://i"
        );
        assert!(matches!(
            require_index_url("u", &entry("a", None)),
            Err(FeedError::MissingIndexUrl { .. })
        ));
    }

    #[test]
    fn entry_round_trips_with_and_without_index_url() {
        let with: PackageFeedEntry = serde_json::from_str(
            r#"{"bos_version":"a","download_url":"d","profile_path":"p","index_url":"i"}"#,
        )
        .expect("BUG: valid JSON");
        assert_eq!(with.index_url.as_deref(), Some("i"));
        let without: PackageFeedEntry =
            serde_json::from_str(r#"{"bos_version":"a","download_url":"d","profile_path":"p"}"#)
                .expect("BUG: valid JSON");
        assert!(without.index_url.is_none());
        assert!(
            !serde_json::to_string(&without)
                .expect("BUG: serializable")
                .contains("index_url")
        );
    }

    #[test]
    fn entry_round_trips_with_and_without_signature() {
        let with: PackageFeedEntry = serde_json::from_str(
            r#"{"bos_version":"a","download_url":"d","profile_path":"p","signature":"k:c2ln"}"#,
        )
        .expect("BUG: valid JSON");
        assert_eq!(with.signature.as_deref(), Some("k:c2ln"));
        let without: PackageFeedEntry =
            serde_json::from_str(r#"{"bos_version":"a","download_url":"d","profile_path":"p"}"#)
                .expect("BUG: valid JSON");
        assert!(without.signature.is_none());
        assert!(
            !serde_json::to_string(&without)
                .expect("BUG: serializable")
                .contains("signature")
        );
    }

    #[test]
    fn feed_accepts_v1_document() {
        let feed: PackageFeed = serde_json::from_str(
            r#"{
                "version": 1,
                "entries": [{
                    "bos_version": "1.0.0",
                    "download_url": "https://example.com/tarball.tar.gz",
                    "profile_path": "/nix/var/nix/gcroots/profiles/bmc"
                }]
            }"#,
        )
        .expect("BUG: test JSON should be valid");
        assert!(validate_feed("u", &feed).is_ok());
        assert_eq!(feed.entries.len(), 1);
        assert_eq!(feed.entries[0].bos_version, "1.0.0");
    }
}
