# Widget hardware actions via `deck_widget_v1`

## Description

Widgets request system-level effects — playing a sound, driving the LED strip — through the `deck_widget_v1` Wayland
protocol. They have no direct hardware access. A widget calls into the wasm guest SDK; the host runtime forwards the
call as a typed `LedRequest`/sound action to the widget's wayland surface, which serializes it onto `deck_widget_v1`;
the compositor receives the request, attaches the originating widget's `InstanceId`, and forwards it over an mpsc
channel; a long-lived action handler task drains the channel and dispatches to `SoundController` or to a scene-aware
LED state manager.

LED requests are **acknowledged**: each `led_temporary`/`led_endless` carries a widget-allocated `request_id` and the
compositor relays a `led_request_status` event whenever the request advances (accepted, completed, or superseded).
Sound requests remain fire-and-forget.

## End-to-end flow

```
┌─ wasm guest ──────────────────────────────────────────────────────┐
│  led::set_effect(Effect, Color, period_ms, Option<u32>)           │
│     │                                                             │
│     └─► host_led_set_endless / host_led_set_temporary  (FFI: u8)  │
└──────────────────────────┬────────────────────────────────────────┘
                           ▼
┌─ host runtime (bmc-wasm-runtime) ─────────────────────────────────┐
│  imports/led.rs    LedEffect::try_from(u8)                        │
│                    led_request_alloc.alloc()    (per-guest u32)   │
│                    mpsc::Sender<LedRequest> ─┐                    │
└──────────────────────────────────────────────┼────────────────────┘
                                               ▼
┌─ widgets/wasm (wasm widget binary, wayland client) ───────────────┐
│  wayland.rs::flush_led_requests                                   │
│    led_request_to_action(req) → ActionPayload                     │
│    DeckWidgetSurfaceClient::request_action  (wayland call)        │
└──────────────────────────┬────────────────────────────────────────┘
                           ▼
┌─ compositor (bmc-openwrt) ────────────────────────────────────────┐
│  protocol/dispatch.rs   led_effect_from_protocol(P → wpb)         │
│                         WidgetAction { instance_id, payload }     │
│                         mpsc::Sender<WidgetAction>  ─┐            │
└──────────────────────────────────────────────────────┼────────────┘
                                                       ▼
┌─ bmc (action handler task) ───────────────────────────────────────┐
│  widget/action_handler.rs                                         │
│    PlaySound/StopSound  ──► sound manager task ─► SoundController │
│    LedTemporary         ─┐                                        │
│    LedEndless           ─┼──► LedSceneManager ─► LedController    │
│    StopLed              ─┘     │                                  │
│  CompositorEvent::ActiveSceneChanged → manager.on_scene_changed   │
│  CompositorEvent::WidgetDisconnected → manager.on_widget_disconn. │
│                                                                   │
│  LedSceneManager ──► status_tx ──► compositor                     │
│                       └─► led_request_status event on widget      │
└───────────────────────────────────────────────────────────────────┘
```

## Wire surface

`bmc-widget-protocol::ActionPayload` covers every action a widget can issue:

```
PlaySound    { sound: String }
StopSound    {}
LedTemporary { request_id: LedRequestId, effect: LedEffect, color: RgbColor, period_ms: u32, duration_ms: u32 }
LedEndless   { request_id: LedRequestId, effect: LedEffect, color: RgbColor, period_ms: u32 }
StopLed      { request_id: LedRequestId }
```

`LedEffect` on the wire is unit-typed; the color travels separately. The wire enum discriminants are pinned by
`deck-widget-v1.xml`: `chase=0, knight_rider=1, scan=2, snake=3, breathe=4, solid=5`. `period_ms` controls per-effect
animation speed; `0` means "use the effect's default" and translates to `period: None` on the `LedScene` passed to the
driver. Effects without a notion of period (e.g. `Solid`) ignore the value.

### `request_id` semantics

`request_id` is widget-allocated from a `u32` namespace scoped to the calling surface; uniqueness on the host side is
keyed on `(instance_id, request_id)`. The reserved value `0` (`LED_REQUEST_ID_ALL`) is invalid for `led_temporary` and
`led_endless` (the compositor drops such requests with a warning) and on `stop_led` means "cancel every outstanding LED
request from this widget."

The wasm runtime allocates ids on behalf of guests via a per-`HostState` monotonic counter starting at 1 and wrapping
past `u32::MAX` back to 1 (skipping the reserved 0). The guest SDK never sees ids; `stop()` always emits
`StopLed { request_id: 0 }`.

### Status replies (LED only)

For each LED start request, the compositor emits zero or more `led_request_status` events back on the originating
widget surface, echoing the `request_id`.

```
LedRequestStatus
├── Accepted    queued or activated; followed eventually by Completed or Superseded
├── Rejected    reserved; no current code path emits this
├── Superseded  cancelled before completing (stop_led, replaced endless, widget disconnect)
└── Completed   led_temporary ran for its full duration
```

`led_endless` requests never receive `Completed` — they end with `Superseded` once cancelled or pushed down the endless
stack. `stop_led` itself is always accepted and never produces its own reply; cancelled requests receive `Superseded`
under their original ids.

## WASM guest side

### Guest SDK (`bmc-wasm-runtime/sdk/src/led.rs`)

```rust
pub fn set_effect(effect: LedEffect, color: Color, period_ms: u32, duration_ms: Option<u32>);
pub fn stop();
```

`duration_ms = None` → endless; `Some(n)` → temporary for `n` ms (including `Some(0)` — a zero-duration temporary the
host fires and immediately expires). `stop()` cancels every LED request this widget has outstanding; there is no
`LedEffect::None` variant — `stop()` is the canonical off path. The SDK does not auto-stop a previous endless on a
fresh `set_effect`: the protocol already supersedes endlesses and queues temporaries, and the SDK does not hide that.

The `Color` and `LedEffect` types come from `bmc-wasm-protocol`. `LedEffect` is `#[repr(u8)]` with discriminants pinned
to the wire enum; a `discriminants_match_protocol` test catches drift.

The corresponding sound SDK is `bmc-wasm-runtime/sdk/src/audio.rs` (`play_sound(name)`, `stop_sound()`).

### Host imports (`bmc-wasm-runtime/src/runtime/imports/led.rs`)

```
host_led_set_endless   (effect: u32, r: u32, g: u32, b: u32, period_ms: u32)
host_led_set_temporary (effect: u32, r: u32, g: u32, b: u32, period_ms: u32, duration_ms: u32)
host_led_stop          ()
```

Two distinct imports for the two effect shapes keep `None` and `Some(0)` distinguishable on the wire without a
sentinel. The handler:

1. Decodes the effect byte with `LedEffect::try_from(effect as u8)`; unknown discriminants are logged and dropped.
2. Allocates a non-zero `request_id` via the per-`HostState` counter (only for `set_endless`/`set_temporary`; `stop`
   always sends `LED_REQUEST_ID_ALL`).
3. Records a `FixtureEventKind::LedSet{Endless,Temporary,Stop}` entry if recording is enabled.
4. Sends a `LedRequest::{SetEffect, Stop}` on the runtime's outbound channel.

`LedRequest` is the widget-perspective channel type. `SetEffect` carries `effect: LedEffect`, `color: Rgb`,
`period_ms: u32`, and `duration: Option<Duration>`; `Stop` carries the `request_id`.

### `widgets/wasm` forwarding (`widgets/wasm/src/wayland.rs`)

The widget binary owns the wayland-client lifecycle. On `connect()` it allocates an `mpsc::channel::<LedRequest>` and
stores both halves; on first runtime construction it moves the `Sender` into `RuntimeConfig::led_request_sender` via
`Option::take`. The main loop drains the `Receiver` immediately after `poll_dispatch`:

```rust
fn flush_led_requests(&mut self) -> Result<()> {
    while let Ok(req) = self.led_rx.try_recv() {
        let action = led_request_to_action(&req);
        self.surface.request_action(&action)?;
    }
    Ok(())
}
```

`led_request_to_action` is an inline helper, exhaustively matched on both the runtime `LedEffect` (6 variants, no
wildcard) and `LedRequest` (`SetEffect`/`Stop`). It maps `duration: None` → `ActionPayload::LedEndless`, `Some(d)` →
`ActionPayload::LedTemporary` with `u32::try_from(d.as_millis()).unwrap_or(u32::MAX)`. There is no separate translator
module.

`DeckWidgetSurfaceClient::request_action` (in `bmc-widget`) dispatches the typed `ActionPayload` onto the wire via
`surface.led_endless` / `surface.led_temporary` / `surface.stop_led` / `surface.play_sound` / `surface.stop_sound` and
flushes the connection. The wire effect byte is produced by `to_protocol::led_effect`, a named-variant match from
`bmc_widget_protocol::LedEffect` to the wayland-scanner-generated client enum.

## Compositor side (bmc-openwrt)

`bmc-openwrt/src/compositor/protocol/dispatch.rs` implements the server-side handlers for the `deck_widget_v1`
requests. Each LED start request is converted to a `bmc_widget_protocol::LedEffect` via `led_effect_from_protocol`
(named match), assembled into an `ActionPayload`, attached to the widget's `InstanceId` to form a `WidgetAction`, and
sent on the unbounded `mpsc<WidgetAction>` the compositor exposes.

LED status events flow the other way: `bmc` sends `WidgetRequestStatus { instance_id, request_id, status }` on an
unbounded mpsc; the compositor looks up the widget surface and emits a `led_request_status` event on it.

The compositor also publishes scene lifecycle on a broadcast channel: `ActiveSceneChanged { scene_id, widget_ids }`
when the user navigates between scenes, and `WidgetDisconnected { instance_id }` when a widget's wayland client goes
away.

## Action handler task (`bmc/src/widget/action_handler.rs`)

`spawn_action_handler` runs once during startup, between `LedController::new` and `Coordinator::new`. It owns the
compositor's action receiver, a clone of the compositor event broadcast receiver, and the status sender. It lives for
the lifetime of the `bmc-openwrt` process.

```
spawn_action_handler
├─ spawn_sound_manager(rx, sound_controller)              // separate task
└─ tokio::spawn(async move {
     let mut led = LedSceneManager::new(led_controller, status_tx);
     loop tokio::select! {
       biased;
       () = sleep_until(led.active_deadline())  =>  led.on_active_expiry();
       action = action_rx.recv()                =>  dispatch ActionPayload
       event  = event_rx.recv()                 =>  dispatch CompositorEvent
     }
   })
```

- `PlaySound`/`StopSound` → `sound_tx.try_send(SoundCommand::Play|Stop)` on a bounded channel
  (`SOUND_CHANNEL_CAPACITY = 4`).
- `LedTemporary` → `led.on_temporary(instance_id, request_id, effect, color, period_ms, duration_ms)`.
- `LedEndless` → `led.on_endless(...)`.
- `StopLed { request_id }` → `led.on_stop(&instance_id, request_id)`.
- `ActiveSceneChanged` → `led.on_scene_changed(scene_id, widget_ids)`.
- `WidgetDisconnected` → `led.on_widget_disconnected(&instance_id)` (equivalent to a `stop_led(0)` from that widget).

The `biased` select gives the expiry timer priority over new actions so a `Completed` reply is not delayed by a burst
of incoming requests.

### Sound manager

A separate task owns the currently-active `CancellationToken` and serializes playback:

- `PlaySound { sound }` cancels the in-progress token (if any), looks up the sound name via `Sounds::from_str`, and
  spawns a fresh task that calls `controller.play_sound(sound, token)`. New replaces old immediately; widgets do not
  have to wait for the previous clip to finish.
- `StopSound` cancels the active token without spawning a replacement.
- Unknown sound names are logged and dropped (the widget is authoritative on what it asks for; the system is
  authoritative on what it ships).

The sound command channel is a bounded mpsc — backpressure is acceptable because sound playback is a single shared
resource, and dropping requests under sustained pressure is preferable to unbounded queueing.

## LED scene manager (`bmc/src/widget/led_state.rs`)

`LedSceneManager` is **scene-aware**: every widget action is associated with the scene the widget belongs to
(`widget_to_scene: HashMap<InstanceId, SceneId>`), and effects only drive the strip while their scene is the active
one. The manager's internal state is per-scene:

```
SceneEffectState
├── endless_stack:  Vec<EndlessEntry>      // top = currently-applied endless on this scene
├── temp_queue:     VecDeque<TempEntry>    // pending temporaries waiting their turn
└── active_temp:    Option<ActiveTemp>
                       ├─ Running { entry, until: Instant }   // playing on the strip (active scene)
                       └─ Paused  { entry }                   // scene not active; remaining preserved
```

### Endless: stack semantics

Successive `led_endless` requests for the same scene push onto `endless_stack`. The new top supersedes the previous
top (which receives `Superseded`); only the top is applied to the strip. `stop_led` removes the matching entry from
the stack — if the removed entry was the top, the new top is reapplied; if the stack becomes empty (and no temporary
is running), the strip is cleared.

### Temporary: per-scene queue

`led_temporary` requests start immediately if the active scene matches and that scene has no running temporary and no
queued temporary; otherwise they're appended to the scene's `temp_queue`. The `Running { until }` slot drives the
manager's `active_deadline()`, which the action-handler loop sleeps on. When the timer fires, `on_active_expiry`
emits `Completed`, drains the next queue entry, or falls back to the top of the endless stack.

### Scene change: pause / resume

`on_scene_changed(scene_id, widget_ids)` pauses the previous active scene's running temporary (preserving its
remaining duration) and applies the new scene's effect: a paused temporary on the new scene is resumed with its
remaining time; otherwise the next queued temporary starts; otherwise the new scene's endless-stack top is applied;
otherwise the strip is cleared.

### Widget disconnect

`on_widget_disconnected(&instance_id)` is equivalent to `stop_led(0)` from that widget — every outstanding request the
widget owns is removed and replied to with `Superseded` — plus the widget's `widget_to_scene` mapping is dropped.

## Discriminant pinning across the chain

Every conversion site between adjacent enums is a **named-variant match**, not a numeric cast:

```
bmc-wasm-protocol::LedEffect        (#[repr(u8)] Chase=0..Solid=5)
  ↓  imports/led.rs:84               LedEffect::try_from(u8)
                                     match (in widgets/wasm)
bmc_widget_protocol::LedEffect      (#[derive] Chase..Solid)
  ↓  bmc-widget::to_protocol::led_effect       match
deck_widget_surface_v1::LedEffect    (wayland-scanner; XML pins values)
  ───────  wayland wire (u32)  ───────
deck_widget_surface_v1::LedEffect   (server side, same XML)
  ↓  bmc-openwrt led_effect_from_protocol      match
bmc_widget_protocol::LedEffect      (host side)
  ↓  bmc led_state.rs:proto_to_hw_effect       match
bmc_led::data::LedEffect            (variant carries Rgb)
  ↓  apa102_spi/platform_led_driver.rs:149     match
hardware effects
```

`bmc_led::data::LedEffectKind` has its own `#[repr(u8)]` enum with **different** discriminants (`None=0, Chase=1, …`)
used for compact serialization in the hardware driver, but those discriminants never appear on the wire — every
crossing into and out of `bmc_led::data::LedEffect` is a named match.

The only numeric crossing on the production path is the wasm FFI boundary (guest `effect as u8` → host
`LedEffect::try_from(u8)`), and there the same `bmc-wasm-protocol::LedEffect` type sits on both sides.

## Constraints

- Sound names must match an entry in the system's known sounds set; mismatches are silently dropped.
- The action handler holds the only consumer of the compositor's action receiver — calling
  `compositor.action_receiver()` twice will panic, so wiring must happen exactly once during startup.
- LED `request_id == 0` is reserved on start requests; the compositor drops them with a warning. On `stop_led` it
  means "cancel all of this widget's outstanding requests."
- A widget that issues `led_temporary` or `led_endless` before its surface has appeared in any
  `ActiveSceneChanged.widget_ids` is silently dropped with a warning — the manager refuses requests it cannot place
  on a scene.
- `bmc_led::data::LedCommand` has no "clear active temporary" variant: when `stop_led` cancels the *active* temporary
  on the active scene, the manager advances its bookkeeping immediately but the driver's temporary slot keeps running
  until its natural duration elapses. Cancelling the *active endless* (via stack removal) is unaffected — the new
  top, or `None`, overwrites the persistent slot.
