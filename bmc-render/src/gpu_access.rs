// Copyright (C) 2026  Braiins Systems s.r.o.
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

#[cfg(debug_assertions)]
use std::cell::Cell;

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationState {
    Disabled,
    Pending,
    Authorized,
}

#[cfg(debug_assertions)]
thread_local! {
    static VALIDATION_STATE: Cell<ValidationState> = const { Cell::new(ValidationState::Disabled) };
}

#[must_use]
#[derive(Debug)]
pub struct GpuAccessValidationScope {
    #[cfg(debug_assertions)]
    previous: ValidationState,
}

impl GpuAccessValidationScope {
    pub fn enter() -> Self {
        #[cfg(debug_assertions)]
        {
            let previous = VALIDATION_STATE.replace(ValidationState::Pending);
            Self { previous }
        }
        #[cfg(not(debug_assertions))]
        Self {}
    }
}

impl Drop for GpuAccessValidationScope {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        VALIDATION_STATE.set(self.previous);
    }
}

pub fn authorize_gpu_access() {
    #[cfg(debug_assertions)]
    VALIDATION_STATE.with(|state| {
        if state.get() == ValidationState::Pending {
            state.set(ValidationState::Authorized);
        }
    });
}

pub(crate) fn assert_gpu_access_authorized() {
    #[cfg(debug_assertions)]
    VALIDATION_STATE.with(|state| {
        assert_ne!(
            state.get(),
            ValidationState::Pending,
            "GPU operation reached before delivery acquired GPU access"
        );
    });
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{GpuAccessValidationScope, assert_gpu_access_authorized, authorize_gpu_access};

    #[test]
    #[should_panic(expected = "GPU operation reached before delivery acquired GPU access")]
    fn validation_rejects_gpu_work_before_authorization() {
        let _scope = GpuAccessValidationScope::enter();
        assert_gpu_access_authorized();
    }

    #[test]
    fn validation_accepts_gpu_work_after_authorization() {
        let _scope = GpuAccessValidationScope::enter();
        authorize_gpu_access();
        assert_gpu_access_authorized();
    }
}
