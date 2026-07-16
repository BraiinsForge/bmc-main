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

//! Lightweight iCal (.ics) parser — extracts VEVENTs from RFC 5545 data.
//!
//! Hand-rolled line-based parser instead of the `ical` crate to stay within
//! the WASM fuel budget. iCal is a simple line-based format:
//! - `BEGIN:VEVENT` / `END:VEVENT` delimit events
//! - Properties are `NAME:VALUE` or `NAME;PARAM=VAL:VALUE`
//! - Long lines are folded: continuation lines start with a space or tab

/// A single parsed event before RRULE expansion.
#[derive(Debug, Clone)]
pub struct RawEvent {
    pub summary: String,
    pub description: Option<String>,
    pub location: Option<String>,
    /// Raw DTSTART value (e.g. "20260310T090000" or "20260310")
    pub dtstart: String,
    /// Raw DTEND value
    pub dtend: Option<String>,
    /// TZID parameter from DTSTART (e.g. "Europe/Prague")
    pub tzid: Option<String>,
    /// Whether this is an all-day event (DATE vs DATE-TIME)
    pub all_day: bool,
    /// RRULE string if present (e.g. "FREQ=WEEKLY;BYDAY=MO,WE,FR")
    pub rrule: Option<String>,
}

/// Split raw iCal data into per-VEVENT chunks for incremental parsing.
///
/// This is a cheap byte scan — no line unfolding, no property parsing.
/// Each returned string contains everything between (exclusive)
/// `BEGIN:VEVENT` and `END:VEVENT`, ready to be passed to [`parse_chunk`].
pub fn split_into_chunks(data: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut search_from = 0;

    loop {
        // Find the next BEGIN:VEVENT (must be at line start or after \n)
        let begin = match find_line(data, search_from, "BEGIN:VEVENT") {
            Some(pos) => pos,
            None => break,
        };
        let body_start = begin + "BEGIN:VEVENT".len();

        // Find the matching END:VEVENT
        let end = match find_line(data, body_start, "END:VEVENT") {
            Some(pos) => pos,
            None => break,
        };

        chunks.push(data[body_start..end].to_string());
        search_from = end + "END:VEVENT".len();
    }

    chunks
}

/// Parse a single VEVENT chunk (the text between BEGIN:VEVENT and END:VEVENT).
pub fn parse_chunk(chunk: &str) -> Option<RawEvent> {
    let lines = unfold_lines(chunk);
    let mut builder = RawEventBuilder::default();
    for line in &lines {
        builder.parse_line(line);
    }
    builder.build()
}

/// Find `needle` at a line boundary starting from `from`.
/// Returns the byte offset of the start of `needle`, or `None`.
fn find_line(data: &str, from: usize, needle: &str) -> Option<usize> {
    let mut pos = from;
    loop {
        let idx = data[pos..].find(needle)?;
        let abs = pos + idx;
        // Must be at start of data or preceded by \n (possibly \r\n)
        if abs == 0 || data.as_bytes()[abs - 1] == b'\n' {
            return Some(abs);
        }
        pos = abs + 1;
    }
}

/// Unfold RFC 5545 line folding: continuation lines start with a space or tab.
fn unfold_lines(data: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    for raw_line in data.lines() {
        // Strip trailing \r (lines() handles \n but not always \r\n cleanly)
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation of previous line
            if let Some(prev) = result.last_mut() {
                prev.push_str(&line[1..]);
            }
        } else {
            result.push(line.to_string());
        }
    }

    result
}

/// Builder for accumulating VEVENT properties.
#[derive(Default)]
struct RawEventBuilder {
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    dtstart: Option<String>,
    dtstart_params: Option<String>,
    dtend: Option<String>,
    tzid: Option<String>,
    all_day: bool,
    rrule: Option<String>,
}

impl RawEventBuilder {
    /// Parse a single property line within a VEVENT.
    fn parse_line(&mut self, line: &str) {
        // Split into name (with optional params) and value at the first unquoted ':'
        let Some((name_part, value)) = split_property(line) else {
            return;
        };

        // Extract the property name and parameters
        let (name, params) = if let Some(idx) = name_part.find(';') {
            (&name_part[..idx], Some(&name_part[idx + 1..]))
        } else {
            (name_part, None)
        };

        match name {
            "SUMMARY" => self.summary = Some(value.to_string()),
            "DESCRIPTION" => {
                if !value.is_empty() {
                    self.description = Some(value.to_string());
                }
            }
            "LOCATION" => {
                if !value.is_empty() {
                    self.location = Some(value.to_string());
                }
            }
            "DTSTART" => {
                self.dtstart = Some(value.to_string());
                self.dtstart_params = params.map(str::to_string);

                // Check for VALUE=DATE (all-day event)
                if let Some(p) = params {
                    if p.contains("VALUE=DATE") && !p.contains("VALUE=DATE-TIME") {
                        self.all_day = true;
                    }
                    // Extract TZID
                    if let Some(tz) = extract_param(p, "TZID") {
                        self.tzid = Some(tz.to_string());
                    }
                }

                // Heuristic: 8-digit value = all-day (YYYYMMDD)
                if value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) {
                    self.all_day = true;
                }
            }
            "DTEND" => {
                if !value.is_empty() {
                    self.dtend = Some(value.to_string());
                }
            }
            "RRULE" => {
                if !value.is_empty() {
                    self.rrule = Some(value.to_string());
                }
            }
            _ => {}
        }
    }

    /// Convert to a `RawEvent` if we have at least DTSTART and SUMMARY.
    fn build(self) -> Option<RawEvent> {
        let dtstart = self.dtstart?;
        let summary = self.summary.unwrap_or_default();

        if summary.is_empty() {
            return None;
        }

        Some(RawEvent {
            summary,
            description: self.description,
            location: self.location,
            dtstart,
            dtend: self.dtend,
            tzid: self.tzid,
            all_day: self.all_day,
            rrule: self.rrule,
        })
    }
}

/// Split a property line into (name_with_params, value) at the first ':'.
///
/// Handles quoted parameter values that may contain ':' characters, e.g.:
/// `DTSTART;TZID="US/Eastern":20260310T090000`
fn split_property(line: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (i, b) in line.bytes().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                return Some((&line[..i], &line[i + 1..]));
            }
            _ => {}
        }
    }
    None
}

/// Extract a parameter value from a parameter string.
///
/// E.g. `extract_param("TZID=Europe/Prague;VALUE=DATE-TIME", "TZID")` → `Some("Europe/Prague")`
fn extract_param<'a>(params: &'a str, key: &str) -> Option<&'a str> {
    for part in params.split(';') {
        if let Some(val) = part.strip_prefix(key).and_then(|s| s.strip_prefix('=')) {
            // Strip quotes if present
            return Some(val.trim_matches('"'));
        }
    }
    None
}

/// Parse all VEVENTs from an iCal string in one pass (old non-batched path).
/// Kept for testing equivalence with the chunked path.
#[cfg(test)]
fn parse_ics_oneshot(data: &str) -> Vec<RawEvent> {
    let lines = unfold_lines(data);
    let mut events = Vec::new();
    let mut in_vevent = false;
    let mut current: Option<RawEventBuilder> = None;

    for line in &lines {
        if line == "BEGIN:VEVENT" {
            in_vevent = true;
            current = Some(RawEventBuilder::default());
            continue;
        }
        if line == "END:VEVENT" {
            if let Some(builder) = current.take() {
                if let Some(event) = builder.build() {
                    events.push(event);
                }
            }
            in_vevent = false;
            continue;
        }
        if !in_vevent {
            continue;
        }
        if let Some(builder) = current.as_mut() {
            builder.parse_line(line);
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: run the chunked path and collect all events.
    fn parse_ics_chunked(data: &str) -> Vec<RawEvent> {
        split_into_chunks(data)
            .iter()
            .filter_map(|chunk| parse_chunk(chunk))
            .collect()
    }

    /// Assert two event lists are field-identical.
    fn assert_events_eq(old: &[RawEvent], new: &[RawEvent]) {
        assert_eq!(old.len(), new.len(), "event count mismatch");
        for (i, (a, b)) in old.iter().zip(new.iter()).enumerate() {
            assert_eq!(a.summary, b.summary, "event {i}: summary mismatch");
            assert_eq!(a.dtstart, b.dtstart, "event {i}: dtstart mismatch");
            assert_eq!(a.dtend, b.dtend, "event {i}: dtend mismatch");
            assert_eq!(a.tzid, b.tzid, "event {i}: tzid mismatch");
            assert_eq!(a.all_day, b.all_day, "event {i}: all_day mismatch");
            assert_eq!(a.rrule, b.rrule, "event {i}: rrule mismatch");
            assert_eq!(
                a.description, b.description,
                "event {i}: description mismatch"
            );
            assert_eq!(a.location, b.location, "event {i}: location mismatch");
        }
    }

    #[test]
    fn chunked_matches_oneshot_basic() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
BEGIN:VEVENT\r\n\
DTSTART:20260310T090000Z\r\n\
DTEND:20260310T100000Z\r\n\
SUMMARY:Morning standup\r\n\
LOCATION:Room 42\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
DTSTART;VALUE=DATE:20260315\r\n\
SUMMARY:Company holiday\r\n\
DESCRIPTION:Day off\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert_eq!(old.len(), 2);
    }

    #[test]
    fn chunked_matches_oneshot_rrule_and_tzid() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=Europe/Prague:20260101T080000\r\n\
DTEND;TZID=Europe/Prague:20260101T090000\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\n\
SUMMARY:Recurring meeting\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert_eq!(old[0].rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
        assert_eq!(old[0].tzid.as_deref(), Some("Europe/Prague"));
    }

    #[test]
    fn chunked_matches_oneshot_folded_lines() {
        // Note: RFC 5545 folded continuation lines start with a space or tab.
        // Can't use Rust `\` line continuation here — it eats leading whitespace.
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20260310T090000Z\r\nSUMMARY:This is a very long summary that gets\r\n  folded across multiple lines\r\nDESCRIPTION:Also\r\n folded\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert_eq!(
            old[0].summary,
            "This is a very long summary that gets folded across multiple lines"
        );
        assert_eq!(old[0].description.as_deref(), Some("Alsofolded"));
    }

    #[test]
    fn chunked_matches_oneshot_quoted_tzid() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\n\
DTSTART;TZID=\"US/Eastern\":20260310T090000\r\n\
SUMMARY:Quoted TZ\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert_eq!(old[0].tzid.as_deref(), Some("US/Eastern"));
    }

    #[test]
    fn split_skips_non_vevent_components() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Europe/Prague\r\n\
BEGIN:STANDARD\r\n\
DTSTART:19701025T030000\r\n\
END:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
DTSTART:20260310T090000Z\r\n\
SUMMARY:Real event\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let chunks = split_into_chunks(ics);
        assert_eq!(chunks.len(), 1);
        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
    }

    #[test]
    fn empty_and_no_summary_skipped() {
        let ics = "\
BEGIN:VCALENDAR\r\n\
BEGIN:VEVENT\r\n\
DTSTART:20260310T090000Z\r\n\
END:VEVENT\r\n\
BEGIN:VEVENT\r\n\
DTSTART:20260311T090000Z\r\n\
SUMMARY:Valid\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].summary, "Valid");
    }

    #[test]
    fn chunked_matches_oneshot_real_feed() {
        let ics = include_str!("../testdata/finland.ics");
        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert!(
            old.len() >= 20,
            "expected at least 20 events, got {}",
            old.len()
        );
    }

    #[test]
    fn unix_line_endings() {
        let ics = "\
BEGIN:VCALENDAR\n\
BEGIN:VEVENT\n\
DTSTART:20260310T090000Z\n\
SUMMARY:Unix LF\n\
END:VEVENT\n\
END:VCALENDAR\n";

        let old = parse_ics_oneshot(ics);
        let new = parse_ics_chunked(ics);
        assert_events_eq(&old, &new);
        assert_eq!(old.len(), 1);
    }
}
