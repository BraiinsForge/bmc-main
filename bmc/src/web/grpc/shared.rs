// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_grpc::web::NumberFormat;
use chrono::NaiveTime;
use tonic::Status;
use tonic_types::FieldViolation;
use tracing::warn;

pub type ParseOutput<T> = (Option<T>, FieldViolations);

pub fn unchecked_field_violations_status() -> Status {
    Status::internal("Unchecked field violations")
}

#[derive(Debug)]
pub struct FieldViolations(Vec<FieldViolation>);

impl FieldViolations {
    pub fn new() -> Self {
        Self(Vec::with_capacity(0))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, field: impl Into<String>, description: impl Into<String>) {
        self.0.push(FieldViolation::new(field, description));
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

pub fn parse_hhmm_to_naive_time(input: &str) -> Result<NaiveTime, Status> {
    NaiveTime::parse_from_str(input, "%H:%M").map_err(|e| {
        warn!("Failed to parse Time in format HH:MM, error: {}", e);
        Status::invalid_argument("Invalid time format")
    })
}

pub(crate) fn try_from_number_format(
    value: NumberFormat,
) -> Result<bmc_shared_utils::number_format::NumberFormat, FieldViolation> {
    match value {
        NumberFormat::Unspecified => Err(FieldViolation::new(
            "number_format",
            "number_format cannot be unspecified",
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
