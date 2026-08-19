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

//! A blueprint-authored HTTP status, validated at load.

use std::borrow::Cow;
use std::fmt;

use axum::http::StatusCode;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserialize, Deserializer, Visitor};

/// An endpoint's HTTP status, validated to a registered code at load.
///
/// A blueprint sets a status to force a failure (`503`, `401`), so a typo like
/// `status: 99` must not degrade to a healthy `200` and serve success in place
/// of the fault it asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpStatus(StatusCode);

/// Every code `http` carries a reason phrase for — the one source both the
/// schema enum and the load-time check read, so the two cannot drift.
fn registered_codes() -> impl Iterator<Item = u16> {
    (100..1000)
        .filter(|&code| StatusCode::from_u16(code).is_ok_and(|s| s.canonical_reason().is_some()))
}

impl HttpStatus {
    pub const OK: Self = Self(StatusCode::OK);
    pub const SERVICE_UNAVAILABLE: Self = Self(StatusCode::SERVICE_UNAVAILABLE);

    #[must_use]
    pub fn code(self) -> StatusCode {
        self.0
    }
}

impl Default for HttpStatus {
    fn default() -> Self {
        Self::OK
    }
}

/// Validate a blueprint integer into a status, or say why it cannot stand.
fn from_code<E: de::Error>(value: i128) -> Result<HttpStatus, E> {
    u16::try_from(value)
        .ok()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .filter(|status| status.canonical_reason().is_some())
        .map(HttpStatus)
        .ok_or_else(|| E::custom(format!("{value} is not a registered HTTP status code")))
}

impl<'de> Deserialize<'de> for HttpStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StatusVisitor;

        impl Visitor<'_> for StatusVisitor {
            type Value = HttpStatus;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an HTTP status code (100–999)")
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<HttpStatus, E> {
                from_code(i128::from(value))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<HttpStatus, E> {
                from_code(i128::from(value))
            }
        }

        // Reject inside the visitor, not via `#[serde(try_from = "u16")]`: json5
        // stamps a location onto whatever its `deserialize_*` call returns, and a
        // `try_from` runs after that call, arriving unlocated — which costs the
        // frame its caret. `i64` also keeps a negative literal intact to report.
        deserializer.deserialize_i64(StatusVisitor)
    }
}

impl JsonSchema for HttpStatus {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("HttpStatus")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        let codes: Vec<u16> = registered_codes().collect();
        json_schema!({
            "type": "integer",
            "enum": codes,
            "description": "HTTP status this endpoint returns, e.g. 200, 401 or 503"
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::HttpStatus;

    #[test]
    fn accepts_a_valid_fault_status() {
        let status: HttpStatus = json5::from_str("503").expect("BUG: 503 is a valid status");
        assert_eq!(status.code().as_u16(), 503);
    }

    #[test]
    fn default_is_two_hundred() {
        assert_eq!(HttpStatus::default().code().as_u16(), 200);
    }

    #[test]
    fn rejects_an_out_of_range_status_with_a_located_error() {
        let source = "{\n  status: 99,\n}";
        let err = json5::from_str::<BTreeMap<String, HttpStatus>>(source)
            .expect_err("BUG: 99 must be rejected");
        let json5::Error::Message { msg, location } = err;
        assert!(
            msg.contains("99 is not a registered HTTP status code"),
            "message was: {msg}"
        );
        let location = location.expect("BUG: json5 must stamp the source location");
        assert_eq!(location.line, 2, "the status is on the second line");
        assert_eq!(location.column, 11, "the caret must sit on the `99`");
    }

    #[test]
    fn rejects_a_negative_status() {
        let err = json5::from_str::<HttpStatus>("-1").expect_err("BUG: -1 must be rejected");
        let json5::Error::Message { msg, .. } = err;
        assert!(msg.contains("-1 is not a registered"), "message was: {msg}");
    }

    #[test]
    fn rejects_a_status_past_the_u16_range() {
        assert!(json5::from_str::<HttpStatus>("70000").is_err());
    }

    #[test]
    fn rejects_an_in_range_but_unregistered_code() {
        // `StatusCode::from_u16` accepts anything in 100–999, so a bare range
        // check would serve 700 as a "fault" no client could interpret.
        assert!(
            json5::from_str::<HttpStatus>("700").is_err(),
            "700 has no registered reason phrase"
        );
    }

    #[test]
    fn the_schema_enumerates_the_codes_the_check_accepts() {
        let schema = schemars::schema_for!(HttpStatus);
        let json = serde_json::to_value(&schema).expect("BUG: schema must serialize");
        let codes: Vec<u64> = json["enum"]
            .as_array()
            .expect("BUG: schema must enumerate codes")
            .iter()
            .map(|v| v.as_u64().expect("BUG: codes are integers"))
            .collect();

        assert!(codes.contains(&200) && codes.contains(&503));
        assert!(
            !codes.contains(&99),
            "the schema must reject what the check does"
        );
        assert!(
            !codes.contains(&700),
            "unregistered codes stay out of the menu"
        );
        for code in &codes {
            let literal = code.to_string();
            assert!(
                json5::from_str::<HttpStatus>(&literal).is_ok(),
                "schema offers {code} but the load-time check refuses it"
            );
        }
    }
}
