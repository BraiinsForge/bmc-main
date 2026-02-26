# LED Driver CPU Hog Bug

## Summary

The LED driver's SPI worker task runs at **1 microsecond intervals** when no LED
effect is active, writing "off" frames to the SPI bus ~1 million times per second.
This consumes ~60% of one CPU core in kernel time, accounting for nearly all idle
CPU usage on the device.

## Root Cause

### Chain of Events

1. `PlatformLedDriver::new()` spawns `led_worker` with default `LedState`:
   - `persistent_effect = LedEffect::None`
   - `period_us = SOLID_PERIOD.as_micros() = 0` (SOLID_PERIOD is 0ms)

2. `calc_frame_interval(0)` computes:
   ```
   0 / (LED_COUNT * SUB_STEPS) = 0 / (10 * 200) = 0
   ```
   The zero-duration fallback clamps to `Duration::from_micros(1)`.

3. The tokio interval ticks at 1us. Each tick writes an SPI "off" frame via
   kernel ioctl, burning CPU in kernel mode.

4. At startup, `DeviceReady` event clears the device-persist effect.
   `select_persistent()` returns `LedEffect::None` with `Duration::from_secs(0)`,
   perpetuating the 1us interval.

### The Hot Path

```
led_worker loop (1MHz):
  ├── check for commands (empty → skip)
  ├── calculate phase (period_us=0 → phase=0)
  ├── update_none() → SPI ioctl (kernel time)
  └── interval.tick().await → returns in 1us
```

## Evidence

Per-thread CPU measurement over 5 seconds on target (ARMv7):

| Thread            | utime delta | stime delta | CPU%  |
|-------------------|-------------|-------------|-------|
| tokio-runtime-w   | 9 ticks     | **301 ticks** | **61%** |
| egl-compositor    | 0 ticks     | 0 ticks     | 0%    |

The compositor thread sleeps correctly in `epoll_wait(-1)`. All idle CPU is from
the tokio worker running the LED SPI loop.

## Fix

Two problems cause unnecessary CPU load:

1. **Static effects busy-loop at 1us** (the critical bug above).
2. **Variable frame rate for animations** — the old code derives frame interval
   from the effect's period (`period / LED_COUNT / SUB_STEPS`).  A 1000ms
   KnightRider with 10 LEDs × 200 sub-steps = 0.5us per frame ≈ 2 MHz tick
   rate, far beyond what is visible to the human eye and wasteful on an
   embedded ARM core.

### Refactor LedState

Rewrite `LedState` into a self-contained object:

- Use a **fixed 200 Hz frame rate** (`FRAME_RATE_HZ = 200`, 5ms per frame) —
  create the interval once, share across all effects.  200 fps is well above
  the perceptible animation threshold and keeps CPU load minimal.
- Store period as **`Option<Duration>`** — `None` for static effects (Solid,
  None), `Some(...)` for animated.  Read it directly in `phase()`.
- Add **`next_wake()`** to decide how to sleep:
  - Animated (`period = Some`): tick `frame_interval` at 200 Hz
  - Static with temp expiry: `sleep_until(expiry)` — wake exactly once
  - Fully idle: `std::future::pending()` — block forever, CPU ≈ 0%

### Fix temp-expiry overwrite bug

When temp + persistent commands arrive together (they always do from
`LedEventHandler`), the persistent command's interval creation destroys the
temp's expiry timer.  Restrict `period`/`animation_start` updates to
`Persistent` commands only; temp commands set their own fields.

### Fix PreviewScene period

Change `PreviewScene` from `KNIGHT_RIDER_PERIOD` (1000ms) to `SOLID_PERIOD`
(0ms) — stop unnecessary frame ticks for a static solid effect.

Reduce idle CPU from ~60% to ~0% for the LED subsystem.
