// Copyright (C) 2026  Braiins Systems s.r.o.

//! LED request scope, shared by the wasm SDK (guest) and the host runtime.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LedScope {
    Local = 0,
    Global = 1,
}

impl TryFrom<u8> for LedScope {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Local),
            1 => Ok(Self::Global),
            other => Err(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_match_protocol() {
        assert_eq!(LedScope::Local as u8, 0);
        assert_eq!(LedScope::Global as u8, 1);
    }

    #[test]
    fn try_from_round_trips_known_values() {
        for v in 0_u8..=1 {
            assert_eq!(
                LedScope::try_from(v).expect("BUG: 0..=1 must be valid") as u8,
                v
            );
        }
        assert_eq!(LedScope::try_from(2_u8), Err(2));
    }
}
