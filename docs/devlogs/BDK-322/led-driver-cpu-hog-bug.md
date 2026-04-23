# LED Driver CPU Hog Bug

## Summary

The LED driver's SPI worker task runs at **1 microsecond intervals** when no LED effect is active, writing "off" frames
to the SPI bus ~1 million times per second. This consumes ~60% of one CPU core in kernel time, accounting for nearly all
idle CPU usage on the device.

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

3. The tokio interval ticks at 1us. Each tick writes an SPI "off" frame via kernel ioctl, burning CPU in kernel mode.

4. At startup, `DeviceReady` event clears the device-persist effect. `select_persistent()` returns `LedEffect::None`
   with `Duration::from_secs(0)`, perpetuating the 1us interval.

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

| Thread          | utime delta | stime delta   | CPU%    |
| --------------- | ----------- | ------------- | ------- |
| tokio-runtime-w | 9 ticks     | **301 ticks** | **61%** |
| egl-compositor  | 0 ticks     | 0 ticks       | 0%      |

The compositor thread sleeps correctly in `epoll_wait(-1)`. All idle CPU is from the tokio worker running the LED SPI
loop.

## Fix

Two problems cause unnecessary CPU load:

1. **Static effects busy-loop at 1us** (the critical bug above).
2. **Variable frame rate for animations** — the old code derives frame interval from the effect's period
   (`period / LED_COUNT / SUB_STEPS`). A 1000ms KnightRider with 10 LEDs × 200 sub-steps = 0.5us per frame ≈ 2 MHz tick
   rate, far beyond what is visible to the human eye and wasteful on an embedded ARM core.

### Refactor LedState

Rewrite `LedState` into a self-contained object:

- Use a **fixed 120 Hz frame rate** (`FRAME_RATE_HZ = 120`, ~8.3ms per frame) — create the interval once, share across
  all effects. 120 fps matches the LED refresh rate and keeps CPU load minimal.
- Store period as **`Option<Duration>`** — `None` for static effects (Solid, None), `Some(...)` for animated. Read it
  directly in `phase()`.
- Add **`next_wake()`** to decide how to sleep:
  - Animated (`period = Some`): tick `frame_interval` at 120 Hz
  - Static with temp expiry: `sleep_until(expiry)` — wake exactly once
  - Fully idle: `std::future::pending()` — block forever, CPU ≈ 0%

### Fix temp-expiry overwrite bug

When temp + persistent commands arrive together (they always do from `LedEventHandler`), the persistent command's
interval creation destroys the temp's expiry timer. Restrict `period`/`animation_start` updates to `Persistent` commands
only; temp commands set their own fields.

### Fix PreviewScene period

Change `PreviewScene` from `KNIGHT_RIDER_PERIOD` (1000ms) to `SOLID_PERIOD` (0ms) — stop unnecessary frame ticks for a
static solid effect.

Reduce idle CPU from ~60% to ~0% for the LED subsystem.

## HW verification

### Architecture

Add a debug gRPC service (`LedTestService`) that injects `LedCommand` directly into the LED driver's command channel,
bypassing the event handler (`LedIndicatorsState`). This isolates and tests the exact code changed by the fix:
`LedState::apply_command()`, `update_effect()`, `phase()`, `next_wake()`, temp-effect expiry, brightness control, and
the idle-CPU behavior.

```
Normal path:       LedEvent → LedIndicatorsState → LedCommand → led_worker → SPI
                                  (event handler)

Test path:         grpcurl → LedTestService (gRPC) → LedCommand → led_worker → SPI
                                  (event handler bypassed)
```

Store a clone of `Sender<LedCommand>` in `LedController` during `init()` and expose a `send_command()` method. The gRPC
service receives the controller clone and calls `send_command()` for each RPC (SetEffect, SetBrightness, Enable,
Disable).

### Security

`LedTestService` is wrapped in the same middleware stack as all other authenticated services: `GrpcWebLayer` for
HTTP/gRPC-web content-type demux, `InterceptorFor` with `AuthInterceptor` for bearer-token validation, and
`GrpcLoggingLayer` for request logging. Unauthenticated requests are rejected before reaching the service handler.

The test script uses `-plaintext` (no TLS) because the device serves gRPC on port 80 without TLS. Authentication is
currently disabled for this endpoint during development; re-enable `AuthInterceptor` before shipping.

### Running the test

```sh
./docs/devlogs/BDK-322/led-test-effects.sh DEVICE_IP
```

Requires `grpcurl`. Exercises all 7 effect types, brightness ramp, enable/ disable, and temp-effect expiry with a 3s
pause between steps for visual verification.
