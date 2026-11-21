# WASM Widget Timers

A widget renders only while it is on screen, but its timers keep firing while it is dormant. Work done on a cadence,
such as sampling a value for a chart, therefore has no gaps for the time the widget spent off screen.

Use a timer for background work. Animation and periodic repaints belong to `request_frame_after()`, which only runs
while the widget renders.

## Start And Stop A Timer

`Timer::start(to, period_ms, Self::method)` starts a timer whose ticks run that method on `to`, and returns `Ok` with a
`Timer` handle, the only one for that timer: it cannot be copied, and dropping it stops the timer. The first tick
arrives one period after the call, and every tick calls the method with a reference to the timer and the number of
periods the tick covers.

```rust
use bmc_wasm_sdk::timer::Timer;
use bmc_wasm_sdk::*;

struct Chart {
    // Set by the widget's fetch reply handler.
    latest_hashrate: Option<f64>,
    // One entry per minute; `None` marks a minute with no sample.
    samples: Vec<Option<f64>>,
    sampler: Option<Timer>,
}

impl Chart {
    fn sample_hashrate(&mut self, _timer: &Timer, elapsed_periods: u32) {
        let missed_periods = elapsed_periods as usize - 1;
        self.samples.resize(self.samples.len() + missed_periods, None);
        self.samples.push(self.latest_hashrate);
        request_frame();
    }
}

#[widget]
impl Widget for Chart {
    fn init(me: &Building<Self>) -> Self {
        let sampler = Timer::start(me, 60_000, Self::sample_hashrate).ok();
        Self { latest_hashrate: None, samples: Vec::new(), sampler }
    }

    fn render(&mut self, _delta_ms: u32) { /* … */ }
}
```

The tick only records the sample. `request_frame()` is what puts it on the chart, since the host does not render after a
tick by itself.

`Timer::start` returns `Err(RegisterError)` when the host refuses the timer, and the host logs why. Through a
`PartHandle` it returns `Err(HandleError::Register)` for that refusal, or `Err(HandleError::Part)` without asking the
host while the part is not live. Keep the handle, and when there is none, make the widget show that the chart has no
history instead of leaving it empty.

`Timer::start(&part, period_ms, T::method)` starts a timer whose ticks run a method of the value in a `Part<T>` instead;
see [Widget Struct](widget-struct.md) for every receiver. `timer::start(period_ms, callback)` takes a plain function,
and stays only so that a widget not yet ported still builds.

`Timer::set_period(period_ms)` moves a running timer to a new period. It keeps its id and callback, and its next tick
arrives one new period after the call; a tick of the old period already due in the same delivery pass is dropped. It
returns `false`, leaving the timer as it was, when the period is below the minimum, or when a callback calls it on its
argument after stopping that timer.

`Timer::stop()` ends a timer and consumes the handle; dropping the handle does the same. A tick of that timer already
due in the same delivery pass is dropped, so no callback arrives after the timer stops.

A callback may start and stop timers. The argument is only borrowed, so a callback stops its own timer through the
stored handle:

```rust
if let Some(sampler) = self.sampler.take() {
    sampler.stop();
}
```

Several timers can share one callback. For a fixed set, give each timer a one-line method that names which timer it is
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

| Limit                     | Value                                                 | Past the limit                                   |
| ------------------------- | ----------------------------------------------------- | ------------------------------------------------ |
| Shortest period           | 1000 ms by default (the host's `min_timer_period_ms`) | `start` is refused, `set_period` returns `false` |
| Running timers per widget | 4 by default (the host's `max_timers`)                | `start` is refused                               |

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
recommends for all widget logic, so a unit test can cover it without driving a timer.
