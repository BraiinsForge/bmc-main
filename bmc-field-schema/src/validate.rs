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
    ArrayParam, DoubleParam, IntegerParam, ItemShape, MAX_PARAM_STRING_LENGTH, ObjectParam,
    ParamDefinition, ParamKey, ParamKind, ParamValue, Scalar, Shape, StringParam, UniqueItems,
    f64_canonical_bits,
};

/// What a schema key the input omits turns into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingValues {
    /// The field's default, or `Null` for an optional field without one.
    Default,
    /// A "Value is required" violation.
    Reject,
}

/// A value that failed its schema, addressed relative to the value map:
/// `["key"]`, `["key"][i]` for a list item, `["key"][i]["field"]` for an object item's field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub path: String,
    pub message: String,
}

impl Violation {
    fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

/// Validate wire values against a field schema and project them
/// onto typed values, collecting every violation before returning.
///
/// Each present value is as the wire edge decoded it:
/// an `Err` is a value the edge could not read, reported at its key as-is.
pub fn validate_values(
    fields: &IndexMap<ParamKey, ParamDefinition>,
    values: &BTreeMap<String, Result<ParamValue, String>>,
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
                MissingValues::Reject => violations.push(Violation::new(path, "Value is required")),
            }
            continue;
        };
        let value = match value {
            Ok(value) => value,
            Err(message) => {
                violations.push(Violation::new(path, message.clone()));
                continue;
            }
        };

        if matches!(value, ParamValue::Null) {
            if def.is_optional {
                typed.insert(key.clone(), ParamValue::Null);
            } else {
                violations.push(Violation::new(path, "Value is required"));
            }
            continue;
        }

        if let Some(value) = validate_value(&path, &def.kind, value, &mut violations) {
            typed.insert(key.clone(), value);
        }
    }

    for key in values.keys() {
        if !fields.contains_key(key.as_str()) {
            violations.push(Violation::new(key_path(key), "Unknown param"));
        }
    }

    if violations.is_empty() {
        Ok(typed)
    } else {
        Err(violations)
    }
}

/// Debug-formats the key, so a client-supplied key
/// with quotes, newlines, etc. cannot break the path.
fn key_path(key: &str) -> String {
    format!("[{key:?}]")
}

fn type_mismatch_message(scalar: Scalar<'_>) -> &'static str {
    match scalar {
        Scalar::String(_) => "Must be text",
        Scalar::Integer(_) => "Must be a whole number",
        Scalar::Double(_) => "Must be a number",
        Scalar::Boolean(_) => "Must be true or false",
        Scalar::Timezone(_) => "Must be a timezone",
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

pub(crate) fn validate_list(
    path: &str,
    array: &ArrayParam,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    let ParamValue::List(items) = value else {
        violations.push(Violation::new(path, "Must be a list"));
        return None;
    };
    let before = violations.len();
    if items.len() < array.min_items {
        violations.push(Violation::new(
            path,
            format!("Must have at least {}", item_count(array.min_items)),
        ));
    }
    if items.len() > array.max_items {
        // Past the bound, item checks add nothing but one violation per item to the response.
        violations.push(Violation::new(
            path,
            format!("Must have at most {}", item_count(array.max_items)),
        ));
        return None;
    }
    let typed = match array.items.shape() {
        ItemShape::Scalar(scalar) => {
            let items = validate_items(path, items, violations, |path, item, violations| {
                validate_scalar(path, scalar, item, violations)
            })?;
            match &array.unique_items {
                UniqueItems::Off => {}
                UniqueItems::Whole => push_repeats(path, &items, None, violations),
                UniqueItems::By(_) => {
                    panic!("BUG: manifest load refuses unique_items keys on scalar items")
                }
            }
            items
        }
        ItemShape::Object(object) => {
            let rows = validate_items(path, items, violations, |path, item, violations| {
                validate_object(path, object, item, violations)
            })?;
            match &array.unique_items {
                UniqueItems::Off => {}
                UniqueItems::Whole => push_repeats(path, &rows, None, violations),
                UniqueItems::By(keys) => {
                    let identities: Vec<_> = rows.iter().map(|row| key_fields(row, keys)).collect();
                    push_repeats(path, &identities, Some(keys), violations);
                }
            }
            rows.into_iter().map(ParamValue::Object).collect()
        }
    };
    (violations.len() == before).then_some(ParamValue::List(typed))
}

/// Every item typed, or `None` if any item has a violation.
/// Unlike an optional field, an item is never null.
fn validate_items<T>(
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
            if matches!(item, ParamValue::Null) {
                violations.push(Violation::new(path, "Value is required"));
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
        violations.push(Violation::new(path, "Must be an object"));
        return None;
    };
    let before = violations.len();
    let mut typed = BTreeMap::new();
    for (key, field) in &object.fields {
        let field_path = format!("{path}{}", key_path(key.as_str()));
        let value = fields.get(key).unwrap_or(&ParamValue::Null);
        if matches!(value, ParamValue::Null) {
            if field.is_optional {
                typed.insert(key.clone(), ParamValue::Null);
            } else {
                violations.push(Violation::new(field_path, "Value is required"));
            }
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
                "Unknown field",
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
                format!("Repeats item {item}"),
            )),
            Some(keys) => violations.extend(keys.iter().map(|key| {
                Violation::new(
                    format!("{path}[{i}]{}", key_path(key.as_str())),
                    format!("Same as item {item}"),
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
                "BUG: manifest load lets unique_items name only declared fields, \
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

pub(crate) fn validate_scalar(
    path: &str,
    scalar: Scalar<'_>,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    match (scalar, value) {
        (Scalar::String(StringParam { enum_values, .. }), ParamValue::String(s)) => {
            if s.len() > MAX_PARAM_STRING_LENGTH {
                violations.push(Violation::new(
                    path,
                    format!("Must be at most {MAX_PARAM_STRING_LENGTH} bytes"),
                ));
                return None;
            }
            if !enum_values.is_empty() && !enum_values.iter().any(|o| &o.value == s) {
                violations.push(Violation::new(path, "Must be one of the listed options"));
                return None;
            }
            Some(ParamValue::String(s.clone()))
        }
        (Scalar::Timezone(_), ParamValue::String(s)) => {
            if Timezone::lookup(s).is_none() {
                violations.push(Violation::new(path, "Must be a valid timezone"));
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
            if let Some(lo) = min
                && i < lo
            {
                violations.push(Violation::new(path, format!("Must be at least {lo}")));
                ok = false;
            }
            if let Some(hi) = max
                && i > hi
            {
                violations.push(Violation::new(path, format!("Must be at most {hi}")));
                ok = false;
            }
            if !enum_values.is_empty() && !enum_values.iter().any(|o| o.value == *i) {
                violations.push(Violation::new(path, "Must be one of the listed options"));
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
            violations.push(Violation::new(path, type_mismatch_message(other_kind)));
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
        violations.push(Violation::new(path, "Must be a finite number"));
        return None;
    }
    let mut ok = true;
    if let Some(lo) = param.min
        && d < lo
    {
        violations.push(Violation::new(path, format!("Must be at least {lo}")));
        ok = false;
    }
    if let Some(hi) = param.max
        && d > hi
    {
        violations.push(Violation::new(path, format!("Must be at most {hi}")));
        ok = false;
    }
    if !param.enum_values.is_empty()
        && !param
            .enum_values
            .iter()
            .any(|o| f64_canonical_bits(o.value) == f64_canonical_bits(d))
    {
        violations.push(Violation::new(path, "Must be one of the listed options"));
        ok = false;
    }
    ok.then_some(ParamValue::Double(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    type Values = BTreeMap<String, Result<ParamValue, String>>;

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
                Violation::new(r#"["list"][2]"#, "Repeats item 1"),
                Violation::new(r#"["list"][3]"#, "Repeats item 2"),
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
            [Violation::new(r#"["list"][1]"#, "Repeats item 1")],
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
            [Violation::new(r#"["list"][1]"#, "Repeats item 1")],
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
            [Violation::new(r#"["list"][1]["label"]"#, "Same as item 1")],
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
                Violation::new(r#"["list"][2]["label"]"#, "Same as item 1"),
                Violation::new(r#"["list"][2]["url"]"#, "Same as item 1"),
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
            [Violation::new(r#"["list"][1]["url"]"#, "Same as item 1")],
        );
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
            [Violation::new(r#"["ratio"]"#, "Must be at most 2")],
        );
    }
}
