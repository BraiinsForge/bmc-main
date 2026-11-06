// Copyright (C) 2025  Braiins Systems s.r.o.
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

use bmc_field_schema::{UnreadableValue, ViolationKind};
use bmc_grpc::google::rpc::BadRequest;
use bmc_grpc::google::rpc::bad_request::FieldViolation;
use bmc_grpc::web::NumberFormat;
use bmc_intl::{Intl, format_message};
use buffa::{EnumValue, MessageName as _};
use chrono::NaiveTime;
use connectrpc::{ConnectError, ErrorDetail};
use tracing::{error, warn};

use crate::BmcManager;
use crate::manager::BmcState;

pub type ParseOutput<T> = (Option<T>, FieldViolations);

/// An error a call can be refused with in the caller's language.
pub(crate) trait ToStatus {
    /// Detail the status leaves out is the implementation's to log, unless its source already did.
    fn to_status(&self, intl: &Intl) -> ConnectError;
}

/// `?` for a `ToStatus` error.
/// A `From` impl cannot do it, as the status needs the request's `Intl`.
pub(crate) trait StatusExt<T> {
    fn or_status(self, intl: &Intl) -> Result<T, ConnectError>;
}

impl<T, E: ToStatus> StatusExt<T> for Result<T, E> {
    fn or_status(self, intl: &Intl) -> Result<T, ConnectError> {
        self.map_err(|error| error.to_status(intl))
    }
}

/// The device's setup state, when `allows` lets the call run in it.
/// Otherwise the call is refused with `refusal`, and the state it met is logged.
pub(crate) async fn require_state(
    manager: &impl BmcManager,
    allows: fn(BmcState) -> bool,
    refusal: impl FnOnce() -> String,
) -> Result<BmcState, ConnectError> {
    let state = manager
        .network_manager()
        .provisioning()
        .device_state()
        .await;
    if allows(state) {
        return Ok(state);
    }
    warn!("Call refused in device state {state}");
    Err(ConnectError::failed_precondition(refusal()))
}

/// A failure that should never happen: `cause` goes to the log and the user gets one sentence.
pub(crate) fn internal_error(intl: &Intl, cause: &str) -> ConnectError {
    error!("BUG: {cause}");
    ConnectError::internal(format_message!(
        intl,
        default_message: "The device hit an internal error.",
        description: "Error for a failure that should never happen; the details are in the device's log",
    ))
}

/// A parsed value that is absent even though validation reported no violation for it.
pub(crate) fn unchecked_field(intl: &Intl) -> ConnectError {
    internal_error(intl, "a field passed validation without a value")
}

/// A violation of `field`, named by its wire name.
pub(crate) fn violation(
    field: impl Into<String>,
    description: impl Into<String>,
) -> FieldViolation {
    FieldViolation {
        field: field.into(),
        description: description.into(),
        ..Default::default()
    }
}

pub(crate) fn missing_value(intl: &Intl) -> String {
    format_message!(
        intl,
        default_message: "Missing value!",
        description: "Error under a required form field left empty",
    )
}

fn unreadable_message(intl: &Intl, unreadable: &UnreadableValue) -> String {
    match unreadable {
        UnreadableValue::KindUnset { at } if at.is_empty() => format_message!(
            intl,
            default_message: "This value has no type!",
            description: "Error on a widget setting sent without saying what kind of value it is",
        ),
        UnreadableValue::KindUnset { at } => format_message!(
            intl,
            default_message: "The value at {at} has no type!",
            description: "Error on a widget setting's nested value sent without its kind; at is its position, such as [3]",
            values: { at: at.as_str() },
        ),
        UnreadableValue::InvalidFieldKey { at, key } if at.is_empty() => format_message!(
            intl,
            default_message: "Invalid field name: {key}!",
            description: "Error on a widget setting with a malformed field; key is the field's name",
            values: { key: key.as_str() },
        ),
        UnreadableValue::InvalidFieldKey { at, key } => format_message!(
            intl,
            default_message: "Invalid field name at {at}: {key}!",
            description: "Error on a widget setting's nested value with a malformed field; key is the field's name, at its position",
            values: { key: key.as_str(), at: at.as_str() },
        ),
        UnreadableValue::Conversion(error) => violation_message(intl, &error.into()),
    }
}

/// `kind` in the request's language. Its `Display` is the same English.
#[expect(clippy::too_many_lines, reason = "one message per violation kind")]
pub(crate) fn violation_message(intl: &Intl, kind: &ViolationKind) -> String {
    fn whole(count: usize) -> u64 {
        u64::try_from(count).expect("BUG: a schema's count fits in u64")
    }

    match kind {
        ViolationKind::Required => missing_value(intl),
        ViolationKind::Unreadable(unreadable) => unreadable_message(intl, unreadable),
        ViolationKind::UnknownParam => format_message!(
            intl,
            default_message: "Unknown setting!",
            description: "Error for a widget setting the widget does not have",
        ),
        ViolationKind::UnknownField => format_message!(
            intl,
            default_message: "Unknown field!",
            description: "Error on a field a widget list row does not have",
        ),
        ViolationKind::NotAList => format_message!(
            intl,
            default_message: "Must be a list",
            description: "Error under a widget list setting",
        ),
        ViolationKind::NotAnObject => format_message!(
            intl,
            default_message: "Must be an object",
            description: "Error on a widget list row; an object is a set of fields",
        ),
        ViolationKind::NotText => format_message!(
            intl,
            default_message: "Must be text",
            description: "Error under a text widget setting",
        ),
        ViolationKind::NotWholeNumber => format_message!(
            intl,
            default_message: "Must be a whole number",
            description: "Error under a whole-number widget setting",
        ),
        ViolationKind::NotNumber => format_message!(
            intl,
            default_message: "Must be a number",
            description: "Error under a numeric widget setting",
        ),
        ViolationKind::NotTrueOrFalse => format_message!(
            intl,
            default_message: "Must be true or false",
            description: "Error under an on or off widget setting",
        ),
        ViolationKind::NotTimezone => format_message!(
            intl,
            default_message: "Must be a timezone",
            description: "Error under a timezone widget setting",
        ),
        ViolationKind::InvalidTimezone => format_message!(
            intl,
            default_message: "Must be a valid timezone",
            description: "Error under a timezone widget setting naming an unknown one",
        ),
        ViolationKind::NotFinite => format_message!(
            intl,
            default_message: "Must be a finite number",
            description: "Error under a numeric widget setting holding infinity",
        ),
        ViolationKind::NotListed => format_message!(
            intl,
            default_message: "Must be one of the listed options",
            description: "Error under a widget setting with fixed choices",
        ),
        ViolationKind::TooFewItems { min } => format_message!(
            intl,
            default_message: "{count, plural, one {Must have at least {count} item} other {Must have at least {count} items}}",
            description: "Error under a widget list setting; count is the fewest rows allowed",
            values: { count: whole(*min) },
        ),
        ViolationKind::TooManyItems { max } => format_message!(
            intl,
            default_message: "{count, plural, one {Must have at most {count} item} other {Must have at most {count} items}}",
            description: "Error under a widget list setting; count is the most rows allowed",
            values: { count: whole(*max) },
        ),
        ViolationKind::TooManyBytes { max } => format_message!(
            intl,
            default_message: "Must be at most {max} bytes",
            description: "Error under a widget text setting; max is a whole number",
            values: { max: whole(*max) },
        ),
        ViolationKind::TooFewCharacters { min } => format_message!(
            intl,
            default_message: "{count, plural, one {Must be at least {count} character} other {Must be at least {count} characters}}",
            description: "Error under a widget text setting; count is the shortest allowed",
            values: { count: whole(*min) },
        ),
        ViolationKind::TooManyCharacters { max } => format_message!(
            intl,
            default_message: "{count, plural, one {Must be at most {count} character} other {Must be at most {count} characters}}",
            description: "Error under a widget text setting; count is the longest allowed",
            values: { count: whole(*max) },
        ),
        ViolationKind::BelowMin(min) => format_message!(
            intl,
            default_message: "Must be at least {min}",
            description: "Error under a numeric widget setting; min is a number",
            values: { min: min.to_string() },
        ),
        ViolationKind::AboveMax(max) => format_message!(
            intl,
            default_message: "Must be at most {max}",
            description: "Error under a numeric widget setting; max is a number",
            values: { max: max.to_string() },
        ),
        ViolationKind::RepeatsItem { item } => format_message!(
            intl,
            default_message: "Repeats item {position}!",
            description: "Error on a widget list row; position is the earlier row's number",
            values: { position: whole(*item) },
        ),
        ViolationKind::SameAsItem { item } => format_message!(
            intl,
            default_message: "Same as item {position}!",
            description: "Error on a widget list row's field; position is the earlier row's number",
            values: { position: whole(*item) },
        ),
    }
}

/// `error` carrying `violations` as a `google.rpc.BadRequest` detail.
pub(crate) fn with_violations(
    error: ConnectError,
    violations: Vec<FieldViolation>,
) -> ConnectError {
    let bad_request = BadRequest {
        field_violations: violations,
    };
    error.with_detail(ErrorDetail::from_message(
        BadRequest::FULL_NAME,
        &bad_request,
    ))
}

/// The `InvalidArgument` of a refused form, with the reason for each field.
pub(crate) fn bad_request(intl: &Intl, field_violations: Vec<FieldViolation>) -> ConnectError {
    let message = format_message!(
        intl,
        default_message: "Some of the fields are invalid.",
        description: "Error above a form the device refused, with the reasons under its fields",
    );
    with_violations(ConnectError::invalid_argument(message), field_violations)
}

/// The `InvalidArgument` of one wrong field.
pub(crate) fn invalid_field(
    intl: &Intl,
    field: &str,
    description: impl Into<String>,
) -> ConnectError {
    bad_request(intl, vec![violation(field, description)])
}

#[derive(Debug)]
pub struct FieldViolations(Vec<FieldViolation>);

impl FieldViolations {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, field: impl Into<String>, description: impl Into<String>) {
        self.0.push(violation(field, description));
    }

    pub fn extend(&mut self, other: Self) {
        self.0.extend(other.0);
    }
}

impl From<FieldViolations> for Vec<FieldViolation> {
    fn from(value: FieldViolations) -> Self {
        value.0
    }
}

pub fn naive_time_to_hhmm(time: NaiveTime) -> String {
    time.format("%H:%M").to_string()
}

pub fn parse_hhmm_to_naive_time(intl: &Intl, input: &str) -> Result<NaiveTime, ConnectError> {
    NaiveTime::parse_from_str(input, "%H:%M").map_err(|e| {
        warn!("Failed to parse Time in format HH:MM, error: {}", e);
        ConnectError::invalid_argument(format_message!(
            intl,
            default_message: "Invalid time format!",
            description: "Error when a time of day, such as when night mode starts, is not in hours and minutes",
        ))
    })
}

pub(crate) fn try_from_number_format(
    intl: &Intl,
    value: EnumValue<NumberFormat>,
) -> Result<bmc_shared_utils::number_format::NumberFormat, FieldViolation> {
    match value.as_known().unwrap_or(NumberFormat::Unspecified) {
        NumberFormat::Unspecified => Err(violation(
            "number_format",
            format_message!(
                intl,
                default_message: "Choose a number format",
                description: "Error under the number format field when nothing is chosen",
            ),
        )),
        NumberFormat::SpaceGroupCommaDecimal => {
            Ok(bmc_shared_utils::number_format::NumberFormat::SpaceGroupCommaDecimal)
        }
        NumberFormat::CommaGroupDotDecimal => {
            Ok(bmc_shared_utils::number_format::NumberFormat::CommaGroupDotDecimal)
        }
        NumberFormat::DotGroupCommaDecimal => {
            Ok(bmc_shared_utils::number_format::NumberFormat::DotGroupCommaDecimal)
        }
        NumberFormat::SpaceGroupDotDecimal => {
            Ok(bmc_shared_utils::number_format::NumberFormat::SpaceGroupDotDecimal)
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_field_schema::{Bound, ParamValueConversionError};
    use bmc_intl::Language;

    use super::*;

    /// No wildcard: a new kind stops compiling here, the cue to add it to the samples below.
    fn name_every_kind(kind: &ViolationKind) {
        match kind {
            ViolationKind::Required
            | ViolationKind::Unreadable(
                UnreadableValue::KindUnset { .. }
                | UnreadableValue::InvalidFieldKey { .. }
                | UnreadableValue::Conversion(
                    ParamValueConversionError::InvalidObjectKey(_)
                    | ParamValueConversionError::UnrepresentableNumber
                    | ParamValueConversionError::NonFiniteNumber
                    | ParamValueConversionError::StringTooLong { .. },
                ),
            )
            | ViolationKind::UnknownParam
            | ViolationKind::UnknownField
            | ViolationKind::NotAList
            | ViolationKind::NotAnObject
            | ViolationKind::NotText
            | ViolationKind::NotWholeNumber
            | ViolationKind::NotNumber
            | ViolationKind::NotTrueOrFalse
            | ViolationKind::NotTimezone
            | ViolationKind::InvalidTimezone
            | ViolationKind::NotFinite
            | ViolationKind::NotListed
            | ViolationKind::TooFewItems { .. }
            | ViolationKind::TooManyItems { .. }
            | ViolationKind::TooManyBytes { .. }
            | ViolationKind::TooFewCharacters { .. }
            | ViolationKind::TooManyCharacters { .. }
            | ViolationKind::BelowMin(_)
            | ViolationKind::AboveMax(_)
            | ViolationKind::RepeatsItem { .. }
            | ViolationKind::SameAsItem { .. } => {}
        }
    }

    #[test]
    fn every_violation_kind_words_its_display_in_english() {
        let intl = bmc_intl::shared_formatter(Language::EN);
        for kind in [
            ViolationKind::Required,
            ViolationKind::Unreadable(UnreadableValue::KindUnset { at: String::new() }),
            ViolationKind::Unreadable(UnreadableValue::KindUnset {
                at: "[1]".to_owned(),
            }),
            ViolationKind::Unreadable(UnreadableValue::InvalidFieldKey {
                at: String::new(),
                key: "1".to_owned(),
            }),
            ViolationKind::Unreadable(UnreadableValue::InvalidFieldKey {
                at: "[1]".to_owned(),
                key: "2".to_owned(),
            }),
            ViolationKind::Unreadable(UnreadableValue::Conversion(
                ParamValueConversionError::InvalidObjectKey("1".to_owned()),
            )),
            ViolationKind::Unreadable(UnreadableValue::Conversion(
                ParamValueConversionError::UnrepresentableNumber,
            )),
            ViolationKind::Unreadable(UnreadableValue::Conversion(
                ParamValueConversionError::NonFiniteNumber,
            )),
            ViolationKind::Unreadable(UnreadableValue::Conversion(
                ParamValueConversionError::StringTooLong {
                    len: 1025,
                    max: 1024,
                },
            )),
            ViolationKind::UnknownParam,
            ViolationKind::UnknownField,
            ViolationKind::NotAList,
            ViolationKind::NotAnObject,
            ViolationKind::NotText,
            ViolationKind::NotWholeNumber,
            ViolationKind::NotNumber,
            ViolationKind::NotTrueOrFalse,
            ViolationKind::NotTimezone,
            ViolationKind::InvalidTimezone,
            ViolationKind::NotFinite,
            ViolationKind::NotListed,
            ViolationKind::TooFewItems { min: 1 },
            ViolationKind::TooFewItems { min: 2 },
            ViolationKind::TooManyItems { max: 1 },
            ViolationKind::TooManyItems { max: 2 },
            ViolationKind::TooManyBytes { max: 1024 },
            ViolationKind::TooFewCharacters { min: 1 },
            ViolationKind::TooFewCharacters { min: 2 },
            ViolationKind::TooManyCharacters { max: 1 },
            ViolationKind::TooManyCharacters { max: 2 },
            ViolationKind::BelowMin(Bound::Integer(-3)),
            ViolationKind::BelowMin(Bound::Double(0.5)),
            ViolationKind::AboveMax(Bound::Integer(1234)),
            ViolationKind::AboveMax(Bound::Double(2.5)),
            ViolationKind::RepeatsItem { item: 1 },
            ViolationKind::SameAsItem { item: 2 },
        ] {
            name_every_kind(&kind);
            assert_eq!(violation_message(&intl, &kind), kind.to_string());
        }
    }
}
