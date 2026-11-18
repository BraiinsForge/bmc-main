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

pub(crate) fn version_undetected(intl: &Intl) -> ConnectError {
    ConnectError::internal(format_message!(
        intl,
        default_message: "Failed to detect current version!",
        description: "Error when the device cannot tell which firmware version it runs",
    ))
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
    // Paths only: a violation's description can quote the submitted value.
    let fields: Vec<&str> = field_violations.iter().map(|v| v.field.as_str()).collect();
    warn!(?fields, "Request refused with field violations");
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
    use bmc_field_schema::{
        Bound, BoundDiscriminants as BoundName, ParamValueConversionError,
        ParamValueConversionErrorDiscriminants as ConversionName,
        UnreadableValueDiscriminants as UnreadableName, ViolationKindDiscriminants as KindName,
    };
    use bmc_intl::Language;

    use super::*;
    use crate::web::grpc::tests::{assert_worded, sampled, status_text};

    /// A sample for each way a kind is worded,
    /// such as a count's singular and plural.
    fn every_kind() -> Vec<ViolationKind> {
        sampled(|name| match name {
            KindName::Required => vec![ViolationKind::Required],
            KindName::Unreadable => sampled(unreadable_samples)
                .into_iter()
                .map(ViolationKind::Unreadable)
                .collect(),
            KindName::UnknownParam => vec![ViolationKind::UnknownParam],
            KindName::UnknownField => vec![ViolationKind::UnknownField],
            KindName::NotAList => vec![ViolationKind::NotAList],
            KindName::NotAnObject => vec![ViolationKind::NotAnObject],
            KindName::NotText => vec![ViolationKind::NotText],
            KindName::NotWholeNumber => vec![ViolationKind::NotWholeNumber],
            KindName::NotNumber => vec![ViolationKind::NotNumber],
            KindName::NotTrueOrFalse => vec![ViolationKind::NotTrueOrFalse],
            KindName::NotTimezone => vec![ViolationKind::NotTimezone],
            KindName::InvalidTimezone => vec![ViolationKind::InvalidTimezone],
            KindName::NotFinite => vec![ViolationKind::NotFinite],
            KindName::NotListed => vec![ViolationKind::NotListed],
            KindName::TooFewItems => vec![
                ViolationKind::TooFewItems { min: 1 },
                ViolationKind::TooFewItems { min: 2 },
            ],
            KindName::TooManyItems => vec![
                ViolationKind::TooManyItems { max: 1 },
                ViolationKind::TooManyItems { max: 2 },
            ],
            KindName::TooManyBytes => vec![ViolationKind::TooManyBytes { max: 1024 }],
            KindName::TooFewCharacters => vec![
                ViolationKind::TooFewCharacters { min: 1 },
                ViolationKind::TooFewCharacters { min: 2 },
            ],
            KindName::TooManyCharacters => vec![
                ViolationKind::TooManyCharacters { max: 1 },
                ViolationKind::TooManyCharacters { max: 2 },
            ],
            KindName::BelowMin => bound_samples(-3, 0.5)
                .into_iter()
                .map(ViolationKind::BelowMin)
                .collect(),
            KindName::AboveMax => bound_samples(1234, 2.5)
                .into_iter()
                .map(ViolationKind::AboveMax)
                .collect(),
            KindName::RepeatsItem => vec![ViolationKind::RepeatsItem { item: 1 }],
            KindName::SameAsItem => vec![ViolationKind::SameAsItem { item: 2 }],
        })
    }

    fn bound_samples(integer: i32, double: f64) -> Vec<Bound> {
        sampled(|name| {
            vec![match name {
                BoundName::Integer => Bound::Integer(integer),
                BoundName::Double => Bound::Double(double),
            }]
        })
    }

    /// `KindUnset` and `InvalidFieldKey` read one way
    /// for the value itself and another for a part nested in it.
    fn unreadable_samples(name: UnreadableName) -> Vec<UnreadableValue> {
        match name {
            UnreadableName::KindUnset => vec![
                UnreadableValue::KindUnset { at: String::new() },
                UnreadableValue::KindUnset {
                    at: "[1]".to_owned(),
                },
            ],
            UnreadableName::InvalidFieldKey => vec![
                UnreadableValue::InvalidFieldKey {
                    at: String::new(),
                    key: "1".to_owned(),
                },
                UnreadableValue::InvalidFieldKey {
                    at: "[1]".to_owned(),
                    key: "2".to_owned(),
                },
            ],
            UnreadableName::Conversion => sampled(conversion_samples)
                .into_iter()
                .map(UnreadableValue::Conversion)
                .collect(),
        }
    }

    fn conversion_samples(name: ConversionName) -> Vec<ParamValueConversionError> {
        match name {
            ConversionName::InvalidObjectKey => {
                vec![ParamValueConversionError::InvalidObjectKey("1".to_owned())]
            }
            ConversionName::UnrepresentableNumber => {
                vec![ParamValueConversionError::UnrepresentableNumber]
            }
            ConversionName::NonFiniteNumber => vec![ParamValueConversionError::NonFiniteNumber],
            ConversionName::StringTooLong => vec![ParamValueConversionError::StringTooLong {
                len: 1025,
                max: 1024,
            }],
        }
    }

    #[test]
    fn every_violation_kind_words_its_display_in_english() {
        let intl = bmc_intl::shared_formatter(Language::EN);
        for kind in every_kind() {
            assert_eq!(violation_message(&intl, &kind), kind.to_string());
        }
    }

    #[test]
    fn every_violation_kind_is_worded() {
        assert_worded(|intl| {
            every_kind()
                .iter()
                // Reads as another kind's sentence, which the list already holds.
                .filter(|kind| {
                    !matches!(
                        kind,
                        ViolationKind::Unreadable(UnreadableValue::Conversion(_))
                    )
                })
                .map(|kind| violation_message(intl, kind))
                .collect()
        });
    }

    #[test]
    fn a_refused_form_is_worded_above_its_fields() {
        assert_worded(|intl| vec![status_text(&bad_request(intl, vec![]))]);
    }

    #[test]
    fn every_shared_refusal_is_worded() {
        assert_worded(|intl| {
            vec![
                status_text(&internal_error(intl, "1")),
                status_text(&version_undetected(intl)),
                status_text(&parse_hhmm_to_naive_time(intl, "1").expect_err("BUG: no time")),
                missing_value(intl),
            ]
        });
    }

    #[test]
    fn an_unchecked_field_is_worded() {
        // Words as `internal_error`, so it needs a list of its own.
        assert_worded(|intl| vec![status_text(&unchecked_field(intl))]);
    }

    #[test]
    fn a_refused_form_logs_its_field_paths_but_not_their_descriptions() {
        let (logs, _capturing) = crate::test_support::LogCapture::capture();
        let intl = bmc_intl::shared_formatter(Language::EN);
        bad_request(
            &intl,
            vec![violation("fieldValues.password", "Too short: hunter2")],
        );
        let text = logs.text();
        assert!(text.contains("fieldValues.password"), "got: {text}");
        assert!(
            !text.contains("hunter2"),
            "a description reached the log: {text}"
        );
    }
}
