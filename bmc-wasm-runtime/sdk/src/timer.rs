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
//! A started timer comes back as a [`Timer`]; dropping it stops the timer.
//! A refused [`Timer::start`] returns `RegisterError`
//! (`HandleError::Register` through a `PartHandle`), and the host logs why;
//! one through a `PartHandle` that is not live returns `HandleError::Part`, without a log.
//! A refused free `start` returns `None`.
//! [`Timer::set_period`] changes the period of a running timer.
//!
//! # Example
//!
//! ```ignore
//! use bmc_wasm_sdk::timer::Timer;
//! use bmc_wasm_sdk::{Building, Widget, widget};
//!
//! struct Chart {
//!     // Set by the widget's fetch reply handler.
//!     latest_hashrate: Option<f64>,
//!     // One entry per minute; `None` marks a minute with no sample.
//!     samples: Vec<Option<f64>>,
//!     sampler: Option<Timer>,
//! }
//!
//! impl Chart {
//!     fn sample_hashrate(&mut self, _timer: &Timer, elapsed_periods: u32) {
//!         let missed_periods = elapsed_periods as usize - 1;
//!         self.samples.resize(self.samples.len() + missed_periods, None);
//!         self.samples.push(self.latest_hashrate);
//!         // The host does not render after a tick by itself.
//!         bmc_wasm_sdk::request_frame();
//!     }
//! }
//!
//! #[widget]
//! impl Widget for Chart {
//!     fn init(me: &Building<Self>) -> Self {
//!         // A refused timer leaves `sampler` None: the host has logged why,
//!         // and `render` shows a notice instead of an empty chart.
//!         let sampler = Timer::start(me, 60_000, Self::sample_hashrate).ok();
//!         Self { latest_hashrate: None, samples: Vec::new(), sampler }
//!     }
//!
//!     fn render(&mut self, _delta_ms: u32) { /* … */ }
//! }
//! ```

use core::cell::RefCell;
use core::mem::ManuallyDrop;
use std::collections::HashMap;
use std::rc::Rc;

use bmc_wasm_protocol::TimerId;

use crate::widget::Target;
#[cfg(target_arch = "wasm32")]
use crate::widget::{self, Receiver, RegisterError};

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_timer_start(period_ms: u32) -> u32;
    fn host_timer_set_period(timer_id: u32, period_ms: u32) -> u32;
    fn host_timer_stop(timer_id: u32);
}

/// Called with the timer that fired and the number of periods the tick covers.
pub type TimerCallback = fn(&Timer, u32);

type Handler = Rc<dyn Fn(&Timer, u32)>;

/// The only handle to a running timer; dropping it stops the timer.
#[derive(Debug, PartialEq, Eq)]
pub struct Timer(TimerId);

thread_local! {
    static CALLBACKS: RefCell<HashMap<TimerId, Handler>> = RefCell::new(HashMap::new());
}

/// Start a timer whose first tick arrives one period from now.
///
/// Returns `None` when `period_ms` is below the host's minimum,
/// when the widget already runs the most timers the host allows,
/// or once it has used every timer id.
#[cfg(target_arch = "wasm32")]
#[must_use = "dropping the returned Timer stops it"]
pub fn start(period_ms: u32, callback: TimerCallback) -> Option<Timer> {
    start_with(period_ms, Rc::new(callback))
}

#[cfg(target_arch = "wasm32")]
fn start_with(period_ms: u32, callback: Handler) -> Option<Timer> {
    let id = TimerId::from_wire(unsafe { host_timer_start(period_ms) })?;
    register(id, callback);
    Some(Timer(id))
}

/// A timer's tick as a method of the value it is delivered to.
pub type TimerMethod<T> = fn(&mut T, &Timer, u32);

fn on_target<T: 'static>(target: Target<T>, method: TimerMethod<T>) -> Handler {
    Rc::new(move |timer: &Timer, elapsed_periods| {
        target.deliver(|value| method(value, timer, elapsed_periods));
    })
}

#[cfg(target_arch = "wasm32")]
impl Timer {
    /// Start a timer whose ticks run `method` on `to`.
    ///
    /// Otherwise as [`start`],
    /// but a refusal is [`RegisterError`] rather than `None`,
    /// or [`HandleError`](crate::HandleError) through a [`PartHandle`](crate::PartHandle).
    /// Ticks after the part is dropped run nothing.
    #[must_use = "dropping the returned Timer stops it"]
    pub fn start<T: 'static, R: Receiver<T>>(
        to: &R,
        period_ms: u32,
        method: TimerMethod<T>,
    ) -> R::Out<Result<Timer, RegisterError>> {
        to.__register(|target| {
            start_with(period_ms, on_target(target, method)).ok_or(RegisterError::new(()))
        })
    }

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

fn register(id: TimerId, callback: Handler) {
    CALLBACKS.with(|callbacks| callbacks.borrow_mut().insert(id, callback));
}

fn unregister(id: TimerId) {
    CALLBACKS.with(|callbacks| callbacks.borrow_mut().remove(&id));
}

fn dispatch(id: TimerId, elapsed_periods: u32) {
    // Cloned out first: the callback may start or stop timers itself.
    let callback = CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).cloned());
    if let Some(callback) = callback {
        // Borrowed for the tick only; dropping it would stop the timer.
        callback(&ManuallyDrop::new(Timer(id)), elapsed_periods);
    }
}

/// Called by the host for each due timer.
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn __on_timer(timer_id: u32, elapsed_periods: u32) {
    widget::export(|| {
        if let Some(id) = TimerId::from_wire(timer_id) {
            dispatch(id, elapsed_periods);
        }
    });
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};
    use std::rc::Rc;

    use bmc_wasm_protocol::TimerId;

    use super::{CALLBACKS, Timer, TimerCallback, dispatch, on_target};
    use crate::wasm_test;
    use crate::widget::Part;

    thread_local! {
        static FIRED: Cell<Option<(TimerId, u32)>> = const { Cell::new(None) };
        static STORED: RefCell<Option<Timer>> = const { RefCell::new(None) };
    }

    fn id(raw: u32) -> TimerId {
        TimerId::from_wire(raw).expect("BUG: test ids are non-zero")
    }

    fn register(raw: u32, callback: TimerCallback) {
        super::register(id(raw), Rc::new(callback));
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

    #[wasm_test]
    fn a_tick_reaches_its_own_callback_with_the_elapsed_count() {
        register(1, record);

        dispatch(id(1), 3);

        assert_eq!(FIRED.get(), Some((id(1), 3)));
    }

    #[wasm_test]
    fn a_tick_leaves_its_timer_running() {
        register(2, record);

        dispatch(id(2), 1);

        assert!(is_registered(2));
    }

    #[wasm_test]
    fn a_dropped_handle_stops_its_timer() {
        register(3, record);

        drop(Timer(id(3)));
        dispatch(id(3), 1);

        assert_eq!(FIRED.get(), None);
    }

    #[wasm_test]
    fn a_callback_can_stop_its_own_timer_through_the_stored_handle() {
        register(4, stop_stored);
        STORED.set(Some(Timer(id(4))));

        dispatch(id(4), 1);

        assert!(!is_registered(4));
    }

    struct Sampler {
        ticks: u32,
        timer: Option<Timer>,
    }

    impl Sampler {
        fn tick(&mut self, _timer: &Timer, elapsed_periods: u32) {
            self.ticks += elapsed_periods;
            TICKS.set(TICKS.get() + 1);
        }

        fn stop_own_timer(&mut self, _timer: &Timer, _elapsed_periods: u32) {
            self.timer.take();
        }
    }

    thread_local! {
        static TICKS: Cell<u32> = const { Cell::new(0) };
    }

    fn sampler() -> Part<Sampler> {
        Part::new(Sampler {
            ticks: 0,
            timer: None,
        })
    }

    // The tick export exists on wasm32 only.
    #[cfg(target_arch = "wasm32")]
    mod export {
        use super::{TICKS, Timer, register};
        use crate::{wasm_test, widget};

        fn defer_a_tick(_timer: &Timer, _elapsed_periods: u32) {
            widget::defer(|| TICKS.set(TICKS.get() + 1));
        }

        #[wasm_test]
        fn work_deferred_from_a_tick_runs_once_the_tick_returned() {
            register(7, defer_a_tick);

            crate::timer::__on_timer(7, 1);

            assert_eq!(
                TICKS.get(),
                1,
                "the tick's export drains before it hands back to the host"
            );
        }
    }

    #[wasm_test]
    fn a_part_stops_its_own_timer_from_the_tick() {
        let mut part = sampler();
        super::register(id(6), on_target(part.target(), Sampler::stop_own_timer));
        part.with_mut(|value| value.timer = Some(Timer(id(6))));

        dispatch(id(6), 1);

        assert!(!is_registered(6));
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_test]
    fn a_tick_from_the_host_runs_as_a_method_of_the_part() {
        let part = sampler();
        super::register(id(8), on_target(part.target(), Sampler::tick));

        crate::widget::as_host(|| super::__on_timer(8, 2));

        assert_eq!(part.with(|value| value.ticks), 2);
    }

    #[wasm_test]
    fn a_dropped_part_gets_no_ticks() {
        let part = sampler();
        super::register(id(7), on_target(part.target(), Sampler::tick));
        drop(part);

        dispatch(id(7), 1);

        assert_eq!(TICKS.get(), 0);
    }
}
