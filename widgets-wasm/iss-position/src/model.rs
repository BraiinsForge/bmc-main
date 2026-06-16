// Copyright (C) 2026  Braiins Systems s.r.o.

//! ISS data model and nexus payload parsing.

#[cfg(target_arch = "wasm32")]
use bmc_wasm_sdk::JsonDoc;
use bmc_wasm_sdk::{Length, Speed};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Daylight,
    Eclipsed,
}

impl Visibility {
    /// Parse the nexus lowercase wire value; anything but `eclipsed`
    /// reads as daylight so a stray value never hides the marker.
    #[must_use]
    pub fn from_wire(value: Option<&str>) -> Self {
        match value {
            Some("eclipsed") => Self::Eclipsed,
            _ => Self::Daylight,
        }
    }
}

/// Two-line orbital elements for SGP4 propagation.
#[derive(Clone)]
pub struct Tle {
    pub line1: String,
    pub line2: String,
}

/// One ISS snapshot from nexus: the reported position plus
/// the orbital elements the widget propagates from between refreshes.
pub struct IssData {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: Length,
    pub velocity: Speed,
    pub visibility: Visibility,
    pub solar_lat: f64,
    pub solar_lon: f64,
    // Also carried by the nexus payload but not surfaced yet
    // — kept here to document the wire contract:
    //   pub footprint: f64,  // km, ground-coverage radius
    //   pub timestamp: i64,  // unix seconds of the position snapshot
    /// Absent when nexus could not supply a TLE; the globe then falls back to
    /// the reported position with no orbital track.
    pub tle: Option<Tle>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssParseError {
    InvalidDocument,
    MissingField(&'static str),
}

#[cfg(target_arch = "wasm32")]
impl TryFrom<&JsonDoc> for IssData {
    type Error = IssParseError;

    fn try_from(doc: &JsonDoc) -> Result<Self, Self::Error> {
        if !doc.is_valid() {
            return Err(IssParseError::InvalidDocument);
        }
        let f64_at = |ptr: &'static str| doc.f64(ptr).ok_or(IssParseError::MissingField(ptr));

        Ok(Self {
            latitude: f64_at("/data/position/latitude")?,
            longitude: f64_at("/data/position/longitude")?,
            altitude: Length::from_kilometers(doc.f64("/data/position/altitude").unwrap_or(0.0)),
            velocity: Speed::from_kilometers_per_hour(
                doc.f64("/data/position/velocity").unwrap_or(0.0),
            ),
            visibility: Visibility::from_wire(doc.str("/data/position/visibility").as_deref()),
            solar_lat: doc.f64("/data/position/solar_lat").unwrap_or(0.0),
            solar_lon: doc.f64("/data/position/solar_lon").unwrap_or(0.0),
            // footprint: doc.f64("/data/position/footprint").unwrap_or(0.0),
            // timestamp: doc.i64("/data/position/timestamp").unwrap_or(0),
            tle: parse_tle(doc),
        })
    }
}

/// A TLE only when both lines are present and non-empty.
#[cfg(target_arch = "wasm32")]
fn parse_tle(doc: &JsonDoc) -> Option<Tle> {
    let line1 = doc.str("/data/tle/line1")?;
    let line2 = doc.str("/data/tle/line2")?;
    if line1.is_empty() || line2.is_empty() {
        return None;
    }
    Some(Tle { line1, line2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_maps_only_eclipsed_to_shadow() {
        assert_eq!(
            Visibility::from_wire(Some("eclipsed")),
            Visibility::Eclipsed
        );
        assert_eq!(
            Visibility::from_wire(Some("daylight")),
            Visibility::Daylight
        );
        // An unknown or absent value must not hide the daylight marker.
        assert_eq!(
            Visibility::from_wire(Some("nonsense")),
            Visibility::Daylight
        );
        assert_eq!(Visibility::from_wire(None), Visibility::Daylight);
    }
}
