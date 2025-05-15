// Copyright (C) 2023  Braiins Systems s.r.o.

use std::fmt;
use std::fmt::{Display, Formatter};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct DisplayOption<'a, T: Display, N: Display> {
    option: &'a Option<T>,
    display_none: N,
}

impl<T: Display, N: Display> Display for DisplayOption<'_, T, N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.option {
            Some(item) => Display::fmt(item, f),
            None => Display::fmt(&self.display_none, f),
        }
    }
}

pub trait DisplayNoneAs<T: Display, N: Display> {
    fn display_none_as(&self, display: N) -> DisplayOption<'_, T, N>;
}

impl<T: Display, N: Display> DisplayNoneAs<T, N> for Option<T> {
    fn display_none_as(&self, display: N) -> DisplayOption<'_, T, N> {
        DisplayOption {
            option: self,
            display_none: display,
        }
    }
}
