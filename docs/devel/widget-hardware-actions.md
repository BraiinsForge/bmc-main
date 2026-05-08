# Widget hardware actions via `deck_widget_v1`

## Description

Widgets request system-level effects — playing a sound, driving the LED strip — through the `deck_widget_v1` Wayland
protocol. They have no direct hardware access; the compositor receives each request as an `ActionPayload`, packages it
with the originating widget's instance id as a `WidgetAction`, and forwards it over an mpsc channel. A long-lived action
handler task drains the channel and dispatches each request to `SoundController` or `LedController`.

LED requests are **acknowledged**: each `led_temporary`/`led_endless` carries a widget-allocated `request_id` and the
compositor replies with a `led_request_status` event when the request progresses (accepted, completed, or superseded).
Sound requests remain fire-and-forget.

## Wire surface

`bmc-widget-protocol::ActionPayload` covers all currently supported requests:

```
PlaySound    { sound: String }
StopSound    {}
LedTemporary { request_id: LedRequestId, effect: LedEffect, color: RgbColor, period_ms: u32, duration_ms: u32 }
LedEndless   { request_id: LedRequestId, effect: LedEffect, color: RgbColor, period_ms: u32 }
StopLed      { request_id: LedRequestId }
```

`LedEffect` on the wire is unit-typed and the color travels separately. The hardware enum (`bmc_led::data::LedEffect`)
folds the color into each variant. The conversion lives in `proto_to_hw_effect` in `bmc/src/widget/action_handler.rs` so
future call sites reuse the mapping rather than duplicating the match.

`period_ms` controls per-effect animation speed; `0` means "use the effect's default" and translates to `period: None`
on the `LedScene` passed to the LED driver. Effects without a notion of period (e.g. `Solid`) ignore the value.

### `request_id` and status replies

`request_id` is widget-allocated from a u32 namespace scoped to the calling surface; uniqueness on the host side is
keyed on `(instance_id, request_id)`. The reserved value `0` (`LED_REQUEST_ID_ALL`) is invalid for `led_temporary` and
`led_endless` (the compositor drops such requests with a warning) and on `stop_led` means "cancel every outstanding LED
request from this widget."

For each LED start request, the compositor emits zero or more `led_request_status` events back on the originating widget
surface, echoing the `request_id`. `stop_led` itself is always accepted and never produces its own reply — but cancelled
requests receive a `superseded` status under their original ids.

```
LedRequestStatus
├── Accepted    queued or activated; followed eventually by Completed or Superseded
├── Rejected    reserved; no current code path emits this
├── Superseded  cancelled before completing (stop_led, replaced endless, widget disconnect)
└── Completed   led_temporary ran for its full duration
```

`led_endless` requests never receive `Completed` — they end with `Superseded` once cancelled or replaced.

## Dispatch architecture

```
compositor → mpsc<WidgetAction>         → action_handler task
                                            ├─► led queue        ──► LedController::send_command
                                            │     │
                                            │     └─► tokio::time::sleep_until(active.until)
                                            │
                                            └─► sound manager task ──► SoundController::play_sound

action_handler ──── mpsc<WidgetRequestStatus> ──── compositor → led_request_status event on widget surface
```

`spawn_action_handler` runs once during startup, between `LedController::new` and `Coordinator::new`. It owns the action
receiver and the status sender from the compositor and lives for the lifetime of the `bmc-openwrt` process.

### LED queue

The action handler holds:

- `queue: VecDeque<TempEntry>` — pending `LedTemporary` requests waiting their turn
- `active_temp: Option<ActiveTemp>` — the temporary currently playing on the strip; carries the `tokio::time::Instant`
  at which its duration elapses
- `active_endless: Option<ActiveEndless>` — the most recent `LedEndless` request, if any

Per-message handling:

- **`LedTemporary`** — emit `Accepted`. If `active_temp` is empty, push the scene to the driver and start a
  `tokio::time::sleep_until` timer for `duration_ms`. Otherwise queue.
- **`LedEndless`** — if a previous endless is held, emit `Superseded` for it. Push the new scene to the driver as
  `LedScene { duration: None, … }` (the driver's persistent slot) and emit `Accepted`.
- **`StopLed { request_id }`** — match any (`instance_id`, `request_id`) the calling widget owns. `request_id == 0`
  matches all of the widget's outstanding requests. Each cancelled request emits `Superseded`. If the active endless was
  cancelled, a `None` scene is sent to clear the driver's persistent slot.
- **Active temporary expiry** — `tokio::time::sleep_until` fires; emit `Completed`, advance to the next queue entry (if
  any).

The `select!` in the main loop alternates between `action_rx.recv()` and the active temporary's expiry timer.

The driver's existing `temporary` / `persistent` slot model already implements the "return to whatever LED state was
active before the request" semantics — the queue here only adds *serialization* between successive temporaries from
multiple widgets, which the single-slot driver alone cannot provide.

#### Cancelling the active temporary — v1 limitation

`bmc_led::data::LedCommand` has no "clear temporary" variant today. When `StopLed` cancels the *active* temporary and
the queue is empty, the action handler advances its bookkeeping (so a follow-up `Completed` does not fire), but the
driver's temporary slot keeps running until its natural duration elapses. A future fix should either add a clear command
or a zero-duration sentinel to `LedCommand`. Cancelling the active *endless* is unaffected (the persistent slot is
overwritten with `None`).

### Sound manager

The sound manager owns the currently-active `CancellationToken` and serializes playback:

- `PlaySound { sound }` cancels the in-progress token (if any), looks up the sound name via `Sounds::from_str`, and
  spawns a fresh task that calls `controller.play_sound(sound, token)`. New replaces old immediately; widgets don't have
  to wait for the previous clip to finish.
- `StopSound` cancels the active token without spawning a replacement.
- Unknown sound names are logged and dropped (the widget is authoritative on what it asks for, but the system is
  authoritative on what it ships).

The sound command channel is a bounded mpsc (`SOUND_CHANNEL_CAPACITY = 4`) — backpressure is acceptable here because
sound playback is a single shared resource, and dropping requests under sustained pressure is preferable to unbounded
queueing.

## Constraints

- Sound names must match an entry in the system's known sounds set; mismatches are silently dropped.
- The action handler holds the only consumer of the compositor's action receiver — calling
  `compositor.action_receiver()` twice will panic, so wiring must happen exactly once during startup.
- LED `request_id == 0` is reserved on the start requests; the compositor drops them with a warning. On `stop_led` it
  means "cancel all of this widget's outstanding requests."
- `led_endless` is forwarded straight to the driver's persistent slot, so a fresh endless replaces the previous one
  immediately. The previously-active endless still receives a `Superseded` reply.
- A widget that disconnects mid-flight does *not* currently get its outstanding LED requests cancelled — surfacing
  `WidgetDisconnected` to the action handler is a follow-up to the permissions framework.
