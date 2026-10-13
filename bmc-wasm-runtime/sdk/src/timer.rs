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

//! Periodic timers for background work.
//!
//! A widget renders only while it is on screen,
//! but its timers keep firing while it is dormant,
//! so work done on a cadence (sampling a value for a chart, say)
//! has no gaps for the time the widget spent off screen.
//!
//! A tick runs like a fetch reply: submitting a tree from it traps,
//! so call `request_frame()` when the screen should change.
//! It runs under the same per-callback fuel budget as other deliveries,
//! and running out of fuel tears the widget down.
//!
//! # Limits
//!
//! - the period is at least 1000 ms by default (the host's `min_timer_period_ms`);
//!   a shorter one is refused
//! - a widget runs at most 4 timers at once by default (the host's `max_timers`)
//! - missed periods are not replayed: after a stall, one tick arrives
//!   carrying the number of periods it covers
//!
//! [`start`] returns a [`Timer`]; dropping it stops the timer.
//! A refused [`start`] returns `None` and the host logs why.
//! [`Timer::set_period`] changes the period of a running timer.
//!
//! # Example
//!
//! ```ignore
//! use std::cell::{Cell, RefCell};
//!
//! use bmc_wasm_sdk::timer::{self, Timer};
//!
//! thread_local! {
//!     // Set by the widget's fetch reply handler.
//!     static LATEST_HASHRATE: Cell<Option<f64>> = const { Cell::new(None) };
//!     // One entry per minute; `None` marks a minute with no sample.
//!     static SAMPLES: RefCell<Vec<Option<f64>>> = const { RefCell::new(Vec::new()) };
//!     static SAMPLER: RefCell<Option<Timer>> = const { RefCell::new(None) };
//!     static HISTORY_UNAVAILABLE: Cell<bool> = const { Cell::new(false) };
//! }
//!
//! fn init() {
//!     let sampler = timer::start(60_000, sample_hashrate);
//!     if sampler.is_none() {
//!         // The host has logged why; render a notice instead of an empty chart.
//!         HISTORY_UNAVAILABLE.set(true);
//!     }
//!     SAMPLER.set(sampler);
//! }
//!
//! fn sample_hashrate(_timer: &Timer, elapsed_periods: u32) {
//!     let missed_periods = elapsed_periods as usize - 1;
//!     SAMPLES.with_borrow_mut(|samples| {
//!         samples.resize(samples.len() + missed_periods, None);
//!         samples.push(LATEST_HASHRATE.get());
//!     });
//!     // The host does not render after a tick by itself.
//!     bmc_wasm_sdk::request_frame();
//! }
//! ```

use core::cell::RefCell;
use core::mem::ManuallyDrop;
use std::collections::HashMap;

use bmc_wasm_protocol::TimerId;

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_timer_start(period_ms: u32) -> u32;
    fn host_timer_set_period(timer_id: u32, period_ms: u32) -> u32;
    fn host_timer_stop(timer_id: u32);
}

/// Called with the timer that fired and the number of periods the tick covers.
pub type TimerCallback = fn(&Timer, u32);

/// The only handle to a running timer; dropping it stops the timer.
#[derive(Debug, PartialEq, Eq)]
pub struct Timer(TimerId);

thread_local! {
    static CALLBACKS: RefCell<HashMap<TimerId, TimerCallback>> = RefCell::new(HashMap::new());
}

/// Start a timer whose first tick arrives one period from now.
///
/// Returns `None` when `period_ms` is below the host's minimum,
/// when the widget already runs the most timers the host allows,
/// or once it has used every timer id.
#[cfg(target_arch = "wasm32")]
#[must_use = "dropping the returned Timer stops it"]
pub fn start(period_ms: u32, callback: TimerCallback) -> Option<Timer> {
    let id = TimerId::from_wire(unsafe { host_timer_start(period_ms) })?;
    CALLBACKS.with(|callbacks| callbacks.borrow_mut().insert(id, callback));
    Some(Timer(id))
}

#[cfg(target_arch = "wasm32")]
impl Timer {
    /// Run the timer at `period_ms`, its next tick one new period from now.
    ///
    /// The timer keeps its id and callback,
    /// and a tick of the old period already due in the same delivery pass is dropped.
    /// Returns `false`, leaving the timer as it was,
    /// when `period_ms` is below the host's minimum,
    /// or when called on a callback's argument after the callback stopped the timer.
    #[must_use]
    pub fn set_period(&self, period_ms: u32) -> bool {
        unsafe { host_timer_set_period(self.0.to_wire(), period_ms) != 0 }
    }
}

impl Timer {
    /// Stop the timer. A tick already due in the same delivery pass is dropped.
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        unregister(self.0);
        #[cfg(target_arch = "wasm32")]
        unsafe {
            host_timer_stop(self.0.to_wire());
        }
    }
}

fn unregister(id: TimerId) {
    CALLBACKS.with(|callbacks| callbacks.borrow_mut().remove(&id));
}

fn dispatch(id: TimerId, elapsed_periods: u32) {
    // Copied out first: the callback may start or stop timers itself.
    let callback = CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).copied());
    if let Some(callback) = callback {
        // Borrowed for the tick only; dropping it would stop the timer.
        callback(&ManuallyDrop::new(Timer(id)), elapsed_periods);
    }
}

/// Called by the host for each due timer.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn __on_timer(timer_id: u32, elapsed_periods: u32) {
    if let Some(id) = TimerId::from_wire(timer_id) {
        dispatch(id, elapsed_periods);
    }
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use bmc_wasm_protocol::TimerId;

    use super::{CALLBACKS, Timer, TimerCallback, dispatch};

    thread_local! {
        static FIRED: Cell<Option<(TimerId, u32)>> = const { Cell::new(None) };
        static STORED: RefCell<Option<Timer>> = const { RefCell::new(None) };
    }

    fn id(raw: u32) -> TimerId {
        TimerId::from_wire(raw).expect("BUG: test ids are non-zero")
    }

    fn register(raw: u32, callback: TimerCallback) {
        CALLBACKS.with(|callbacks| callbacks.borrow_mut().insert(id(raw), callback));
    }

    fn is_registered(raw: u32) -> bool {
        CALLBACKS.with(|callbacks| callbacks.borrow().contains_key(&id(raw)))
    }

    fn record(timer: &Timer, elapsed_periods: u32) {
        FIRED.set(Some((timer.0, elapsed_periods)));
    }

    fn stop_stored(_timer: &Timer, _elapsed_periods: u32) {
        if let Some(stored) = STORED.take() {
            stored.stop();
        }
    }

    #[test]
    fn a_tick_reaches_its_own_callback_with_the_elapsed_count() {
        register(1, record);

        dispatch(id(1), 3);

        assert_eq!(FIRED.get(), Some((id(1), 3)));
    }

    #[test]
    fn a_tick_leaves_its_timer_running() {
        register(2, record);

        dispatch(id(2), 1);

        assert!(is_registered(2));
    }

    #[test]
    fn a_dropped_handle_stops_its_timer() {
        register(3, record);

        drop(Timer(id(3)));
        dispatch(id(3), 1);

        assert_eq!(FIRED.get(), None);
    }

    #[test]
    fn a_callback_can_stop_its_own_timer_through_the_stored_handle() {
        register(4, stop_stored);
        STORED.set(Some(Timer(id(4))));

        dispatch(id(4), 1);

        assert!(!is_registered(4));
    }
}
