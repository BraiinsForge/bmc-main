// Copyright (C) 2026  Braiins Systems s.r.o.

const COMPASS: [&str; 8] = [
    "North",
    "Northeast",
    "East",
    "Southeast",
    "South",
    "Southwest",
    "West",
    "Northwest",
];

#[must_use]
pub fn cardinal(degrees: f64) -> &'static str {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "bearing divided by 45 and rounded is always a small integer; truncation is intentional"
    )]
    let index = (degrees / 45.0).round() as i64;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rem_euclid(8) on i64 is always in [0, 7]; fits usize on all targets"
    )]
    COMPASS[index.rem_euclid(8) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_degrees_to_eight_point_compass() {
        assert_eq!(cardinal(0.0), "North");
        assert_eq!(cardinal(45.0), "Northeast");
        assert_eq!(cardinal(177.0), "South");
        assert_eq!(cardinal(315.0), "Northwest");
        assert_eq!(cardinal(360.0), "North");
    }
}
