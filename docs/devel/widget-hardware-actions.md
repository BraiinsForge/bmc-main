# Widget hardware actions via `deck_widget_v1`

## Description

Widgets request system-level effects — playing a sound, driving the LED strip — through the `deck_widget_v1` Wayland
protocol. They have no direct hardware access; the compositor receives each request as an `ActionPayload`, packages it
with the originating widget's instance id as a `WidgetAction`, and forwards it over an mpsc channel. A long-lived action
handler task drains the channel and dispatches each request to `SoundController` or `LedController`.

The widget side is fire-and-forget: requests carry no acknowledgement and there is no completion stream. Hardware
controllers are shared across all widgets; the most recent request wins.

## Wire surface

`bmc-widget-protocol::ActionPayload` covers all currently supported requests:

```
PlaySound    { sound: String }
StopSound    {}
LedTemporary { effect: LedEffect, color: RgbColor, period_ms: u32, duration_ms: u32 }
LedEndless   { effect: LedEffect, color: RgbColor, period_ms: u32 }
StopLed      {}
```

`LedEffect` on the wire is unit-typed and the color travels separately. The hardware enum (`bmc_led::data::LedEffect`)
folds the color into each variant. The conversion lives in `proto_to_hw_effect` in `bmc/src/widget/action_handler.rs` so
future call sites reuse the mapping rather than duplicating the match.

`period_ms` controls per-effect animation speed; `0` means "use the effect's default" and translates to `period: None`
on the `LedScene` passed to the LED driver. Effects without a notion of period (e.g. `Solid`) ignore the value.

## Dispatch architecture

```
compositor → mpsc<WidgetAction> → action_handler task
                                       │
                                       ├── Led*  ──► LedController::send_command (synchronous)
                                       │
                                       └── *Sound ─► sound manager task ──► SoundController::play_sound
```

`spawn_action_handler` runs once during startup, between `LedController::new` and `Coordinator::new`. It owns the action
receiver from the compositor and lives for the lifetime of the `bmc-openwrt` process.

LED actions complete in microseconds (a single `LedCommand` push to the controller's queue) so the action handler
processes them inline. Sound actions delegate to a dedicated **sound manager task** because `play_sound` blocks until
the audio finishes — running it inline would stall LED and stop-sound processing for the duration of every clip.

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

- Requests are fire-and-forget. There is no per-widget arbitration; if two widgets compete for the LED strip the most
  recent request wins.
- Sound names must match an entry in the system's known sounds set; mismatches are silently dropped.
- The action handler holds the only consumer of the compositor's action receiver — calling
  `compositor.action_receiver()` twice will panic, so wiring must happen exactly once during startup.
