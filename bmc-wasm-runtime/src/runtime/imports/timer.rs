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

//! Periodic timer imports.

use anyhow::Result;
use bmc_wasm_protocol::TimerId;
use wasmi::{Caller, Linker};

use crate::host_api::HostState;
use crate::log_once::warn_once;
use crate::runtime::{PeriodChangeRefusal, TimerRefusal};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_timer_start",
        |mut caller: Caller<'_, HostState>, period_ms: u32| -> u32 {
            let state = caller.data_mut();
            match state.timers.start(period_ms, state.monotonic_ms) {
                Ok(id) => id.to_wire(),
                Err(TimerRefusal::PeriodTooShort) => {
                    warn_once!(
                        state.logged_sites,
                        instance_id = %state.instance_id,
                        period_ms,
                        min_timer_period_ms = state.resource_limits.min_timer_period_ms.get(),
                        "host_timer_start refused: period below the minimum"
                    );
                    0
                }
                Err(TimerRefusal::AtCap) => {
                    warn_once!(
                        state.logged_sites,
                        instance_id = %state.instance_id,
                        max_timers = state.resource_limits.max_timers,
                        "host_timer_start refused: runtime timer limit reached"
                    );
                    0
                }
                Err(TimerRefusal::NoOnTimerExport) => {
                    warn_once!(
                        state.logged_sites,
                        instance_id = %state.instance_id,
                        "host_timer_start refused: the widget exports no `__on_timer(u32, u32)`"
                    );
                    0
                }
                Err(TimerRefusal::IdsExhausted) => {
                    warn_once!(
                        state.logged_sites,
                        instance_id = %state.instance_id,
                        "host_timer_start refused: the widget has used every timer id"
                    );
                    0
                }
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "host_timer_set_period",
        |mut caller: Caller<'_, HostState>, timer_id: u32, period_ms: u32| -> u32 {
            let Some(id) = TimerId::from_wire(timer_id) else {
                return 0;
            };
            let state = caller.data_mut();
            match state.timers.set_period(id, period_ms, state.monotonic_ms) {
                Ok(()) => 1,
                Err(PeriodChangeRefusal::PeriodTooShort) => {
                    warn_once!(
                        state.logged_sites,
                        instance_id = %state.instance_id,
                        period_ms,
                        min_timer_period_ms = state.resource_limits.min_timer_period_ms.get(),
                        "host_timer_set_period refused: period below the minimum"
                    );
                    0
                }
                Err(PeriodChangeRefusal::NotRunning) => 0,
            }
        },
    )?;

    linker.func_wrap(
        "env",
        "host_timer_stop",
        |mut caller: Caller<'_, HostState>, timer_id: u32| {
            if let Some(id) = TimerId::from_wire(timer_id) {
                caller.data_mut().timers.stop(id);
            }
        },
    )?;

    Ok(())
}
