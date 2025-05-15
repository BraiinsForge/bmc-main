// Copyright (C) 2024  Braiins Systems s.r.o.

use chrono::{DateTime, TimeZone, Utc};

#[must_use]
pub fn datetime2proto<Tz: TimeZone>(dt: &DateTime<Tz>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        #[expect(clippy::cast_possible_wrap)] // 1s converted to ms is always < i32::MAX
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

#[must_use]
pub fn proto2datetime(proto: &prost_types::Timestamp) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(proto.seconds, u32::try_from(proto.nanos).ok()?)
}
