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

impl<T> From<Option<T>> for Availability<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Available(value),
            None => Self::Unavailable,
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

    #[test]
    fn option_converts_to_availability() {
        assert_eq!(Availability::from(Some(7_u32)), Availability::Available(7));
        assert_eq!(Availability::from(None::<u32>), Availability::Unavailable);
    }
}
