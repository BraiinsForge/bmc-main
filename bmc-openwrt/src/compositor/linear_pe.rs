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

//! Etnaviv's linear PE for the compositor's own GL context.
//!
//! Mesa keeps linear PE behind `ETNA_MESA_DEBUG=linear_pe` (mesa `c5231025`)
//! for faults in the depth/stencil stage while the GPU renders into a linear buffer.
//! The compositor renders linear but uses no depth or stencil.
//! Its children never get the variable, a developer's own value included:
//! a widget falling back to a linear buffer renders with a stencil.

use std::fmt;

use bmc_platform::ETNA_MESA_DEBUG_ENV;

const LINEAR_PE: &str = "linear_pe";

/// What [`enable`] did with `ETNA_MESA_DEBUG`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinearPe {
    On,
    /// Already set, so a developer's choice stays as it is.
    LeftAsSet(String),
}

/// Sets `ETNA_MESA_DEBUG=linear_pe` before the EGL display that reads it comes up,
/// unless a developer set the variable.
///
/// # Safety
///
/// No other thread may exist yet: `set_var` races any concurrent access to the environment.
#[must_use]
pub unsafe fn enable() -> LinearPe {
    let existing =
        std::env::var_os(ETNA_MESA_DEBUG_ENV).map(|value| value.to_string_lossy().into_owned());
    let decision = decide(existing);
    if decision == LinearPe::On {
        // SAFETY: the caller guarantees this process has no other thread yet.
        unsafe { std::env::set_var(ETNA_MESA_DEBUG_ENV, LINEAR_PE) };
    }
    decision
}

fn decide(existing: Option<String>) -> LinearPe {
    match existing {
        Some(existing) => LinearPe::LeftAsSet(existing),
        None => LinearPe::On,
    }
}

impl fmt::Display for LinearPe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::On => write!(f, "on"),
            Self::LeftAsSet(value) => write!(f, "{ETNA_MESA_DEBUG_ENV}={value:?} left as set"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LinearPe, decide};

    #[test]
    fn an_unset_variable_turns_linear_pe_on() {
        assert_eq!(decide(None), LinearPe::On);
    }

    #[test]
    fn a_value_set_by_a_developer_wins() {
        assert_eq!(
            decide(Some("msgs".to_owned())),
            LinearPe::LeftAsSet("msgs".to_owned())
        );
    }
}
