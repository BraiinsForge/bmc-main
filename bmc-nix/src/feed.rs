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

//! Package feed (`nix-package-feed.v1.json`): release artifacts keyed by
//! full BOS versions or shared release names. Each entry provides an init
//! tarball and an optional package index. Fetching and JSON parsing stay
//! with the callers.

use serde::{Deserialize, Serialize};

pub const PACKAGE_FEED_VERSION: u32 = 1;

/// Package feed document (`nix-package-feed.v1.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageFeed {
    pub version: u32,
    pub entries: Vec<PackageFeedEntry>,
}

/// Release artifacts selected by a full BOS version or shared release key.
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

// The release tail matches published keys verbatim; validating calendar dates
// or numeric ranges in the discarded prefix would not improve that lookup.
fn shared_release_name(bos_version: &str) -> Option<&str> {
    let mut parts = bos_version.splitn(6, '-');
    for width in [4, 2, 2] {
        let part = parts.next()?;
        if part.len() != width || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    let day_index = parts.next()?;
    if day_index.is_empty() || !day_index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let commit = parts.next()?;
    if commit.len() != 8 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    parts.next().filter(|release| !release.is_empty())
}

/// Prefer the exact BOS version, then its shared release name.
/// Callers run [`validate_feed`] first. An exact entry stays authoritative
/// even when its artifacts cannot satisfy the operation.
///
/// # Errors
///
/// Returns [`FeedError::MissingEntry`] when no entry matches.
pub fn select_entry<'a>(
    url: &str,
    feed: &'a PackageFeed,
    bos_version: &str,
) -> Result<&'a PackageFeedEntry, FeedError> {
    let entry = feed
        .entries
        .iter()
        .find(|entry| entry.bos_version == bos_version)
        .or_else(|| {
            let release = shared_release_name(bos_version)?;
            let entry = feed
                .entries
                .iter()
                .find(|entry| entry.bos_version == release);
            if entry.is_none() {
                tracing::info!(
                    feed_url = url,
                    firmware = bos_version,
                    shared_bos_version = release,
                    "No matching package feed entry"
                );
            }
            entry
        })
        .ok_or_else(|| FeedError::MissingEntry {
            url: url.to_owned(),
            bos_version: bos_version.to_owned(),
        })?;
    tracing::info!(
        feed_url = url,
        firmware = bos_version,
        selected_bos_version = entry.bos_version,
        "Selected package feed entry"
    );
    Ok(entry)
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
    fn select_preserves_release_variant_patch_and_suffix() {
        for release in [
            "26.09",
            "26.09-plus",
            "26.09-rc",
            "26.09-plus-rc",
            "26.09-plus-nightly",
            "26.09.1-plus-nightly",
            "26.09-plus-a",
            "26.09-plus-custom",
        ] {
            let target = format!("2026-09-07-0-abcdef12-{release}");
            let feed = PackageFeed {
                version: PACKAGE_FEED_VERSION,
                entries: vec![entry(release, Some("https://example.com/shared.json"))],
            };
            validate_feed("u", &feed).expect("BUG: shared keys are valid v1 entries");
            assert_eq!(
                select_entry("u", &feed, &target)
                    .expect("a rebuild must find the published shared entry")
                    .bos_version,
                release
            );

            for other in [
                "26.08-plus-nightly",
                "26.09",
                "26.09-plus",
                "26.09-rc",
                "26.09-plus-rc",
                "26.09-plus-nightly",
                "26.09.1-plus-nightly",
                "26.09-plus-a",
                "26.09-plus-custom",
            ] {
                if other != release {
                    let other_target = format!("2026-09-07-0-abcdef12-{other}");
                    assert!(
                        matches!(
                            select_entry("u", &feed, &other_target),
                            Err(FeedError::MissingEntry { .. })
                        ),
                        "{other_target} must not select {release}"
                    );
                }
            }
        }
    }

    #[test]
    fn exact_entry_wins_whole_regardless_of_feed_order() {
        let target = "2026-09-07-0-abcdef12-26.09-plus-nightly";
        let mut feed = PackageFeed {
            version: PACKAGE_FEED_VERSION,
            entries: vec![
                entry(
                    "26.09-plus-nightly",
                    Some("https://example.com/shared.json"),
                ),
                entry(target, None),
            ],
        };
        for _ in 0..2 {
            validate_feed("u", &feed).expect("BUG: exact and shared entries can coexist");
            let selected = select_entry("u", &feed, target).expect("the exact entry must win");
            assert_eq!(selected.bos_version, target);
            assert!(
                matches!(
                    require_index_url("u", selected),
                    Err(FeedError::MissingIndexUrl { .. })
                ),
                "an incomplete exact entry must not borrow the shared index"
            );
            feed.entries.reverse();
        }
    }

    #[test]
    fn release_extraction_preserves_the_tail_without_normalization() {
        for release in [
            "26.09",
            "26.09-plus",
            "26.09-rc",
            "26.09-plus-nightly",
            "26.09.1-plus-rc",
            "26.09-plus-custom",
            "26.9",
            "26.09.01",
            "custom-release-name",
        ] {
            let firmware = format!("2026-09-07-0-abcdef12-{release}");
            assert_eq!(shared_release_name(&firmware), Some(release), "{firmware}");
        }
    }

    #[test]
    fn release_extraction_does_not_validate_calendar_or_day_index_ranges() {
        for prefix in [
            "2026-02-30-0-abcdef12",
            "2026-09-07-256-abcdef12",
            "2026-09-07-999999999999999999999999-ABCDEF12",
        ] {
            let firmware = format!("{prefix}-26.09-plus-custom");
            assert_eq!(
                shared_release_name(&firmware),
                Some("26.09-plus-custom"),
                "{firmware}"
            );
        }
    }

    #[test]
    fn release_extraction_requires_a_complete_bos_prefix_and_tail() {
        for firmware in [
            "26.09-plus-nightly",
            "2026-09-07-0-abcdef12",
            "2026-09-07-0-abcdef12-",
            "2026-09-07--abcdef12-26.09",
            "2026-09-07-x-abcdef12-26.09",
            "2026-09-07-0-abcdef1-26.09",
            "2026-09-07-0-abcdef123-26.09",
            "2026-09-07-０-abcdef12-26.09",
            "2026-09-07-0-abcdefg1-26.09",
            "2026-9-07-0-abcdef12-26.09",
            "2026-09-7-0-abcdef12-26.09",
            "026-09-07-0-abcdef12-26.09",
            "２０２６-09-07-0-abcdef12-26.09",
        ] {
            assert_eq!(shared_release_name(firmware), None, "{firmware}");
        }
    }

    #[test]
    fn malformed_targets_do_not_guess_shared_entries() {
        let feed = PackageFeed {
            version: PACKAGE_FEED_VERSION,
            entries: vec![entry("26.09-plus-nightly", None)],
        };
        for target in [
            "garbage-26.09-plus-nightly",
            "2026-xx-30-0-abcdef12-26.09-plus-nightly",
            "2026-09-07-0-notahash-26.09-plus-nightly",
            "2026-09-07-0-abcdef12-26.09-plus-unknown",
        ] {
            assert!(
                matches!(
                    select_entry("u", &feed, target),
                    Err(FeedError::MissingEntry { .. })
                ),
                "{target} must not acquire a guessed fallback"
            );
        }
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
