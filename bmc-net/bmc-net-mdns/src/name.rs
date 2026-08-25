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

//! Construction of the effective mDNS hostname.
//!
//! The effective name is `<hostname>`, or `<hostname>-<suffix>` when the caller
//! supplies a suffix — a stable device-unique identifier (a short HWID). The
//! caller passes one only after the link reports the plain hostname is taken,
//! so devices sharing the fleet-wide factory default end up distinct while a
//! uniquely named device keeps the name its owner configured.
//!
//! Per RFC 6762 §3, the result is always a single DNS label under `.local.`.
//! Hostnames containing only LDH characters (ASCII letters, digits, hyphens)
//! are used verbatim with case preserved. Hostnames containing dots or other
//! non-LDH characters are slugified to ASCII alphanumerics and hyphens, with
//! runs of separators collapsed to a single `-`. When the result would exceed
//! the 63-byte label limit the name is truncated, but the suffix is never
//! sacrificed.

/// Maximum length of a single DNS label, per RFC 1035 (and RFC 6762 §16).
pub const MAX_LABEL_BYTES: usize = 63;

/// Name to advertise when the configured hostname slugifies to nothing.
const FALLBACK_STEM: &str = "braiins";

/// Build the effective mDNS hostname (also used as the service instance name)
/// from the configured hostname and the device-unique suffix.
///
/// Degenerate inputs still yield a syntactically valid label: an empty
/// hostname falls back to a fixed stem and an empty suffix produces a plain
/// name instead of one with a dangling dash.
#[must_use]
pub fn effective_hostname(hostname: &str, suffix: &str) -> String {
    let hostname = if hostname.is_empty() {
        FALLBACK_STEM
    } else {
        hostname
    };
    if suffix.is_empty() {
        return shrink_to_label(hostname);
    }
    // The suffix is caller-supplied and is appended verbatim below, so a stray
    // dot or separator in it would silently produce a multi-label name.
    let suffix = &slugify(suffix);

    let verbatim = format!("{hostname}-{suffix}");
    if is_valid_label(&verbatim) {
        return verbatim;
    }

    let slug = slugify(hostname);
    let slugified = format!("{slug}-{suffix}");
    if slugified.len() <= MAX_LABEL_BYTES {
        return slugified;
    }

    let keep = MAX_LABEL_BYTES.saturating_sub(suffix.len() + 1);
    let stem: String = slug.chars().take(keep).collect();
    let stem = stem.trim_end_matches('-');
    if stem.is_empty() {
        return shrink_to_label(suffix);
    }
    format!("{stem}-{suffix}")
}

fn is_valid_label(label: &str) -> bool {
    if label.is_empty() || label.len() > MAX_LABEL_BYTES {
        return false;
    }
    if label.starts_with('-') || label.ends_with('-') {
        return false;
    }
    label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn shrink_to_label(name: &str) -> String {
    if is_valid_label(name) {
        return name.to_owned();
    }
    // `slugify` yields non-empty ASCII LDH text, so the only remaining way to
    // violate the label rule is length, and clamping cannot expose a leading
    // dash. The slug is pure ASCII, so taking chars == taking bytes.
    let slug = slugify(name);
    if slug.len() <= MAX_LABEL_BYTES {
        return slug;
    }
    let clamped: String = slug.chars().take(MAX_LABEL_BYTES).collect();
    clamped.trim_end_matches('-').to_owned()
}

fn slugify(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        FALLBACK_STEM.to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUFFIX: &str = "3f9c2a";

    #[test]
    fn short_hostname_is_used_verbatim() {
        assert_eq!(effective_hostname("Antminer", SUFFIX), "Antminer-3f9c2a");
        assert_eq!(effective_hostname("miner-a", SUFFIX), "miner-a-3f9c2a");
    }

    #[test]
    fn hostname_at_limit_is_kept() {
        let hostname = "a".repeat(MAX_LABEL_BYTES - SUFFIX.len() - 1);
        let name = effective_hostname(&hostname, SUFFIX);
        assert_eq!(name.len(), MAX_LABEL_BYTES);
        assert_eq!(name, format!("{hostname}-{SUFFIX}"));
    }

    #[test]
    fn overlong_hostname_is_slugified_before_trimming() {
        // Multibyte characters inflate the byte length; slugification alone
        // brings the name back under the limit.
        let hostname = format!("{}-Rack-7", "🦀".repeat(20));
        assert_eq!(effective_hostname(&hostname, SUFFIX), "rack-7-3f9c2a");
    }

    #[test]
    fn overlong_slug_is_trimmed_to_fit_with_suffix() {
        let hostname = "b".repeat(200);
        let name = effective_hostname(&hostname, SUFFIX);
        assert_eq!(name.len(), MAX_LABEL_BYTES);
        assert!(name.ends_with("-3f9c2a"));
        assert!(name.starts_with("bbb"));
    }

    #[test]
    fn trimming_does_not_leave_a_dangling_dash() {
        let keep = MAX_LABEL_BYTES - SUFFIX.len() - 1;
        let hostname = format!("{}--{}", "c".repeat(keep - 1), "d".repeat(100));
        let name = effective_hostname(&hostname, SUFFIX);
        assert!(!name.contains("--"), "unexpected dash run in {name}");
        assert!(name.len() <= MAX_LABEL_BYTES);
    }

    #[test]
    fn unicode_only_hostname_falls_back_to_stem() {
        let hostname = "🦀".repeat(30);
        assert_eq!(effective_hostname(&hostname, SUFFIX), "braiins-3f9c2a");
    }

    #[test]
    fn degenerate_inputs_yield_valid_labels() {
        assert_eq!(effective_hostname("", SUFFIX), "braiins-3f9c2a");
        assert_eq!(effective_hostname("miner-a", ""), "miner-a");
        assert_eq!(effective_hostname("", ""), "braiins");
    }

    #[test]
    fn multibyte_suffix_is_clamped_to_label_bytes() {
        let suffix = "é".repeat(60);
        let name = effective_hostname("miner-a", &suffix);
        assert!(name.len() <= MAX_LABEL_BYTES, "{} bytes", name.len());
        assert!(!name.ends_with('-'));
    }

    #[test]
    fn slugify_collapses_separator_runs() {
        assert_eq!(slugify("My  Shiny__Miner!!"), "my-shiny-miner");
        assert_eq!(slugify("---"), "braiins");
        assert_eq!(slugify(""), "braiins");
    }

    #[test]
    fn dotted_hostname_is_slugified() {
        assert_eq!(
            effective_hostname("dot.separated", SUFFIX),
            "dot-separated-3f9c2a"
        );
        assert_eq!(effective_hostname("a.b.c", SUFFIX), "a-b-c-3f9c2a");
    }

    #[test]
    fn non_ldh_characters_are_slugified() {
        assert_eq!(effective_hostname("my@miner", SUFFIX), "my-miner-3f9c2a");
        assert_eq!(effective_hostname("test_host", SUFFIX), "test-host-3f9c2a");
        assert_eq!(effective_hostname("rack#7", SUFFIX), "rack-7-3f9c2a");
    }

    #[test]
    fn ldh_hostname_is_preserved_verbatim() {
        assert_eq!(effective_hostname("MyMiner", SUFFIX), "MyMiner-3f9c2a");
        assert_eq!(effective_hostname("UPPERCASE", SUFFIX), "UPPERCASE-3f9c2a");
        assert_eq!(effective_hostname("CamelCase", SUFFIX), "CamelCase-3f9c2a");
    }

    #[test]
    fn malformed_suffix_is_normalised() {
        assert_eq!(effective_hostname("miner", "ab.cd.ef"), "miner-ab-cd-ef");
        assert_eq!(effective_hostname("miner", "abc123-"), "miner-abc123");
        assert_eq!(effective_hostname("miner", "AB_CD"), "miner-ab-cd");
        assert_eq!(effective_hostname("miner", "-"), "miner-braiins");
        assert_eq!(effective_hostname("🦀🦀🦀", "-test-"), "braiins-test");
    }

    /// The advertised name is used as a DNS label with no further validation,
    /// so no input may produce one that breaks the LDH or length rules.
    #[test]
    fn output_is_always_a_valid_label() {
        let hostnames = [
            String::new(),
            "-".to_owned(),
            "---".to_owned(),
            "a".to_owned(),
            "...".to_owned(),
            "🦀🦀🦀".to_owned(),
            "dot.separated".to_owned(),
            "UPPER".to_owned(),
            "\0\n\t".to_owned(),
            "x".repeat(200),
        ];
        let suffixes = [
            String::new(),
            "-".to_owned(),
            "abc".to_owned(),
            "ab.cd".to_owned(),
            "AB_CD".to_owned(),
            "abc-".to_owned(),
            "z".repeat(80),
        ];
        for hostname in &hostnames {
            for suffix in &suffixes {
                let label = effective_hostname(hostname, suffix);
                assert!(
                    is_valid_label(&label),
                    "invalid label {label:?} from ({hostname:?}, {suffix:?})"
                );
            }
        }
    }

    #[test]
    fn empty_suffix_produces_single_label() {
        assert_eq!(effective_hostname("normal", ""), "normal");
        assert_eq!(effective_hostname("dot.separated", ""), "dot-separated");
        assert_eq!(effective_hostname("with@special", ""), "with-special");
    }
}
