// Copyright (C) 2026  Braiins Systems s.r.o.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Availability<T> {
    Available(T),
    Unavailable,
}

#[expect(
    clippy::derivable_impls,
    reason = "deriving Default would add a T: Default bound; this impl stays unbounded so containers holding non-Default payloads can still derive Default"
)]
impl<T> Default for Availability<T> {
    fn default() -> Self {
        Self::Unavailable
    }
}

impl<T> Availability<T> {
    pub fn as_option(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_values_can_be_borrowed_as_options() {
        let value = Availability::Available(42_u32);
        assert_eq!(value.as_option(), Some(&42));
    }

    #[test]
    fn default_is_unavailable() {
        let value = Availability::<u32>::default();
        assert_eq!(value, Availability::Unavailable);
    }
}
