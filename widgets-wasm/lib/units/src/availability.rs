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

/// What a widget has to show for one value it reads from a source.
///
/// `Unavailable` and `Failed` both mean "no value", and a widget is free to
/// draw them alike — but only `Failed` says asking has already been tried, so
/// waiting will not help. A screen that draws its loading state for both
/// leaves a source that failed before its first answer loading forever.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Availability<T> {
    Available(T),
    /// No answer yet.
    Unavailable,
    /// Asked, and came back without a value.
    Failed,
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
            Self::Unavailable | Self::Failed => None,
        }
    }

    /// Whether asking has been tried and did not produce a value.
    #[must_use]
    pub fn failed(&self) -> bool {
        matches!(self, Self::Failed)
    }

    /// Record that asking failed — unless a value has already arrived, which
    /// stands: one bad answer is worth less than the data it would blank.
    /// Reports whether this changed anything, so a caller can spare a redraw
    /// for the failure it already knows about.
    pub fn mark_failed(&mut self) -> bool {
        let unset = matches!(self, Self::Unavailable);
        if unset {
            *self = Self::Failed;
        }
        unset
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
    fn failure_does_not_displace_a_value_already_read() {
        let mut value = Availability::Available(42_u32);
        assert!(!value.mark_failed(), "a value already read changed state");
        assert_eq!(value, Availability::Available(42));

        let mut nothing = Availability::<u32>::Unavailable;
        assert!(nothing.mark_failed(), "the first failure went unrecorded");
        assert_eq!(nothing, Availability::Failed);
        assert!(!nothing.mark_failed(), "a repeated failure changed state");
    }

    #[test]
    fn option_converts_to_availability() {
        assert_eq!(Availability::from(Some(7_u32)), Availability::Available(7));
        assert_eq!(Availability::from(None::<u32>), Availability::Unavailable);
    }
}
