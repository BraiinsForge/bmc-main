// Copyright (C) 2025  Braiins Systems s.r.o.
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
