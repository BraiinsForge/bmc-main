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

//! The nexus picture-of-the-day feeds: URL shapes, the metadata the widget
//! keeps, and how that metadata is persisted alongside the cached picture.
//!
//! Everything here is pure, so it builds and tests on the host.

use crate::manifest_params::Source;

/// `{latest|YYYY-MM-DD}_{full|WWWxHHH}.{ext}` under this prefix.
const NEXUS_APOD: &str = "https://nexus.braiinsforge.com/api/v1/data/nasa/apod";

/// Where a source's pictures live.
///
/// Exhaustive on purpose: a second feed must name its endpoint here
/// before the widget will build, rather than quietly serving this one's.
const fn prefix(source: Source) -> &'static str {
    match source {
        Source::NasaApod => NEXUS_APOD,
    }
}

/// Field separator inside the cache identity and the persisted metadata.
/// ASCII unit separator. No field can hold one: [`is_published_date`] rejects
/// a date carrying it, [`one_line`] strips it from the title and the credit,
/// and [`Meta::decode`] rejects a record that has one anyway.
const SEP: char = '\u{1f}';

/// Whether `date` is the `YYYY-MM-DD` shape nexus publishes.
///
/// A gate on a path segment and a cache field, not a date parse:
/// the widget never interprets the value, it only forwards it.
/// Anything else could carry a `/` that builds a path we did not intend,
/// a `?` that truncates one, or a separator that mis-splits the metadata.
#[must_use]
pub fn is_published_date(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| bytes[i].is_ascii_digit())
}

/// Metadata URL. Always `latest`:
/// the device clock may be unset, and the answer names the date authoritatively.
/// The size variant does not affect the metadata, so ask for the canonical one.
#[must_use]
pub fn metadata_url(source: Source) -> String {
    let mut url = String::from(prefix(source));
    url.push_str("/latest_full.json");
    url
}

/// Picture URL for a published date at the widget's pixel size.
/// It doubles as the cache identity: source, date and size all ride inside it,
/// and nothing else distinguishes one cached blob from another.
///
/// Dated rather than `latest`: it names the exact picture the metadata described,
/// so a publication crossing the two fetches cannot file it under the old date.
/// Always JPEG — nexus re-encodes to the size we ask for,
/// and the host's pixel budget gives JPEG 64× the headroom of PNG.
#[must_use]
pub fn picture_url(source: Source, date: &str, width: u32, height: u32) -> String {
    let mut url = String::from(prefix(source));
    url.push('/');
    url.push_str(date);
    url.push('_');
    url.push_str(&width.to_string());
    url.push('x');
    url.push_str(&height.to_string());
    url.push_str(".jpg");
    url
}

/// Whether `url` was built from `source`'s endpoint.
///
/// The identity carries the whole request URL, so its prefix is the only
/// record of which feed served a cached or decoded picture.
#[must_use]
pub fn is_from(source: Source, url: &str) -> bool {
    url.starts_with(prefix(source))
}

/// What the feed said about the current picture.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Meta {
    pub date: String,
    pub title: String,
    pub credit: String,
}

impl Meta {
    /// An empty metadata — nothing fetched yet.
    /// `const` so it can seed a `thread_local!`.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub const fn new() -> Self {
        Self {
            date: String::new(),
            title: String::new(),
            credit: String::new(),
        }
    }

    /// Whether the picture on flash, identified by `stored`,
    /// is the one this metadata describes.
    ///
    /// The two are separate cache entries and can disagree —
    /// a crash between the writes, or a decode that failed after the metadata landed.
    /// Drawing a photographer's credit over someone else's photograph
    /// is the failure worth ruling out, so the caption waits on this.
    #[must_use]
    pub fn describes(&self, source: Source, stored: &str, width: u32, height: u32) -> bool {
        is_published_date(&self.date) && picture_url(source, &self.date, width, height) == stored
    }

    /// Pack for the flash cache as `date SEP title SEP credit`.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut out = self.date.clone();
        out.push(SEP);
        out.push_str(&self.title);
        out.push(SEP);
        out.push_str(&self.credit);
        out
    }

    /// Unpack what [`encode`](Self::encode) wrote. `None` if it is not that.
    #[must_use]
    pub fn decode(packed: &str) -> Option<Self> {
        let mut fields = packed.split(SEP);
        let date = fields.next()?;
        let title = fields.next()?;
        let credit = fields.next()?;
        // A fourth field means a separator reached one of the first three.
        // Nothing says where the title ended, so fail to a cache miss
        // rather than caption the picture with half of its own title.
        if fields.next().is_some() {
            return None;
        }
        // A record on flash is untrusted input too, and its date reaches
        // a URL exactly like one the feed just named.
        if !is_published_date(date) {
            return None;
        }
        Some(Self {
            date: date.to_owned(),
            title: title.to_owned(),
            credit: credit.to_owned(),
        })
    }
}

/// Flatten a feed string to one printable line.
///
/// Upstream credits carry hard line breaks and run to several names with their
/// affiliations — `"First Author\n Text: \nSecond Author \n(MOCK\nORG)"`.
/// Rendered as-is that is a multi-line block in the corner of the picture,
/// so collapse every whitespace run to a single space.
///
/// Every other control character goes too, since none of them draw anything.
/// [`SEP`] is one of those: a title carrying it splits in two off flash,
/// with its tail rendering as the credit.
/// Whitespace survives the filter and is collapsed below,
/// or a `\n` between two names would run them together.
#[must_use]
pub fn one_line(raw: &str) -> String {
    let printable: String = raw
        .chars()
        .filter(|c| c.is_whitespace() || !c.is_control())
        .collect();
    let mut out = String::with_capacity(printable.len());
    for word in printable.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NASA: Source = Source::NasaApod;

    #[test]
    fn is_from_reads_the_feed_out_of_an_identity() {
        assert!(is_from(NASA, &picture_url(NASA, "2026-08-27", 638, 480)));
        assert!(is_from(NASA, &metadata_url(NASA)));
        assert!(!is_from(
            NASA,
            "https://nexus.braiinsforge.com/api/v1/data/esa/hubble/2026-08-27_638x480.jpg"
        ));
        assert!(!is_from(NASA, ""));
    }

    #[test]
    fn picture_url_carries_date_and_pixel_size() {
        assert_eq!(
            picture_url(NASA, "2026-08-27", 638, 480),
            "https://nexus.braiinsforge.com/api/v1/data/nasa/apod/2026-08-27_638x480.jpg"
        );
    }

    #[test]
    fn metadata_url_is_date_free() {
        let url = metadata_url(NASA);
        assert!(url.ends_with("/latest_full.json"), "{url}");
    }

    #[test]
    fn every_source_serves_its_pictures_and_its_metadata_from_one_prefix() {
        for source in Source::ALL {
            let base = prefix(*source);
            assert!(base.starts_with("https://"), "{base}");
            assert!(metadata_url(*source).starts_with(base));
            assert!(picture_url(*source, "2026-08-27", 638, 480).starts_with(base));
        }
    }

    #[test]
    fn the_cache_identity_changes_with_date_and_size() {
        let base = picture_url(NASA, "2026-08-27", 638, 480);
        assert_ne!(base, picture_url(NASA, "2026-08-28", 638, 480));
        assert_ne!(base, picture_url(NASA, "2026-08-27", 317, 238));
        assert_eq!(base, picture_url(NASA, "2026-08-27", 638, 480));
    }

    #[test]
    fn meta_round_trips_through_the_cache_encoding() {
        let meta = Meta {
            date: "2026-08-27".to_owned(),
            title: "Sample Nebula over a Placeholder Horizon".to_owned(),
            credit: "Fixture Author".to_owned(),
        };
        assert_eq!(Meta::decode(&meta.encode()).as_ref(), Some(&meta));
    }

    #[test]
    fn meta_decode_rejects_junk_and_empty_dates() {
        assert_eq!(Meta::decode(""), None);
        assert_eq!(Meta::decode("2026-08-27"), None);
        assert_eq!(Meta::decode("\u{1f}title\u{1f}credit"), None);
    }

    #[test]
    fn a_published_date_is_ten_characters_of_digits_and_two_dashes() {
        assert!(is_published_date("2026-08-27"));
        assert!(
            is_published_date("0000-00-00"),
            "shape only, not a real date"
        );
    }

    #[test]
    fn a_date_that_could_reshape_a_url_or_the_cache_record_is_rejected() {
        for date in [
            "",
            "2026-8-27",
            "2026-08-27 ",
            "2026/08/27",
            "../../secret",
            "2026-08-27?x=1",
            "2026-08-27#f",
            "2026-08-27\u{1f}",
            "2026-08-270",
        ] {
            assert!(!is_published_date(date), "{date:?} must not reach a URL");
        }
    }

    #[test]
    fn meta_decode_rejects_a_flash_record_whose_date_is_not_a_date() {
        // A corrupted record must not seed a fetch.
        assert_eq!(Meta::decode("../../x\u{1f}Title\u{1f}Credit"), None);
    }

    #[test]
    fn meta_decode_rejects_a_record_carrying_a_fourth_field() {
        assert_eq!(
            Meta::decode("2026-08-27\u{1f}A\u{1f}B\u{1f}C"),
            None,
            "a separator inside a field leaves no way to tell where the title ended"
        );
    }

    #[test]
    fn a_title_carrying_a_separator_round_trips_whole() {
        let meta = Meta {
            date: "2026-08-27".to_owned(),
            title: one_line("A\u{1f}B"),
            credit: one_line("C"),
        };
        assert_eq!(meta.title, "AB", "the separator never reaches the record");
        assert_eq!(
            Meta::decode(&meta.encode()).as_ref(),
            Some(&meta),
            "so the title comes back as the title and the credit as the credit"
        );
    }

    #[test]
    fn caption_is_withheld_when_the_stored_picture_is_a_different_one() {
        let meta = Meta {
            date: "2026-08-27".to_owned(),
            ..Meta::default()
        };
        let stored = picture_url(NASA, "2026-08-27", 638, 480);
        assert!(meta.describes(NASA, &stored, 638, 480));
        // Yesterday's picture still on flash, today's metadata already banked.
        let stale = picture_url(NASA, "2026-08-26", 638, 480);
        assert!(!meta.describes(NASA, &stale, 638, 480));
        // A resized viewport is a different blob too.
        assert!(!meta.describes(NASA, &stored, 317, 238));
    }

    #[test]
    fn a_dateless_meta_never_describes_anything() {
        let meta = Meta::default();
        assert!(!meta.describes(NASA, &picture_url(NASA, "", 638, 480), 638, 480));
    }

    #[test]
    fn one_line_flattens_upstream_credit_formatting() {
        assert_eq!(
            one_line("First Author\n Text: \nSecond Author \n(MOCK\nORG, \nDEPT)"),
            "First Author Text: Second Author (MOCK ORG, DEPT)"
        );
        assert_eq!(one_line("  "), "");
        assert_eq!(one_line("Solo"), "Solo");
    }

    #[test]
    fn one_line_drops_control_characters_without_welding_words_together() {
        assert_eq!(one_line("A\u{1f}B"), "AB");
        assert_eq!(one_line("A\u{7}"), "A");
        assert_eq!(
            one_line("First\nSecond"),
            "First Second",
            "a line break separates two names, so it must not be dropped"
        );
        assert_eq!(
            one_line("First \u{1f} Second"),
            "First Second",
            "a word that was nothing but controls leaves no double space"
        );
    }
}
