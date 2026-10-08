# WASM Widget Timers

A widget renders only while it is on screen, but its timers keep firing while it is dormant. Work done on a cadence,
such as sampling a value for a chart, therefore has no gaps for the time the widget spent off screen.

Use a timer for background work. Animation and periodic repaints belong to `request_frame_after()`, which only runs
while the widget renders.

## Start And Stop A Timer

`timer::start(period_ms, callback)` returns a `Timer` handle, the only one for that timer: it cannot be copied, and
dropping it stops the timer. The first tick arrives one period after the call, and every tick calls `callback` with a
reference to the timer and the number of periods the tick covers.

```rust
use std::cell::{Cell, RefCell};

use bmc_wasm_sdk::timer::{self, Timer};
use bmc_wasm_sdk::*;

thread_local! {
    // Set by the widget's fetch reply handler.
    static LATEST_HASHRATE: Cell<Option<f64>> = const { Cell::new(None) };
    // One entry per minute; `None` marks a minute with no sample.
    static SAMPLES: RefCell<Vec<Option<f64>>> = const { RefCell::new(Vec::new()) };
    static SAMPLER: RefCell<Option<Timer>> = const { RefCell::new(None) };
    static HISTORY_UNAVAILABLE: Cell<bool> = const { Cell::new(false) };
}

#[unsafe(no_mangle)]
pub extern "C" fn init() {
    let sampler = timer::start(60_000, sample_hashrate);
    if sampler.is_none() {
        HISTORY_UNAVAILABLE.set(true);
    }
    SAMPLER.set(sampler);
}

fn sample_hashrate(_timer: &Timer, elapsed_periods: u32) {
    let missed_periods = elapsed_periods as usize - 1;
    SAMPLES.with_borrow_mut(|samples| {
        samples.resize(samples.len() + missed_periods, None);
        samples.push(LATEST_HASHRATE.get());
    });
    request_frame();
}
```

The tick only records the sample. `request_frame()` is what puts it on the chart, since the host does not render after a
tick by itself.

`start` returns `None` when the host refuses the timer, and the host logs why. Keep the handle, and when there is none,
make the widget show that the chart has no history instead of leaving it empty.

`Timer::stop()` ends a timer and consumes the handle; dropping the handle does the same. A tick of that timer already
due in the same delivery pass is dropped, so no callback arrives after the timer stops.

A callback may start and stop timers. The argument is only borrowed, so a callback stops its own timer through the
stored handle:

```rust
if let Some(sampler) = SAMPLER.take() {
    sampler.stop();
}
```

Several timers can share one callback. For a fixed set, give each timer a one-line callback that names which timer it is
and calls the shared function. For a set that changes at run time, compare the argument with the stored handles to find
the one that fired.

## What A Tick May Do

A tick runs where fetch replies run, under the same import rules as the `on_*` hooks (see the lifecycle guard matrix in
the `bmc_wasm_sdk` crate docs). Submitting a tree from a tick traps, and the host tears the widget down. Call
`request_frame()` when the change should repaint. While the widget is dormant the request waits: nothing renders off
screen, and the widget repaints when it returns.

Each tick gets the same per-callback fuel budget as any other delivery. A tick that runs out of fuel traps, and the host
tears the widget down, so keep the work done per tick small.

## Limits

| Limit                     | Value                                                 | Past the limit         |
| ------------------------- | ----------------------------------------------------- | ---------------------- |
| Shortest period           | 1000 ms by default (the host's `min_timer_period_ms`) | `start` returns `None` |
| Running timers per widget | 4 by default (the host's `max_timers`)                | `start` returns `None` |

The host logs why it refused a `start`: a warning the first time per widget and reason, and at debug level after that. A
stopped timer no longer counts against the limit.

Widgets built without the SDK must export `__on_timer(timer_id: u32, elapsed_periods: u32)`; the host refuses every
`start` from a widget that lacks it.

## Missed Periods

The host does not replay missed ticks. When it falls behind, for example during a long delivery or a busy main loop, the
overdue periods arrive as one tick whose `elapsed_periods` counts them. A tick on time carries 1.

With a 1000 ms timer started at 0 s, a stall from 1.5 s to 4.5 s produces one tick at 4.5 s carrying 3, for the periods
that ended at 2 s, 3 s and 4 s. The next tick still arrives at 5 s: a late tick does not shift the timer.

Read `elapsed_periods` rather than assuming one period passed. A chart that records one sample per tick otherwise hides
the stall it should show.

## Dormancy

A timer running while the widget is dormant keeps the host busy for a scene nobody is looking at. If the work only
matters on screen, start the timer in `on_wake` instead of `init`, since `on_wake` also runs before the widget's first
frame, and stop it in `on_sleep`.

Keep the logic a tick runs in a plain function that takes `elapsed_periods`, as [Best Practices](best-practices.md)
recommends for all widget logic, so native tests can cover it without a host.
