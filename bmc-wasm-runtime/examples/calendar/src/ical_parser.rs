// Copyright (C) 2026  Braiins Systems s.r.o.

//! Lightweight iCal (.ics) parser — extracts VEVENTs from RFC 5545 data.
//!
//! Hand-rolled line-based parser instead of the `ical` crate to stay within
//! the WASM fuel budget. iCal is a simple line-based format:
//! - `BEGIN:VEVENT` / `END:VEVENT` delimit events
//! - Properties are `NAME:VALUE` or `NAME;PARAM=VAL:VALUE`
//! - Long lines are folded: continuation lines start with a space or tab

#[allow(clippy::wildcard_imports)]
use bmc_wasm_sdk::*;

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

/// Parse an iCal (.ics) string and extract all VEVENTs.
pub fn parse_ics(data: &str) -> Vec<RawEvent> {
    let mut events = Vec::new();
    let lines = unfold_lines(data);

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
            log_info!("skipping VEVENT with no SUMMARY");
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
