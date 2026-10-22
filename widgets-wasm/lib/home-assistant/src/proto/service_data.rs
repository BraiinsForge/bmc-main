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

//! The extra fields of a service call or command, and the check they pass before they are written.

use super::check::is_free_name;
use super::client::{MAX_SERVICE_DATA_TEXT_BYTES, RequestError};
use super::json::ObjectWriter;

/// Extra fields of a service call beside its target, or of a command beside its id and type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServiceData(
    // A Vec, not a map: entries are only scanned, never looked up, and keep their order.
    Vec<(String, DataValue)>,
);

#[derive(Debug, Clone, PartialEq)]
enum DataValue {
    Text(String),
    Integer(i64),
    Bool(bool),
}

impl ServiceData {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn text(mut self, key: &str, value: &str) -> Self {
        self.0
            .push((key.to_owned(), DataValue::Text(value.to_owned())));
        self
    }

    #[must_use]
    pub fn integer(mut self, key: &str, value: i64) -> Self {
        self.0.push((key.to_owned(), DataValue::Integer(value)));
        self
    }

    #[must_use]
    pub fn boolean(mut self, key: &str, value: bool) -> Self {
        self.0.push((key.to_owned(), DataValue::Bool(value)));
        self
    }

    /// The data as it will be written, if it keeps every bound,
    /// and names nothing `reserved` refuses, in its keys or texts.
    pub(super) fn checked(
        &self,
        reserved: fn(&str) -> bool,
    ) -> Result<CheckedData<'_>, RequestError> {
        let text = |text: &str| {
            (text.len() <= MAX_SERVICE_DATA_TEXT_BYTES && !reserved(text))
                .then_some(())
                .ok_or(RequestError::InvalidData)
        };
        let mut checked = Vec::with_capacity(self.0.len());
        for (key, value) in &self.0 {
            if !is_free_name(key) {
                return Err(RequestError::InvalidName);
            }
            if reserved(key) {
                return Err(RequestError::InvalidData);
            }
            let value = match value {
                DataValue::Text(value) => {
                    text(value)?;
                    WireValue::Text(value)
                }
                DataValue::Integer(number) => WireValue::Integer(*number),
                DataValue::Bool(flag) => WireValue::Bool(*flag),
            };
            checked.push((key.as_str(), value));
        }
        Ok(CheckedData(checked))
    }
}

/// [`ServiceData`] that passed every request rule, in the form the writer takes,
/// so nothing unchecked can reach a message.
pub(super) struct CheckedData<'a>(Vec<(&'a str, WireValue<'a>)>);

enum WireValue<'a> {
    Text(&'a str),
    Integer(i64),
    Bool(bool),
}

impl CheckedData<'_> {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn write_into(&self, object: ObjectWriter) -> ObjectWriter {
        self.0
            .iter()
            .fold(object, |object, (key, value)| match value {
                WireValue::Text(text) => object.string(key, text),
                WireValue::Integer(number) => object.int(key, *number),
                WireValue::Bool(flag) => object.bool(key, *flag),
            })
    }
}
