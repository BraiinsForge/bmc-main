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

//! A blueprint-authored calendar day, validated at load.

use std::borrow::Cow;
use std::fmt;

use chrono::NaiveDate;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, Deserialize, Deserializer, Visitor};

/// The day a profile treats as today, standing in for the wall clock.
///
/// A device whose payloads carry a calendar reads the real date by default,
/// so a scenario opened any week announces that week.
/// Naming a day instead scripts the calendar: the eve of a race,
/// a mid-week lull, the turn of a year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarDay(NaiveDate);

impl CalendarDay {
    #[must_use]
    pub fn date(self) -> NaiveDate {
        self.0
    }

    /// The day a profile runs against, real unless the blueprint named one.
    #[must_use]
    pub fn or_today(day: Option<Self>) -> NaiveDate {
        day.map_or_else(|| chrono::Utc::now().date_naive(), Self::date)
    }
}

impl From<NaiveDate> for CalendarDay {
    fn from(date: NaiveDate) -> Self {
        Self(date)
    }
}

impl<'de> Deserialize<'de> for CalendarDay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DayVisitor;

        impl Visitor<'_> for DayVisitor {
            type Value = CalendarDay;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a calendar day written YYYY-MM-DD")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<CalendarDay, E> {
                value
                    .parse::<NaiveDate>()
                    .map(CalendarDay)
                    .map_err(|_| E::custom(format!("{value:?} is not a day written YYYY-MM-DD")))
            }
        }

        // Rejected inside the visitor for the same reason a status is: json5
        // locates whatever `deserialize_*` returns, and a `try_from` running
        // after that call arrives without the caret.
        deserializer.deserialize_str(DayVisitor)
    }
}

impl JsonSchema for CalendarDay {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("CalendarDay")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "format": "date",
            "pattern": r"^\d{4}-\d{2}-\d{2}$",
            "description": "The day the profile treats as today, e.g. 2026-08-21"
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::CalendarDay;

    #[test]
    fn reads_a_written_day() {
        let day: CalendarDay = json5::from_str("'2026-08-21'").expect("BUG: that is a written day");
        assert_eq!(day.date().to_string(), "2026-08-21");
    }

    #[test]
    fn an_absent_day_falls_back_to_the_real_one() {
        assert_eq!(CalendarDay::or_today(None), chrono::Utc::now().date_naive(),);
    }

    #[test]
    fn rejects_a_day_that_never_was_with_a_located_error() {
        let source = "{\n  today: '2026-02-31',\n}";
        let err = json5::from_str::<BTreeMap<String, CalendarDay>>(source)
            .expect_err("BUG: February has no 31st");
        let json5::Error::Message { msg, location } = err;
        assert!(msg.contains("2026-02-31"), "message was: {msg}");
        assert!(location.is_some(), "the frame needs a caret to point with");
    }
}
