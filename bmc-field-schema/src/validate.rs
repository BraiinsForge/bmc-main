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
    ArrayParam, DoubleParam, IntegerParam, ItemKind, ItemShape, MAX_PARAM_STRING_LENGTH,
    ObjectParam, ParamDefinition, ParamKey, ParamKind, ParamValue, Scalar, Shape, StringParam,
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

fn validate_list(
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
    let typed: Vec<ParamValue> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            validate_item(&format!("{path}[{i}]"), &array.items, item, violations)
        })
        .collect();
    (violations.len() == before).then_some(ParamValue::List(typed))
}

/// Unlike an optional field, an item is never null.
pub(crate) fn validate_item(
    path: &str,
    kind: &ItemKind,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    if matches!(value, ParamValue::Null) {
        violations.push(Violation::new(path, "Value is required"));
        return None;
    }
    match kind.shape() {
        ItemShape::Scalar(scalar) => validate_scalar(path, scalar, value, violations),
        ItemShape::Object(object) => validate_object(path, object, value, violations),
    }
}

/// A field the row omits reads as null, which only an optional field accepts.
fn validate_object(
    path: &str,
    object: &ObjectParam,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
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
    (violations.len() == before).then_some(ParamValue::Object(typed))
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
