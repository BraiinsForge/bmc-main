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
    DoubleParam, IntegerParam, ParamDefinition, ParamKey, ParamKind, ParamValue, StringParam,
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

/// A value that failed its schema, addressed relative to the value map: `["key"]`.
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

fn type_mismatch_message(kind: &ParamKind) -> &'static str {
    match kind {
        ParamKind::String(_) => "Must be text",
        ParamKind::Integer(_) => "Must be a whole number",
        ParamKind::Double(_) => "Must be a number",
        ParamKind::Boolean(_) => "Must be true or false",
        ParamKind::Timezone(_) => "Must be a timezone",
    }
}

fn validate_value(
    path: &str,
    kind: &ParamKind,
    value: &ParamValue,
    violations: &mut Vec<Violation>,
) -> Option<ParamValue> {
    match (kind, value) {
        (ParamKind::String(StringParam { enum_values, .. }), ParamValue::String(s)) => {
            if !enum_values.is_empty() && !enum_values.iter().any(|o| &o.value == s) {
                violations.push(Violation::new(path, "Must be one of the listed options"));
                return None;
            }
            Some(ParamValue::String(s.clone()))
        }
        (ParamKind::Timezone(_), ParamValue::String(s)) => {
            if !Timezone::list().iter().any(|tz| tz.iana() == s) {
                violations.push(Violation::new(path, "Must be a valid timezone"));
                return None;
            }
            Some(ParamValue::String(s.clone()))
        }
        (ParamKind::Boolean(_), ParamValue::Boolean(b)) => Some(ParamValue::Boolean(*b)),
        (
            ParamKind::Integer(IntegerParam {
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
        (
            ParamKind::Double(DoubleParam {
                min,
                max,
                enum_values,
                ..
            }),
            ParamValue::Double(d),
        ) => {
            if !d.is_finite() {
                violations.push(Violation::new(path, "Must be a finite number"));
                return None;
            }
            let mut ok = true;
            if let Some(lo) = min
                && d < lo
            {
                violations.push(Violation::new(path, format!("Must be at least {lo}")));
                ok = false;
            }
            if let Some(hi) = max
                && d > hi
            {
                violations.push(Violation::new(path, format!("Must be at most {hi}")));
                ok = false;
            }
            if !enum_values.is_empty()
                && !enum_values
                    .iter()
                    .any(|o| f64_canonical_bits(o.value) == f64_canonical_bits(*d))
            {
                violations.push(Violation::new(path, "Must be one of the listed options"));
                ok = false;
            }
            if ok {
                Some(ParamValue::Double(*d))
            } else {
                None
            }
        }
        (other_kind, _) => {
            violations.push(Violation::new(path, type_mismatch_message(other_kind)));
            None
        }
    }
}
