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
