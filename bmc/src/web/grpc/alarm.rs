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

use std::collections::BTreeSet;

use bmc_grpc::web::{
    AddAlarmRequest, Alarm as AlarmProto, AlarmInfoResponse, AlarmService as GrpcAlarmService,
    ListAlarmsResponse, Off, SetAlarmEnabledRequest, SetAlarmRequest,
    SnoozeDuration as SnoozeDurationProto, SnoozeLimit as SnoozeLimitProto, SnoozeOptionsWrapper,
    Weekday,
    snooze_options_wrapper::{self, Kind as SnoozeKind},
};
use bmc_shared_time::time::WeekDay;
use buffa::EnumValue;
use buffa_types::google::protobuf::{Empty, StringValue};
use chrono::NaiveTime;
use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use std::str::FromStr;
use tap::tap::TapOptional;

use crate::{
    alarm::{
        AlarmController, AlarmData, AlarmError, AlarmId, SnoozeDuration, SnoozeLimit, SnoozeOptions,
    },
    sound::Sounds,
    web::grpc::shared::unchecked_field_violations_status,
};

use super::{
    GrpcError,
    shared::{FieldViolations, ParseOutput, naive_time_to_hhmm, with_violations},
};

pub(crate) struct AlarmService {
    alarm_controller: AlarmController,
}

impl AlarmService {
    pub(crate) fn new(alarm_controller: AlarmController) -> Self {
        Self { alarm_controller }
    }
}

/// The `InvalidArgument` naming every violated field of an alarm request.
fn bad_request(violations: FieldViolations) -> ConnectError {
    with_violations(
        ConnectError::invalid_argument(GrpcError::BadRequest.to_string()),
        violations.into(),
    )
}

impl GrpcAlarmService for AlarmService {
    async fn get_alarm_info(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<AlarmInfoResponse> {
        Response::ok(AlarmInfoResponse {
            repeat: vec![
                Weekday::Monday.into(),
                Weekday::Tuesday.into(),
                Weekday::Wednesday.into(),
                Weekday::Thursday.into(),
                Weekday::Friday.into(),
                Weekday::Saturday.into(),
                Weekday::Sunday.into(),
            ],
            name: String::new(),
            time: String::new(),
            sound_id: Sounds::GreenCandleMorning.to_string(),
            snooze_options: SnoozeOptionsWrapper {
                kind: Some(SnoozeKind::Snooze(bmc_grpc::web::SnoozeOptions {
                    duration: SnoozeDurationProto::SnoozeDuration5Minutes.into(),
                    limit: SnoozeLimitProto::SnoozeLimit3.into(),
                })),
            }
            .into(),
        })
    }

    async fn list_alarms(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListAlarmsResponse> {
        let alarms = self
            .alarm_controller
            .alarms()
            .await
            .into_iter()
            .map(Into::into)
            .collect();

        Response::ok(ListAlarmsResponse { alarms })
    }

    async fn add_alarm(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AddAlarmRequest>,
    ) -> ServiceResult<Empty> {
        let request = request.to_owned_message();

        let repeat = map_weekday_vec(request.repeat);

        let AddAlarmRequest {
            name,
            time,
            enabled,
            sound_id,
            snooze_options,
            ..
        } = request;

        let mut all_field_violations = FieldViolations::new();

        all_field_violations.extend(validate_alarm_name("name", &name));

        let (time, violations) = parse_time("time", &time);
        all_field_violations.extend(violations);

        let (sound, violations) = parse_sound("sound_id", sound_id);
        all_field_violations.extend(violations);

        let (snooze_options, violations) =
            parse_snooze_options_field("snooze_options", snooze_options.into_option());
        all_field_violations.extend(violations);

        if !all_field_violations.is_empty() {
            return Err(bad_request(all_field_violations));
        }

        let time = time.ok_or_else(unchecked_field_violations_status)?;

        let alarm = AlarmData::new(enabled, name, time, repeat, sound, snooze_options);

        self.alarm_controller
            .add_alarm(alarm)
            .await
            .map_err(Into::<ConnectError>::into)?;

        Response::ok(Empty::default())
    }

    async fn set_alarm(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetAlarmRequest>,
    ) -> ServiceResult<Empty> {
        let request = request.to_owned_message();

        let repeat = map_weekday_vec(request.repeat);

        let SetAlarmRequest {
            id,
            name,
            time,
            enabled,
            sound_id,
            snooze_options,
            ..
        } = request;

        let mut all_field_violations = FieldViolations::new();

        all_field_violations.extend(validate_alarm_name("name", &name));

        let (id, violations) = parse_alarm_id("id", &id);
        all_field_violations.extend(violations);

        let (time, violations) = parse_time("time", &time);
        all_field_violations.extend(violations);

        let (sound, violations) = parse_sound("sound_id", sound_id);
        all_field_violations.extend(violations);

        let (snooze_options, violations) =
            parse_snooze_options_field("snooze_options", snooze_options.into_option());
        all_field_violations.extend(violations);

        if !all_field_violations.is_empty() {
            return Err(bad_request(all_field_violations));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;
        let time = time.ok_or_else(unchecked_field_violations_status)?;

        let alarm = AlarmData::new_with_id(id, enabled, name, time, repeat, sound, snooze_options);

        self.alarm_controller
            .set_alarm(alarm)
            .await
            .map_err(Into::<ConnectError>::into)?;

        Response::ok(Empty::default())
    }

    async fn delete_alarm(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StringValue>,
    ) -> ServiceResult<Empty> {
        let (id, violations) = parse_alarm_id("id", request.value);

        if !violations.is_empty() {
            return Err(bad_request(violations));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        self.alarm_controller
            .remove_alarm(id)
            .await
            .map_err(Into::<ConnectError>::into)?;

        Response::ok(Empty::default())
    }

    async fn set_alarm_enabled(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetAlarmEnabledRequest>,
    ) -> ServiceResult<Empty> {
        let (id, violations) = parse_alarm_id("id", request.id);

        if !violations.is_empty() {
            return Err(bad_request(violations));
        }

        let id = id.ok_or_else(unchecked_field_violations_status)?;

        self.alarm_controller
            .set_enabled(id, request.enabled)
            .await
            .map_err(Into::<ConnectError>::into)?;

        Response::ok(Empty::default())
    }
}

impl From<AlarmData> for AlarmProto {
    fn from(value: AlarmData) -> Self {
        let AlarmData {
            enabled,
            id,
            name,
            time,
            repeat,
            sound,
            snooze_options,
        } = value;
        Self {
            enabled,
            id: id.to_string(),
            name,
            time: naive_time_to_hhmm(time),
            repeat: repeat
                .into_iter()
                .map(|weekday| map_weekday_to_proto(weekday).into())
                .collect(),
            sound: sound.map(Into::into).into(),
            snooze_options: map_snooze_options(snooze_options).into(),
        }
    }
}

fn parse_time(field: &str, value: &str) -> ParseOutput<NaiveTime> {
    let mut field_violations = FieldViolations::new();

    let maybe_time = NaiveTime::parse_from_str(value, "%H:%M").ok().tap_none(|| {
        field_violations.push(field, "Invalid time!");
    });

    (maybe_time, field_violations)
}

fn parse_sound(field: &str, value: Option<String>) -> ParseOutput<Sounds> {
    let mut field_violations = FieldViolations::new();

    let maybe_sound = value.and_then(|sound_id| {
        Sounds::from_str(&sound_id).ok().tap_none(|| {
            field_violations.push(field, "Invalid sound!");
        })
    });

    (maybe_sound, field_violations)
}

fn parse_snooze_options_field(
    field: &str,
    input: Option<SnoozeOptionsWrapper>,
) -> ParseOutput<SnoozeOptions> {
    let mut field_violations = FieldViolations::new();

    let Some(kind) = input.and_then(|wrapper| wrapper.kind) else {
        field_violations.push(field, "Missing value!");
        return (None, field_violations);
    };

    let (snooze_options, violations) = parse_snooze_options(kind);
    field_violations.extend(violations);
    (snooze_options, field_violations)
}

fn parse_snooze_options(value: snooze_options_wrapper::Kind) -> ParseOutput<SnoozeOptions> {
    let mut field_violations = FieldViolations::new();

    let maybe_snooze_options = match value {
        SnoozeKind::Snooze(snooze_options) => {
            let limit = map_snooze_limit_proto(snooze_options.limit).tap_none(|| {
                field_violations.push("limit", "Unspecified snooze limit!");
            });

            let duration = map_snooze_duration_proto(snooze_options.duration).tap_none(|| {
                field_violations.push("duration", "Unspecifield snooze duration!");
            });

            if let (Some(limit), Some(duration)) = (limit, duration) {
                Some(SnoozeOptions { limit, duration })
            } else {
                None
            }
        }
        SnoozeKind::Off(_off) => None,
    };

    (maybe_snooze_options, field_violations)
}

fn parse_alarm_id(field: &str, input: &str) -> ParseOutput<AlarmId> {
    let mut field_violations = FieldViolations::new();

    let maybe_id = AlarmId::from_str(input).ok().tap_none(|| {
        field_violations.push(field, "Invalid alarm ID!");
    });

    (maybe_id, field_violations)
}

/// Maximum operator-typed alarm name length, in UTF-8 bytes.
///
/// Matched verbatim by the wayland `next_alarm.name` arg's documented
/// cap (`bmc-widget-protocol/protocol/deck-widget.xml`) and enforced
/// belt-and-braces in the compositor's setting-broadcast relay
/// (`bmc-openwrt/src/compositor/protocol/state.rs`).
///
/// Validating here as well lets the gRPC layer reject oversize input
/// at the operator boundary with a clean `InvalidArgument` rather
/// than silently truncating it downstream.
const ALARM_NAME_MAX_BYTES: usize = 256;

fn validate_alarm_name(field: &str, value: &str) -> FieldViolations {
    let mut field_violations = FieldViolations::new();
    if value.len() > ALARM_NAME_MAX_BYTES {
        field_violations.push(
            field,
            format!(
                "Alarm name must be at most {ALARM_NAME_MAX_BYTES} bytes \
                 (got {} bytes)",
                value.len()
            ),
        );
    }
    field_violations
}

fn map_weekday_vec(value: Vec<EnumValue<Weekday>>) -> BTreeSet<WeekDay> {
    value
        .into_iter()
        .filter_map(|day| day.as_known())
        .filter(|day| *day != Weekday::Unspecified)
        .filter_map(map_weekday_from_proto)
        .collect()
}

pub(crate) fn map_weekday_from_proto(value: Weekday) -> Option<WeekDay> {
    match value {
        Weekday::Unspecified => None,
        Weekday::Monday => Some(WeekDay::Monday),
        Weekday::Tuesday => Some(WeekDay::Tuesday),
        Weekday::Wednesday => Some(WeekDay::Wednesday),
        Weekday::Thursday => Some(WeekDay::Thursday),
        Weekday::Friday => Some(WeekDay::Friday),
        Weekday::Saturday => Some(WeekDay::Saturday),
        Weekday::Sunday => Some(WeekDay::Sunday),
    }
}

pub(crate) fn map_weekday_to_proto(value: WeekDay) -> Weekday {
    match value {
        WeekDay::Monday => Weekday::Monday,
        WeekDay::Tuesday => Weekday::Tuesday,
        WeekDay::Wednesday => Weekday::Wednesday,
        WeekDay::Thursday => Weekday::Thursday,
        WeekDay::Friday => Weekday::Friday,
        WeekDay::Saturday => Weekday::Saturday,
        WeekDay::Sunday => Weekday::Sunday,
    }
}

fn map_snooze_options(value: Option<SnoozeOptions>) -> SnoozeOptionsWrapper {
    SnoozeOptionsWrapper {
        kind: Some(match value {
            Some(snooze_options) => {
                snooze_options_wrapper::Kind::Snooze(bmc_grpc::web::SnoozeOptions {
                    duration: SnoozeDurationProto::from(snooze_options.duration).into(),
                    limit: SnoozeLimitProto::from(snooze_options.limit).into(),
                })
            }
            None => snooze_options_wrapper::Kind::Off(Off::default()),
        }),
    }
}

fn map_snooze_limit_proto(value: EnumValue<SnoozeLimitProto>) -> Option<SnoozeLimit> {
    match value.as_known()? {
        SnoozeLimitProto::SnoozeLimitUnspecified => None,
        SnoozeLimitProto::SnoozeLimitForever => Some(SnoozeLimit::Forever),
        SnoozeLimitProto::SnoozeLimit3 => Some(SnoozeLimit::Three),
        SnoozeLimitProto::SnoozeLimit5 => Some(SnoozeLimit::Five),
    }
}

fn map_snooze_duration_proto(value: EnumValue<SnoozeDurationProto>) -> Option<SnoozeDuration> {
    match value.as_known()? {
        SnoozeDurationProto::SnoozeDurationUnspecified => None,
        SnoozeDurationProto::SnoozeDuration5Minutes => Some(SnoozeDuration::FiveMinutes),
        SnoozeDurationProto::SnoozeDuration10Minutes => Some(SnoozeDuration::TenMinutes),
        SnoozeDurationProto::SnoozeDuration15Minutes => Some(SnoozeDuration::FifteenMinutes),
        SnoozeDurationProto::SnoozeDuration30Minutes => Some(SnoozeDuration::ThirtyMinutes),
    }
}

impl From<SnoozeDuration> for SnoozeDurationProto {
    fn from(value: SnoozeDuration) -> Self {
        match value {
            SnoozeDuration::FiveMinutes => Self::SnoozeDuration5Minutes,
            SnoozeDuration::TenMinutes => Self::SnoozeDuration10Minutes,
            SnoozeDuration::FifteenMinutes => Self::SnoozeDuration15Minutes,
            SnoozeDuration::ThirtyMinutes => Self::SnoozeDuration30Minutes,
        }
    }
}

impl From<SnoozeLimit> for SnoozeLimitProto {
    fn from(value: SnoozeLimit) -> Self {
        match value {
            SnoozeLimit::Forever => Self::SnoozeLimitForever,
            SnoozeLimit::Three => Self::SnoozeLimit3,
            SnoozeLimit::Five => Self::SnoozeLimit5,
        }
    }
}

impl From<AlarmError> for ConnectError {
    fn from(value: AlarmError) -> Self {
        match value {
            AlarmError::SyncToStorage => ConnectError::internal("Failed to save configuration"),
            AlarmError::NotFound => ConnectError::not_found(value.to_string()),
            AlarmError::RemoveAlarm => ConnectError::internal("Failed to remove alarm"),
            AlarmError::ScheduleAlarm => ConnectError::internal("Failed to schedule alarm"),
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_grpc::google::rpc::bad_request::FieldViolation;
    use bmc_grpc::web::{
        Off, SnoozeDuration as SnoozeDurationProto, SnoozeLimit as SnoozeLimitProto,
        SnoozeOptionsWrapper, snooze_options_wrapper::Kind as SnoozeKind,
    };

    use super::{SnoozeDuration, SnoozeLimit, parse_snooze_options_field};

    fn violations_vec(violations: super::FieldViolations) -> Vec<FieldViolation> {
        violations.into()
    }

    #[test]
    fn parse_snooze_options_field_missing_wrapper_yields_violation() {
        let (parsed, violations) = parse_snooze_options_field("snooze_options", None);

        assert!(parsed.is_none());
        let violations = violations_vec(violations);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "snooze_options");
        assert_eq!(violations[0].description, "Missing value!");
    }

    #[test]
    fn parse_snooze_options_field_missing_kind_yields_violation() {
        let input = Some(SnoozeOptionsWrapper { kind: None });

        let (parsed, violations) = parse_snooze_options_field("snooze_options", input);

        assert!(parsed.is_none());
        let violations = violations_vec(violations);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].field, "snooze_options");
        assert_eq!(violations[0].description, "Missing value!");
    }

    #[test]
    fn parse_snooze_options_field_off_yields_no_violations_and_none() {
        let input = Some(SnoozeOptionsWrapper {
            kind: Some(SnoozeKind::Off(Off::default())),
        });

        let (parsed, violations) = parse_snooze_options_field("snooze_options", input);

        assert!(parsed.is_none());
        assert!(violations_vec(violations).is_empty());
    }

    #[test]
    fn parse_snooze_options_field_valid_snooze_yields_options() {
        let input = Some(SnoozeOptionsWrapper {
            kind: Some(SnoozeKind::Snooze(bmc_grpc::web::SnoozeOptions {
                duration: SnoozeDurationProto::SnoozeDuration10Minutes.into(),
                limit: SnoozeLimitProto::SnoozeLimit5.into(),
            })),
        });

        let (parsed, violations) = parse_snooze_options_field("snooze_options", input);

        let parsed = parsed.expect("BUG: valid snooze input must produce SnoozeOptions");
        assert!(matches!(parsed.duration, SnoozeDuration::TenMinutes));
        assert!(matches!(parsed.limit, SnoozeLimit::Five));
        assert!(violations_vec(violations).is_empty());
    }
}
