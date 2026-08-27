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

use std::path::Path;
use tracing::warn;

/// Exclusion and credential-censoring policy for one file family.
///
/// A filter can keep files out of the archive entirely
/// ([`excludes`](Self::excludes)) or censor the content of files it
/// [`matches`](Self::matches) before they are archived.
pub trait SupportFilter: Sync {
    /// Keep `path` out of the archive entirely (e.g. plaintext secret stores).
    fn excludes(&self, _path: &Path) -> bool {
        false
    }

    /// Whether [`apply`](Self::apply) censors this path's content.
    fn matches(&self, _path: &Path) -> bool {
        false
    }

    /// Censor the content of a matched file.
    fn apply(&self, content: &[u8]) -> Vec<u8> {
        content.to_vec()
    }
}

/// Whether any filter keeps `path` out of the archive entirely.
pub(crate) fn is_excluded(filters: &[&dyn SupportFilter], path: &Path) -> bool {
    filters.iter().any(|filter| filter.excludes(path))
}

/// Whether any filter censors `path`'s content. A match means the file must
/// be buffered for censoring instead of streamed straight into the archive.
pub(crate) fn is_censored(filters: &[&dyn SupportFilter], path: &Path) -> bool {
    filters.iter().any(|filter| filter.matches(path))
}

/// Censor `content` with every filter matching `path`, in list order — each
/// filter sees the previous one's output. A file claimed by two censors is
/// filtered by both, so adding an overlapping censor cannot silently leak
/// what an earlier one leaves behind.
pub fn censor(filters: &[&dyn SupportFilter], path: &Path, content: Vec<u8>) -> Vec<u8> {
    filters
        .iter()
        .filter(|filter| filter.matches(path))
        .fold(content, |content, filter| apply_one(*filter, path, content))
}

/// Apply one filter, keeping the pre-filter content on panic so a broken
/// filter never prevents archive creation.
fn apply_one(filter: &dyn SupportFilter, path: &Path, content: Vec<u8>) -> Vec<u8> {
    // NOTE: AssertUnwindSafe is sound here: the filter sees only &self and
    // &[u8], and on panic its result is discarded in favor of the original
    // content, so no partially-updated state is ever observed.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| filter.apply(&content)))
        .unwrap_or_else(|_| {
            warn!(
                path = %path.display(),
                "credential filter panicked, including unfiltered content"
            );
            content
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn engine_censors_only_matching_paths() {
        struct SuffixCensor;
        impl SupportFilter for SuffixCensor {
            fn matches(&self, path: &Path) -> bool {
                path.extension().is_some_and(|ext| ext == "secret")
            }
            fn apply(&self, _content: &[u8]) -> Vec<u8> {
                b"censored".to_vec()
            }
        }
        let filters: &[&dyn SupportFilter] = &[&SuffixCensor];

        assert!(is_censored(filters, Path::new("/tmp/file.secret")));
        assert_eq!(
            censor(filters, Path::new("/tmp/file.secret"), b"x".to_vec()),
            b"censored"
        );
        // A non-matching path is left untouched by both the predicate and the apply.
        assert!(!is_censored(filters, Path::new("/tmp/file.txt")));
        assert_eq!(
            censor(filters, Path::new("/tmp/file.txt"), b"x".to_vec()),
            b"x"
        );
        assert!(!is_excluded(filters, Path::new("/tmp/file.secret")));
    }

    #[test]
    fn engine_applies_every_matching_censor_in_order() {
        struct Append(u8);
        impl SupportFilter for Append {
            fn matches(&self, _path: &Path) -> bool {
                true
            }
            fn apply(&self, content: &[u8]) -> Vec<u8> {
                let mut out = content.to_vec();
                out.push(self.0);
                out
            }
        }
        let first = Append(b'A');
        let second = Append(b'B');
        let filters: &[&dyn SupportFilter] = &[&first, &second];

        // Both match; each sees the previous output, so list order is observable.
        assert_eq!(censor(filters, Path::new("/tmp/f"), b"x".to_vec()), b"xAB");
    }

    #[test]
    fn engine_keeps_going_when_a_censor_panics() {
        struct PanickingFilter;
        impl SupportFilter for PanickingFilter {
            fn matches(&self, _path: &Path) -> bool {
                true
            }
            fn apply(&self, _content: &[u8]) -> Vec<u8> {
                panic!("boom")
            }
        }
        struct AppendZ;
        impl SupportFilter for AppendZ {
            fn matches(&self, _path: &Path) -> bool {
                true
            }
            fn apply(&self, content: &[u8]) -> Vec<u8> {
                let mut out = content.to_vec();
                out.push(b'Z');
                out
            }
        }
        let panicking = PanickingFilter;
        let append = AppendZ;
        let filters: &[&dyn SupportFilter] = &[&panicking, &append];

        // The panicking censor yields its input unchanged; the next still runs.
        let result = censor(filters, Path::new("/tmp/x"), b"original".to_vec());
        assert_eq!(result, b"originalZ");
    }
}
