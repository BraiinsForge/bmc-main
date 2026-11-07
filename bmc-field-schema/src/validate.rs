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

use std::collections::BTreeMap;

use bmc_shared_time::time::Timezone;
use indexmap::IndexMap;

use crate::{
    ArrayParam, DoubleParam, IntegerParam, ListItems, MAX_PARAM_STRING_LENGTH, ObjectParam,
    ParamDefinition, ParamKey, ParamKind, ParamValue, ParamValueConversionError, RowUniqueness,
    Scalar, Shape, StringParam, f64_canonical_bits,
};

/// What a schema key the input omits turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingValues {
    /// The field's default, or `Null` for an optional field without one.
    Default,
    /// A [`ViolationKind::Required`] violation.
    Reject,
}

/// A value that failed its schema, addressed relative to the value map:
/// `["key"]`, `["key"][i]` for a list item, `["key"][i]["field"]` for an object item's field.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    pub path: String,
    pub kind: ViolationKind,
}

impl Violation {
    fn new(path: impl Into<String>, kind: ViolationKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

/// What is wrong with a value.
///
/// A request's edge words it in the request's language.
/// `Display` is the English for what has no language to ask for, such as a manifest's load error.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "test-support",
    derive(strum::EnumDiscriminants),
    strum_discriminants(derive(strum::EnumIter))
)]
pub enum ViolationKind {
    Required,
    /// A value the edge could not read.
    Unreadable(UnreadableValue),
    UnknownParam,
    UnknownField,
    NotAList,
    NotAnObject,
    NotText,
    NotWholeNumber,
    NotNumber,
    NotTrueOrFalse,
    NotTimezone,
    InvalidTimezone,
    NotFinite,
    NotListed,
    TooFewItems {
        min: usize,
    },
    TooManyItems {
        max: usize,
    },
    TooManyBytes {
        max: usize,
    },
    TooFewCharacters {
        min: usize,
    },
    TooManyCharacters {
        max: usize,
    },
    BelowMin(Bound),
    AboveMax(Bound),
    /// `item` is the 1-based place of the earlier item.
    RepeatsItem {
        item: usize,
    },
    /// `item` is the 1-based place of the earlier item.
    SameAsItem {
        item: usize,
    },
}

impl std::fmt::Display for ViolationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Required => f.write_str("Missing value!"),
            Self::Unreadable(value) => write!(f, "{value}"),
            Self::UnknownParam => f.write_str("Unknown setting!"),
            Self::UnknownField => f.write_str("Unknown field!"),
            Self::NotAList => f.write_str("Must be a list"),
            Self::NotAnObject => f.write_str("Must be an object"),
            Self::NotText => f.write_str("Must be text"),
            Self::NotWholeNumber => f.write_str("Must be a whole number"),
            Self::NotNumber => f.write_str("Must be a number"),
            Self::NotTrueOrFalse => f.write_str("Must be true or false"),
            Self::NotTimezone => f.write_str("Must be a timezone"),
            Self::InvalidTimezone => f.write_str("Must be a valid timezone"),
            Self::NotFinite => f.write_str("Must be a finite number"),
            Self::NotListed => f.write_str("Must be one of the listed options"),
            Self::TooFewItems { min } => write!(f, "Must have at least {}", item_count(*min)),
            Self::TooManyItems { max } => write!(f, "Must have at most {}", item_count(*max)),
            Self::TooManyBytes { max } => write!(f, "Must be at most {max} bytes"),
            Self::TooFewCharacters { min } => {
                write!(f, "Must be at least {}", character_count(*min))
            }
            Self::TooManyCharacters { max } => {
                write!(f, "Must be at most {}", character_count(*max))
            }
            Self::BelowMin(bound) => write!(f, "Must be at least {bound}"),
            Self::AboveMax(bound) => write!(f, "Must be at most {bound}"),
            Self::RepeatsItem { item } => write!(f, "Repeats item {item}!"),
            Self::SameAsItem { item } => write!(f, "Same as item {item}!"),
        }
    }
}

/// Why the edge could not read a value into a [`ParamValue`].
///
/// `at` is where it sits below the value, such as `[1]["label"]`,
/// and empty for the value itself.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(
    feature = "test-support",
    derive(strum::EnumDiscriminants),
    strum_discriminants(derive(strum::EnumIter))
)]
pub enum UnreadableValue {
    KindUnset {
        at: String,
    },
    InvalidFieldKey {
        at: String,
        key: String,
    },
    /// Reads as the [`ViolationKind`] it amounts to.
    Conversion(ParamValueConversionError),
}

impl From<&ParamValueConversionError> for ViolationKind {
    fn from(error: &ParamValueConversionError) -> Self {
        match error {
            ParamValueConversionError::InvalidObjectKey(key) => {
                Self::Unreadable(UnreadableValue::InvalidFieldKey {
                    at: String::new(),
                    key: key.clone(),
                })
            }
            ParamValueConversionError::UnrepresentableNumber => Self::NotNumber,
            ParamValueConversionError::NonFiniteNumber => Self::NotFinite,
            ParamValueConversionError::StringTooLong { max, .. } => {
                Self::TooManyBytes { max: *max }
            }
        }
    }
}

impl std::fmt::Display for UnreadableValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KindUnset { at } if at.is_empty() => f.write_str("This value has no type!"),
            Self::KindUnset { at } => write!(f, "The value at {at} has no type!"),
            Self::InvalidFieldKey { at, key } if at.is_empty() => {
                write!(f, "Invalid field name: {key}!")
            }
            Self::InvalidFieldKey { at, key } => write!(f, "Invalid field name at {at}: {key}!"),
            Self::Conversion(error) => write!(f, "{}", ViolationKind::from(error)),
        }
    }
}

/// A numeric limit, whole or not as its schema has it.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "test-support",
    derive(strum::EnumDiscriminants),
    strum_discriminants(derive(strum::EnumIter))
)]
pub enum Bound {
    Integer(i32),
    Double(f64),
}

impl From<i32> for Bound {
    fn from(value: i32) -> Self {
        Self::Integer(value)
    }
}

impl From<f64> for Bound {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl std::fmt::Display for Bound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Integer(value) => write!(f, "{value}"),
            Self::Double(value) => write!(f, "{value}"),
        }
    }
}

/// Validate wire values against a field schema and project them
/// onto typed values, collecting every violation before returning.
///
/// Each present value is as the wire edge decoded it:
/// an `Err` is a value the edge could not read, reported at its key.
pub fn validate_values(
    fields: &IndexMap<ParamKey, ParamDefinition>,
    values: &BTreeMap<String, Result<ParamValue, UnreadableValue>>,
    missing: MissingValues,
) -> Result<BTreeMap<ParamKey, ParamValue>, Vec<Violation>> {
    let mut violations = Vec::new();
    let mut typed = BTreeMap::new();

    for (key, def) in fields {
        let path = key_path(key.as_str());
        let Some(value) = values.get(key.as_str()) else {
            match missing {
                MissingValues::Default => {
                    typed.insert(key.clone(), ParamValue::from_param_kind_default(&def.kind));
                }
                MissingValues::Reject => {
                    violations.push(Violation::new(path, ViolationKind::Required));
                }
            }
            continue;
        };
        let value = match value {
            Ok(value) => value,
            Err(unreadable) => {
                violations.push(Violation::new(
                    path,
                    ViolationKind::Unreadable(unreadable.clone()),
                ));
                continue;
            }
        };

        if def.is_optional && matches!(value, ParamValue::Null) {
            typed.insert(key.clone(), ParamValue::Null);
            continue;
        }
        if !def.is_optional && lacks_value(value) {
            violations.push(Violation::new(path, ViolationKind::Required));
            continue;
        }

        if let Some(value) = validate_value(&path, &def.kind, value, &mut violations) {
            typed.insert(key.clone(), value);
        }
    }

    for key in values.keys() {
        if !fields.contains_key(key.as_str()) {
            violations.push(Violation::new(key_path(key), ViolationKind::UnknownParam));
        }
    }

    if violations.is_empty() {
        Ok(typed)
    } else {
        Err(violations)
    }
}

/// Null, or the empty text the operator UI reads as no value:
/// either way, what a required value refuses.
fn lacks_value(value: &ParamValue) -> bool {
    match value {
        ParamValue::Null => true,
        ParamValue::String(s) => s.is_empty(),
        ParamValue::Boolean(_)
        | ParamValue::Integer(_)
        | ParamValue::Double(_)
        | ParamValue::List(_)
        | ParamValue::Object(_) => false,
    }
}

/// Debug-formats the key, so a client-supplied key
/// with quotes, newlines, etc. cannot break the path.
fn key_path(key: &str) -> String {
    format!("[{key:?}]")
}

fn type_mismatch(scalar: Scalar<'_>) -> ViolationKind {
    match scalar {
        Scalar::String(_) => ViolationKind::NotText,
        Scalar::Integer(_) => ViolationKind::NotWholeNumber,
        Scalar::Double(_) => ViolationKind::NotNumber,
        Scalar::Boolean(_) => ViolationKind::NotTrueOrFalse,
        Scalar::Timezone(_) => ViolationKind::NotTimezone,
    }
}

fn validate_value(
    path: &str,
    kind: &ParamKind,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    match kind.shape() {
        Shape::Scalar(scalar) => validate_scalar(path, scalar, value, violations),
        Shape::Array(array) => validate_list(path, array, value, violations),
    }
}

fn validate_list(
    path: &str,
    array: &ArrayParam,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    let ParamValue::List(items) = value else {
        violations.push(Violation::new(path, ViolationKind::NotAList));
        return None;
    };
    validate_list_items(path, array, items, violations).map(ParamValue::List)
}

/// The items typed, or `None` if the list has a violation.
pub(crate) fn validate_list_items(
    path: &str,
    array: &ArrayParam,
    items: &[ParamValue],
    violations: &mut Vec<Violation>,
) -> Option<Vec<ParamValue>> {
    let before = violations.len();
    if items.len() < array.min_items {
        violations.push(Violation::new(
            path,
            ViolationKind::TooFewItems {
                min: array.min_items,
            },
        ));
    }
    if items.len() > array.max_items {
        // Past the bound, item checks add nothing but one violation per item to the response.
        violations.push(Violation::new(
            path,
            ViolationKind::TooManyItems {
                max: array.max_items,
            },
        ));
        return None;
    }
    let typed = match &array.items {
        ListItems::Scalar { kind, unique } => {
            let items = validate_each(path, items, violations, |path, item, violations| {
                validate_scalar(path, kind.as_scalar(), item, violations)
            })?;
            if *unique {
                push_repeats(path, &items, None, violations);
            }
            items
        }
        ListItems::Object { object, unique } => {
            let rows = validate_each(path, items, violations, |path, item, violations| {
                validate_object(path, object, item, violations)
            })?;
            match unique {
                RowUniqueness::Off => {}
                RowUniqueness::Whole => push_repeats(path, &rows, None, violations),
                RowUniqueness::By(keys) => {
                    let identities: Vec<_> = rows.iter().map(|row| key_fields(row, keys)).collect();
                    push_repeats(path, &identities, Some(keys), violations);
                }
            }
            rows.into_iter().map(ParamValue::Object).collect()
        }
    };
    (violations.len() == before).then_some(typed)
}

/// Every item typed, or `None` if any item has a violation.
/// Unlike an optional field, an item always has a value.
fn validate_each<T>(
    path: &str,
    items: &[ParamValue],
    violations: &mut Vec<Violation>,
    validate: impl Fn(&str, &ParamValue, &mut Vec<Violation>) -> Option<T>,
) -> Option<Vec<T>> {
    let before = violations.len();
    let typed: Vec<T> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let path = format!("{path}[{i}]");
            if lacks_value(item) {
                violations.push(Violation::new(path, ViolationKind::Required));
                return None;
            }
            validate(&path, item, violations)
        })
        .collect();
    (violations.len() == before).then_some(typed)
}

/// A field the row omits reads as null, which only an optional field accepts.
fn validate_object(
    path: &str,
    object: &ObjectParam,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<BTreeMap<ParamKey, ParamValue>> {
    let ParamValue::Object(fields) = value else {
        violations.push(Violation::new(path, ViolationKind::NotAnObject));
        return None;
    };
    let before = violations.len();
    let mut typed = BTreeMap::new();
    for (key, field) in &object.fields {
        let field_path = format!("{path}{}", key_path(key.as_str()));
        let value = fields.get(key).unwrap_or(&ParamValue::Null);
        if field.is_optional && matches!(value, ParamValue::Null) {
            typed.insert(key.clone(), ParamValue::Null);
        } else if !field.is_optional && lacks_value(value) {
            violations.push(Violation::new(field_path, ViolationKind::Required));
        } else if let Some(value) =
            validate_scalar(&field_path, field.kind.as_scalar(), value, violations)
        {
            typed.insert(key.clone(), value);
        }
    }
    for key in fields.keys() {
        if !object.fields.contains_key(key) {
            violations.push(Violation::new(
                format!("{path}{}", key_path(key.as_str())),
                ViolationKind::UnknownField,
            ));
        }
    }
    (violations.len() == before).then_some(typed)
}

/// A whole repeat is reported at the item,
/// a key repeat at each of its key fields.
fn push_repeats<T: PartialEq>(
    path: &str,
    identities: &[T],
    keys: Option<&[ParamKey]>,
    violations: &mut Vec<Violation>,
) {
    for (i, identity) in identities.iter().enumerate() {
        let Some(first) = identities
            .iter()
            .take(i)
            .position(|earlier| earlier == identity)
        else {
            continue;
        };
        let item = first + 1;
        match keys {
            None => violations.push(Violation::new(
                format!("{path}[{i}]"),
                ViolationKind::RepeatsItem { item },
            )),
            Some(keys) => violations.extend(keys.iter().map(|key| {
                Violation::new(
                    format!("{path}[{i}]{}", key_path(key.as_str())),
                    ViolationKind::SameAsItem { item },
                )
            })),
        }
    }
}

fn key_fields<'a>(
    row: &'a BTreeMap<ParamKey, ParamValue>,
    keys: &[ParamKey],
) -> Vec<&'a ParamValue> {
    keys.iter()
        .map(|key| {
            row.get(key).expect(
                "BUG: parsing checks unique_items keys against the row's fields, \
                and a typed row holds every one",
            )
        })
        .collect()
}

fn item_count(n: usize) -> String {
    if n == 1 {
        "1 item".to_owned()
    } else {
        format!("{n} items")
    }
}

fn character_count(n: usize) -> String {
    if n == 1 {
        "1 character".to_owned()
    } else {
        format!("{n} characters")
    }
}

/// The bound `s` breaks, counted in characters (Unicode code points) as JSON Schema counts them.
pub(crate) fn length_violation(
    s: &str,
    min: Option<usize>,
    max: Option<usize>,
) -> Option<ViolationKind> {
    let len = s.chars().count();
    if let Some(lo) = min
        && len < lo
    {
        return Some(ViolationKind::TooFewCharacters { min: lo });
    }
    if let Some(hi) = max
        && len > hi
    {
        return Some(ViolationKind::TooManyCharacters { max: hi });
    }
    None
}

/// The bound `n` breaks.
pub(crate) fn bound_violation<T: Copy + PartialOrd + Into<Bound>>(
    n: T,
    min: Option<T>,
    max: Option<T>,
) -> Option<ViolationKind> {
    if let Some(lo) = min
        && n < lo
    {
        return Some(ViolationKind::BelowMin(lo.into()));
    }
    if let Some(hi) = max
        && n > hi
    {
        return Some(ViolationKind::AboveMax(hi.into()));
    }
    None
}

fn validate_string(
    path: &str,
    param: &StringParam,
    s: &str,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    if s.len() > MAX_PARAM_STRING_LENGTH {
        violations.push(Violation::new(
            path,
            ViolationKind::TooManyBytes {
                max: MAX_PARAM_STRING_LENGTH,
            },
        ));
        return None;
    }
    if let Some(kind) = length_violation(s, param.min_length, param.max_length) {
        violations.push(Violation::new(path, kind));
        return None;
    }
    if !param.enum_values.is_empty() && !param.enum_values.iter().any(|o| o.value == s) {
        violations.push(Violation::new(path, ViolationKind::NotListed));
        return None;
    }
    Some(ParamValue::String(s.to_owned()))
}

pub(crate) fn validate_scalar(
    path: &str,
    scalar: Scalar<'_>,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    match (scalar, value) {
        (Scalar::String(param), ParamValue::String(s)) => {
            validate_string(path, param, s, violations)
        }
        (Scalar::Timezone(_), ParamValue::String(s)) => {
            if Timezone::lookup(s).is_none() {
                violations.push(Violation::new(path, ViolationKind::InvalidTimezone));
                return None;
            }
            Some(ParamValue::String(s.clone()))
        }
        (Scalar::Boolean(_), ParamValue::Boolean(b)) => Some(ParamValue::Boolean(*b)),
        (
            Scalar::Integer(IntegerParam {
                min,
                max,
                enum_values,
                ..
            }),
            ParamValue::Integer(i),
        ) => {
            let mut ok = true;
            if let Some(kind) = bound_violation(*i, *min, *max) {
                violations.push(Violation::new(path, kind));
                ok = false;
            }
            if !enum_values.is_empty() && !enum_values.iter().any(|o| o.value == *i) {
                violations.push(Violation::new(path, ViolationKind::NotListed));
                ok = false;
            }
            if ok {
                Some(ParamValue::Integer(*i))
            } else {
                None
            }
        }
        (Scalar::Double(param), ParamValue::Double(d)) => {
            validate_double(path, param, *d, violations)
        }
        // JSON has one number type, so a manifest's `1` parses as an integer even for a double.
        (Scalar::Double(param), ParamValue::Integer(i)) => {
            validate_double(path, param, f64::from(*i), violations)
        }
        (other_kind, _) => {
            violations.push(Violation::new(path, type_mismatch(other_kind)));
            None
        }
    }
}

fn validate_double(
    path: &str,
    param: &DoubleParam,
    d: f64,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    if !d.is_finite() {
        violations.push(Violation::new(path, ViolationKind::NotFinite));
        return None;
    }
    let mut ok = true;
    if let Some(kind) = bound_violation(d, param.min, param.max) {
        violations.push(Violation::new(path, kind));
        ok = false;
    }
    if !param.enum_values.is_empty()
        && !param
            .enum_values
            .iter()
            .any(|o| f64_canonical_bits(o.value) == f64_canonical_bits(d))
    {
        violations.push(Violation::new(path, ViolationKind::NotListed));
        ok = false;
    }
    ok.then_some(ParamValue::Double(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    type Values = BTreeMap<String, Result<ParamValue, UnreadableValue>>;

    fn violations(
        result: Result<BTreeMap<ParamKey, ParamValue>, Vec<Violation>>,
    ) -> Vec<Violation> {
        result.err().unwrap_or_default()
    }

    fn list_violations(
        items: &serde_json::Value,
        unique_items: &serde_json::Value,
        list: &serde_json::Value,
    ) -> Vec<Violation> {
        let fields: IndexMap<ParamKey, ParamDefinition> = serde_json::from_value(json!({
            "list": {
                "name": "L",
                "type": "array",
                "items": items,
                "max_items": 5,
                "unique_items": unique_items,
            },
        }))
        .expect("BUG: the list schema parses");
        let value = ParamValue::try_from(list).expect("BUG: the list value converts");
        let values = Values::from([("list".to_owned(), Ok(value))]);
        violations(validate_values(&fields, &values, MissingValues::Default))
    }

    #[test]
    fn a_unique_list_reports_each_repeat_at_its_own_index() {
        assert_eq!(
            list_violations(
                &json!({ "type": "string" }),
                &json!(true),
                &json!(["BTC", "ETH", "BTC", "ETH"])
            ),
            [
                Violation::new(r#"["list"][2]"#, ViolationKind::RepeatsItem { item: 1 }),
                Violation::new(r#"["list"][3]"#, ViolationKind::RepeatsItem { item: 2 }),
            ],
        );
    }

    #[test]
    fn a_list_without_unique_items_keeps_its_repeats() {
        let found = list_violations(
            &json!({ "type": "string" }),
            &json!(false),
            &json!(["BTC", "BTC"]),
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_unique_list_counts_zero_and_negative_zero_as_one_number() {
        assert_eq!(
            list_violations(
                &json!({ "type": "double" }),
                &json!(true),
                &json!([0.0, -0.0])
            ),
            [Violation::new(
                r#"["list"][1]"#,
                ViolationKind::RepeatsItem { item: 1 }
            )],
        );
    }

    fn links() -> serde_json::Value {
        json!({
            "type": "object",
            "fields": {
                "label": { "name": "Label", "type": "string" },
                "url": { "name": "URL", "type": "string", "optional": true },
            },
        })
    }

    #[test]
    fn unique_object_rows_must_differ_in_some_field() {
        assert_eq!(
            list_violations(
                &links(),
                &json!(true),
                &json!([
                    { "label": "Home" },
                    { "label": "Home", "url": null },
                    { "label": "Home", "url": "https://braiins.com" },
                ])
            ),
            [Violation::new(
                r#"["list"][1]"#,
                ViolationKind::RepeatsItem { item: 1 }
            )],
        );
    }

    #[test]
    fn a_key_repeat_is_reported_at_the_key_field() {
        assert_eq!(
            list_violations(
                &links(),
                &json!(["label"]),
                &json!([
                    { "label": "Home", "url": "https://braiins.com" },
                    { "label": "Home", "url": "https://braiins.com/pool" },
                ])
            ),
            [Violation::new(
                r#"["list"][1]["label"]"#,
                ViolationKind::SameAsItem { item: 1 }
            )],
        );
    }

    #[test]
    fn a_composite_key_repeat_marks_each_of_its_fields() {
        assert_eq!(
            list_violations(
                &links(),
                &json!(["label", "url"]),
                &json!([
                    { "label": "Home", "url": "https://braiins.com" },
                    { "label": "Home", "url": "https://braiins.com/pool" },
                    { "label": "Home", "url": "https://braiins.com" },
                ])
            ),
            [
                Violation::new(
                    r#"["list"][2]["label"]"#,
                    ViolationKind::SameAsItem { item: 1 }
                ),
                Violation::new(
                    r#"["list"][2]["url"]"#,
                    ViolationKind::SameAsItem { item: 1 }
                ),
            ],
        );
    }

    #[test]
    fn unset_key_fields_match_each_other() {
        assert_eq!(
            list_violations(
                &links(),
                &json!(["url"]),
                &json!([{ "label": "Home" }, { "label": "Pool" }])
            ),
            [Violation::new(
                r#"["list"][1]["url"]"#,
                ViolationKind::SameAsItem { item: 1 }
            )],
        );
    }

    fn symbol_violations(symbol: &str) -> Vec<Violation> {
        let fields: IndexMap<ParamKey, ParamDefinition> = serde_json::from_value(json!({
            "symbol": { "name": "Symbol", "type": "string", "min_length": 2, "max_length": 4 },
        }))
        .expect("BUG: the symbol schema parses");
        let values = Values::from([(
            "symbol".to_owned(),
            Ok(ParamValue::String(symbol.to_owned())),
        )]);
        violations(validate_values(&fields, &values, MissingValues::Default))
    }

    #[test]
    fn a_string_outside_its_length_bounds_is_refused() {
        assert_eq!(
            symbol_violations("B"),
            [Violation::new(
                r#"["symbol"]"#,
                ViolationKind::TooFewCharacters { min: 2 }
            )],
        );
        assert_eq!(
            symbol_violations("BTCUSD"),
            [Violation::new(
                r#"["symbol"]"#,
                ViolationKind::TooManyCharacters { max: 4 }
            )],
        );
    }

    #[test]
    fn a_string_length_counts_characters_not_bytes() {
        for symbol in ["čřžš", "🚀🚀"] {
            let found = symbol_violations(symbol);
            assert!(
                found.is_empty(),
                "{symbol:?} is {} characters in {} bytes: {found:?}",
                symbol.chars().count(),
                symbol.len()
            );
        }
    }

    #[test]
    fn the_byte_cap_is_reported_before_the_length_bounds() {
        assert_eq!(
            symbol_violations(&"x".repeat(MAX_PARAM_STRING_LENGTH + 1)),
            [Violation::new(
                r#"["symbol"]"#,
                ViolationKind::TooManyBytes {
                    max: MAX_PARAM_STRING_LENGTH
                }
            )],
        );
    }

    #[test]
    fn empty_text_is_no_value_where_one_is_required() {
        let fields: IndexMap<ParamKey, ParamDefinition> = serde_json::from_value(json!({
            "name": { "name": "N", "type": "string", "default_value": "x" },
            "tags": { "name": "T", "type": "array", "items": { "type": "string" }, "max_items": 3 },
            "links": {
                "name": "L",
                "type": "array",
                "max_items": 3,
                "items": {
                    "type": "object",
                    "fields": {
                        "label": { "name": "Label", "type": "string" },
                        "url": { "name": "URL", "type": "string", "optional": true },
                    },
                },
            },
        }))
        .expect("BUG: the schema parses");
        let value = |json: serde_json::Value| {
            Ok(ParamValue::try_from(&json).expect("BUG: the value converts"))
        };
        let values = Values::from([
            ("name".to_owned(), value(json!(""))),
            ("tags".to_owned(), value(json!([""]))),
            (
                "links".to_owned(),
                value(json!([{ "label": "", "url": "" }])),
            ),
        ]);
        assert_eq!(
            violations(validate_values(&fields, &values, MissingValues::Default)),
            [
                Violation::new(r#"["links"][0]["label"]"#, ViolationKind::Required),
                Violation::new(r#"["name"]"#, ViolationKind::Required),
                Violation::new(r#"["tags"][0]"#, ViolationKind::Required),
            ],
        );
    }

    #[test]
    fn empty_text_stays_a_value_where_none_is_required() {
        let fields: IndexMap<ParamKey, ParamDefinition> = serde_json::from_value(json!({
            "note": { "name": "N", "type": "string", "optional": true },
        }))
        .expect("BUG: the schema parses");
        let values = Values::from([("note".to_owned(), Ok(ParamValue::String(String::new())))]);
        let typed = validate_values(&fields, &values, MissingValues::Default)
            .expect("BUG: an optional param may be empty");
        assert_eq!(typed["note"], ParamValue::String(String::new()));
    }

    fn ratio(value: i32) -> Result<BTreeMap<ParamKey, ParamValue>, Vec<Violation>> {
        let fields: IndexMap<ParamKey, ParamDefinition> = serde_json::from_value(json!({
            "ratio": { "name": "Ratio", "type": "double", "max": 2.0, "default_value": 1.0 },
        }))
        .expect("BUG: the ratio schema parses");
        let values = Values::from([("ratio".to_owned(), Ok(ParamValue::Integer(value)))]);
        validate_values(&fields, &values, MissingValues::Default)
    }

    #[test]
    fn a_whole_number_for_a_double_reads_as_a_double() {
        let typed = ratio(2).expect("BUG: 2 is within the double's bounds");
        assert_eq!(typed["ratio"], ParamValue::Double(2.0));
    }

    #[test]
    fn a_whole_number_for_a_double_meets_the_double_bounds() {
        assert_eq!(
            violations(ratio(3)),
            [Violation::new(
                r#"["ratio"]"#,
                ViolationKind::AboveMax(Bound::Double(2.0))
            )],
        );
    }
}
