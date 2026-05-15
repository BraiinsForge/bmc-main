// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED effects, shared by the wasm SDK (guest) and the host runtime.
//!
//! Discriminants match `deck_widget_v1.led_effect` exactly so a
//! protocol-aligned `u8` round-trips without translation.

/// One LED effect. Pinned with `repr(u8)` to match the wire enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LedEffect {
    Chase = 0,
    KnightRider = 1,
    Scan = 2,
    Snake = 3,
    Breathe = 4,
    Solid = 5,
}

impl TryFrom<u8> for LedEffect {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Chase),
            1 => Ok(Self::KnightRider),
            2 => Ok(Self::Scan),
            3 => Ok(Self::Snake),
            4 => Ok(Self::Breathe),
            5 => Ok(Self::Solid),
            other => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_match_protocol() {
        assert_eq!(LedEffect::Chase as u8, 0);
        assert_eq!(LedEffect::KnightRider as u8, 1);
        assert_eq!(LedEffect::Scan as u8, 2);
        assert_eq!(LedEffect::Snake as u8, 3);
        assert_eq!(LedEffect::Breathe as u8, 4);
        assert_eq!(LedEffect::Solid as u8, 5);
    }

    #[test]
    fn try_from_round_trips_known_values() {
        for v in 0_u8..=5 {
            assert_eq!(
                LedEffect::try_from(v).expect("BUG: 0..=5 must be valid") as u8,
                v
            );
        }
        assert_eq!(LedEffect::try_from(6_u8), Err(6));
    }
}
