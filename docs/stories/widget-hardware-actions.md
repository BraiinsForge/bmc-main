# Widget hardware actions

Widgets request system-level effects — sounds, LED patterns — through the `deck_widget_v1` Wayland protocol. They have
no direct hardware access; the compositor receives each request and routes it to the appropriate hardware controller.

## Widget stories

### Sound playback

> As a widget, I want to play a named sound (alarm, notification, etc.) so users get audio feedback for time-based or
> stateful events.

- Sound is identified by name from the system's known set; widgets don't ship audio files themselves.
- Playback is asynchronous: the request returns immediately, the sound finishes in its own task.
- A widget can stop the currently-playing sound at any time.

### LED effects

> As a widget, I want to drive the LED strip with a chosen effect and color so users get ambient visual feedback.

- Requests carry the effect type and RGB color.
- A finite-duration request (`LedTemporary`) auto-clears after the duration elapses.
- An indefinite request (`LedEndless`) holds the effect until cancelled.
- A widget can stop the current LED effect at any point.

## Protocol

Requests flow widget → compositor over `deck_widget_v1`:

- `request_action(PlaySound { sound })`
- `request_action(StopSound)`
- `request_action(LedTemporary { effect, color, duration_ms })`
- `request_action(LedEndless { effect, color })`
- `request_action(StopLed)`

The compositor packages each request as a `WidgetAction` carrying the originating widget's instance id and dispatches it
through an mpsc channel to the action handler. The handler matches on the action variant and invokes the corresponding
method on `SoundController` or `LedController`.

Sound playback runs in a separate task with a cancellation token so a long-running `play_sound()` doesn't block
subsequent LED or stop-sound requests.

## Constraints

- All requests are fire-and-forget; widgets receive no acknowledgement or completion signal. State queries ("is a sound
  playing?", "what is the current LED state?") are not part of this protocol.
- Sound names must match an entry in the system's known sounds set; unknown names are logged and dropped.
- Hardware controllers are shared across all widgets; there is no per-widget arbitration. The most recent action wins.
