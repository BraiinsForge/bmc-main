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
use super::client::{
    MAX_SERVICE_DATA_LIST_ITEMS, MAX_SERVICE_DATA_TEXT_BYTES, RequestError, owned,
};
use super::json::{Decimal, ObjectWriter};

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
    Decimal { value: f64, decimals: u32 },
    Bool(bool),
    Texts(Vec<String>),
    Integers(Vec<i64>),
    Decimals { values: Vec<f64>, decimals: u32 },
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

    /// `value` rounded to `decimals` decimal places, such as a temperature of `21.5` with one.
    /// A call or command carrying a NaN, an infinity, more places than [`f64::DIGITS`],
    /// or a value too large to write exactly is refused.
    #[must_use]
    pub fn decimal(mut self, key: &str, value: f64, decimals: u32) -> Self {
        self.0
            .push((key.to_owned(), DataValue::Decimal { value, decimals }));
        self
    }

    #[must_use]
    pub fn boolean(mut self, key: &str, value: bool) -> Self {
        self.0.push((key.to_owned(), DataValue::Bool(value)));
        self
    }

    /// A list of texts, such as the `entity_ids` a command asks about.
    #[must_use]
    pub fn texts(mut self, key: &str, values: &[&str]) -> Self {
        self.0.push((
            key.to_owned(),
            DataValue::Texts(owned(values.iter().copied())),
        ));
        self
    }

    /// A list of integers, such as a light's `rgb_color`.
    #[must_use]
    pub fn integers(mut self, key: &str, values: &[i64]) -> Self {
        self.0
            .push((key.to_owned(), DataValue::Integers(values.to_vec())));
        self
    }

    /// A list of decimals, each rounded as [`Self::decimal`] rounds one, such as a light's `hs_color`.
    #[must_use]
    pub fn decimals(mut self, key: &str, values: &[f64], decimals: u32) -> Self {
        let values = values.to_vec();
        self.0
            .push((key.to_owned(), DataValue::Decimals { values, decimals }));
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
        let list = |len: usize| {
            (len <= MAX_SERVICE_DATA_LIST_ITEMS)
                .then_some(())
                .ok_or(RequestError::InvalidData)
        };
        let decimal =
            |value: f64, decimals| Decimal::new(value, decimals).ok_or(RequestError::InvalidData);
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
                DataValue::Decimal { value, decimals } => {
                    WireValue::Decimal(decimal(*value, *decimals)?)
                }
                DataValue::Bool(flag) => WireValue::Bool(*flag),
                DataValue::Texts(texts) => {
                    list(texts.len())?;
                    texts.iter().try_for_each(|value| text(value))?;
                    WireValue::Texts(texts)
                }
                DataValue::Integers(numbers) => {
                    list(numbers.len())?;
                    WireValue::Integers(numbers)
                }
                DataValue::Decimals { values, decimals } => {
                    list(values.len())?;
                    WireValue::Decimals(
                        values
                            .iter()
                            .map(|value| decimal(*value, *decimals))
                            .collect::<Result<_, _>>()?,
                    )
                }
            };
            checked.push((key.as_str(), value));
        }
        Ok(CheckedData(checked))
    }

    pub(super) fn has_key(&self, key: &str) -> bool {
        self.0.iter().any(|(name, _)| name == key)
    }
}

/// [`ServiceData`] that passed every request rule, in the form the writer takes,
/// so nothing unchecked can reach a message.
pub(super) struct CheckedData<'a>(Vec<(&'a str, WireValue<'a>)>);

enum WireValue<'a> {
    Text(&'a str),
    Integer(i64),
    Decimal(Decimal),
    Bool(bool),
    Texts(&'a [String]),
    Integers(&'a [i64]),
    Decimals(Vec<Decimal>),
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
                WireValue::Decimal(decimal) => object.decimal(key, *decimal),
                WireValue::Bool(flag) => object.bool(key, *flag),
                WireValue::Texts(texts) => object.strings(key, texts.iter().map(String::as_str)),
                WireValue::Integers(numbers) => object.ints(key, numbers),
                WireValue::Decimals(decimals) => object.decimals(key, decimals),
            })
    }
}
