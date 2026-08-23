# Widget hardware actions via `deck_widget`

## Description

Widgets request system-level effects — playing a sound, driving the LED strip — through the `deck_widget` Wayland
protocol. They have no direct hardware access; every effect a widget asks for travels over the wire and is realised by
the system on the widget's behalf.

LED requests are **acknowledged**: each `led_temporary`/`led_endless` carries a widget-allocated `request_id` and the
system replies with `led_request_status` events as the request advances (accepted, then eventually expired or
superseded). Sound requests are fire-and-forget.

## End-to-end flow

```
┌─ wasm guest ──────────────────────────────────────────────────────┐
│  led::set_effect(Effect, Color, period_ms, Option<u32>)           │
│  led::stop()                                                      │
│  audio::audio_play(AudioId, Volume) / audio::audio_stop(AudioId)  │
└──────────────────────────┬────────────────────────────────────────┘
                           │  FFI: host_led_set_endless / host_led_set_temporary
                           │       host_led_stop
                           │       host_audio_play  / host_audio_stop
                           ▼
┌─ wasm host runtime ───────────────────────────────────────────────┐
│  decode effect byte; allocate request_id (LED only)               │
└──────────────────────────┬────────────────────────────────────────┘
                           │  deck_widget requests
                           ▼  ▲  led_request_status events
                           │  │
                       ── system ──
```

## WASM guest SDK

```rust
// LED
pub fn set_effect(effect: LedEffect, color: Color, period_ms: u32, duration_ms: Option<u32>);
pub fn set_effect_global(effect: LedEffect, color: Color, period_ms: u32, duration_ms: Option<u32>);
pub fn stop();
```

`duration_ms = None` → endless effect; `Some(n)` → temporary for `n` ms. `stop()` cancels every LED request this widget
has outstanding (both local and global) and turns the effects off. The SDK does not auto-stop a previous endless on a
fresh `set_effect`: the protocol already supersedes endlesses and queues temporaries (see *Effect lifecycle* below), and
the SDK does not hide that.

`set_effect_global` is the companion for *ambient* signals — effects that should fall to the foreground only when no
scene-specific effect is competing for the strip (e.g. a pomodoro timer's running indicator). See *Scope: local vs
global* under *Effect lifecycle*.

`Color` comes from `bmc-wasm-protocol`; `LedEffect` is `bmc-led`'s `LedEffectKind`, re-exported by the SDK.
`LedEffectKind` is `#[repr(u8)]`; an `effect_kind_wire_bytes_are_stable` test pins the discriminant values (kept equal
to the wire enum by hand).

`request_id`s are allocated by the host runtime on behalf of guests; the guest SDK does not expose them today. `stop()`
always cancels every outstanding LED request from the calling widget, in both tiers. Per-request cancellation is a
planned SDK extension.

## Wire surface

`bmc-widget-protocol::ActionPayload` is the typed envelope for every action a widget can issue:

```
PlaySound    { sound: String }
StopSound    {}
LedTemporary { request_id, effect, color, period_ms, duration_ms, scope: LedScope }
LedEndless   { request_id, effect, color, period_ms, scope: LedScope }
StopLed      { request_id: LedRequestId }
```

`LedEffect` on the wire is unit-typed; the color travels separately. The wire enum discriminants are pinned by
`deck-widget.xml`: `chase=0, knight_rider=1, scan=2, snake=3, breathe=4, solid=5`. `period_ms` controls per-effect
animation speed; `0` means "use the effect's default". Effects without a notion of period (e.g. `Solid`) ignore the
value.

`scope` is `led_scope.local` (default, scene-scoped) or `led_scope.global` (ambient, runs only when nothing local is
playing). `stop_led` carries no scope — `request_id`s are unique within a widget's surface namespace, so a single id
lookup finds the request in whichever tier it lives.

### `request_id` semantics

`request_id` is a `u32` scoped to the calling widget surface; uniqueness is keyed on `(widget, request_id)`. The
reserved value `0` (`LED_REQUEST_ID_ALL`):

- is invalid on `led_temporary` and `led_endless` (the request is dropped with a warning),
- on `stop_led` means "cancel every outstanding LED request from this widget."

When the WASM SDK is in use, ids are allocated automatically.

### Status replies (LED only)

For each LED start request, zero or more `led_request_status` events arrive back on the originating widget surface,
echoing the `request_id`:

```
LedRequestStatus
├── Accepted    queued or activated
├── Rejected    reserved; no current code path emits this
├── Superseded  request displaced from its tier or cancelled via stop_led; will not come back
└── Expired     the led_temporary's run ended (duration elapsed, or its scene was left)
```

`led_endless` requests never receive `Expired` — they end with `Superseded` when they are displaced from their tier: by
`stop_led`, by another endless (same widget or different) landing on the same tier under last-write-wins, or by the
widget disconnecting. `stop_led` itself is always accepted and never produces its own reply; cancelled requests receive
`Superseded` under their original ids.

## Effect lifecycle

The rules below are observable behavior of the protocol — what a widget sees in response to its calls.

### Scope: local vs global

Every LED start request carries a `scope`. The two tiers run side by side with a fixed priority rule: **local always
wins over global.** Local effects drive the strip whenever they have something to play; global effects only become
visible when the active scene's local tier is empty.

The two tiers each have their own endless slot and temporary queue. A widget can use both tiers concurrently — its
`request_id` namespace covers both — and `stop()` / `stop_led(0)` clears the widget from both.

Priority arbitration lives in `LedCoordinator`'s layer ordering, not in `LedSceneManager`. The manager publishes the
local winner to the `LocalScene` layer and the global winner to the `GlobalAmbient` layer; the coordinator picks the
higher-priority filled one. `Superseded` is reserved for *same-tier* displacement: a new endless taking the slot, an
explicit `stop_led`, or widget disconnect. Cross-tier loss is invisible at the lifecycle boundary — the layer simply
doesn't make it to the strip while a higher one is filled.

Permissions are not enforced today. Any widget can flag a request as global; a manifest-driven capability filter is a
follow-up.

### Endless effects: single slot, last-write-wins

Each tier holds a single endless slot. For local, the slot is per-scene. A new `led_endless` on a tier that already
holds one displaces the prior holder: the prior entry receives `Superseded` and is gone. The rule is uniform regardless
of which widget owned the prior slot — same-widget self-replacement and cross-widget takeover follow the same path. The
displaced request does not come back.

`stop_led` clears the matching entry from its tier, again emitting `Superseded` under the original id. There is no
revival of any earlier holder; the strip falls back to whatever the temp queue or another tier provides.

### Temporary effects: queue

Each tier has its own temporary queue. A temp runs for its `duration_ms` only once it reaches the active slot, and a
local temp reaches that slot only while its scene is the active scene.

When a scene becomes active — and each time its current temp finishes — the next queued temp is promoted: its deadline
is set to `now + duration_ms` and it plays. On expiry it emits `Expired` and the next temp starts; when the queue
drains, the tier's endless slot (if any) is reapplied. A temp queued for a scene that is not active simply waits — its
clock does not start until its scene is active, so the moment of submission is irrelevant.

The global tier has no scene affinity: its single queue advances continuously, independent of which scene is active.

### Scene gating

Local effects only drive the strip while the widget's scene is the active scene. The local tier's contribution to the
strip (`LocalScene` layer) goes empty when the user navigates to a scene whose own local tier has nothing to play — the
global tier then takes over (`GlobalAmbient` layer), since `LocalScene` outranks `GlobalAmbient`.

A local temp never ticks off-screen and is never paused-and-resumed. Navigating away from a scene whose temp is mid-run
**drops** that temp and emits `Expired` — it does not resume on return; the scene's remaining queue stays put and plays
from its next entry the next time the scene is active. The lifecycle machine in `LedSceneManager` only ever advances the
active scene's local tier and the global tier. Cross-tier displacement is a separate `LedCoordinator`-layer concern.

Globals have no scene affinity — they apply regardless of which scene is active, subject only to the local-wins rule.

A widget that issues `led_temporary` or `led_endless` **with `scope = local`** must already be mapped to a scene. The
widget→scene mapping is derived entirely from the config snapshot, which is loaded before any widget runs, so a running
widget is always mapped. A local request from a widget absent from the mapping is rejected — a `Superseded` reply is
emitted and the request dropped — which in practice only happens for a widget being removed from config or one that was
never configured. Global requests have no scene affinity and land on the global tier regardless of widget placement.

### State tracking

`LedSceneManager` tracks the active scene and the set of connected widgets from the compositor's **current state**, not
from one-shot events: both arrive as latest-value updates that always reflect the present truth. A transient internal
hiccup therefore can neither strand the manager on a stale active scene nor leak a disconnected widget's effect — on
every update it reconciles, sweeping effects for any widget no longer connected.

### Widget disconnect

When a widget's wayland client goes away, every outstanding LED request from that widget is cancelled — across both
tiers — equivalent to `stop_led(0)` from that widget. The cancellation emits `Superseded` for each request, exactly as a
`stop_led(0)` would; the replies simply cannot reach the departed client.

## Sound

Sound playback is a single shared resource. A new `play_sound` cancels any in-progress clip immediately; widgets do not
wait for the previous clip to finish. `stop_sound` cancels the active clip without starting a replacement. Sound names
must match an entry in the system's known sounds set; unknown names are silently dropped.

WASM widgets typically use the runtime's audio path (register a sample, then `audio_play(id, volume)`), which plays
locally in the widget runtime rather than emitting `play_sound` over `deck_widget`. The wire-level
`PlaySound`/`StopSound` surface is shared with non-WASM widgets and remains available for system-sound playback.

### LED disabled at gRPC

`LedCoordinator::set_enabled(false)` turns the strip off at the hardware boundary. From the widget's perspective the
lifecycle is unchanged: `LedSceneManager` keeps queueing requests, ticking deadlines, and emitting
`Accepted`/`Superseded`/`Expired` events as if the strip were lit. A widget can drive its state machine off these events
without being entangled with the physical-on/off setting — which is intentional, since the disable bit is owned by the
user (via gRPC) and not by the widget. The trade-off is that a temporary on the active scene will `Expired` on schedule
whether or not the strip is lit.

## Constraints

- LED start requests with `request_id == 0` are dropped.
- A *local* LED request from a widget not present in the config's widget→scene mapping is rejected (`Superseded`). The
  mapping comes from config, which is known before widgets run, so this only occurs for removed or unconfigured widgets.
- The global tier is unguarded today — any widget may use `set_effect_global` / `scope = global`. A manifest-driven
  capability filter is a follow-up.
- Sound names not in the system's known sounds set are silently dropped (the widget is authoritative on what it asks
  for; the system is authoritative on what it ships).
- Cancelling the *active* temporary on the active scene is acknowledged but not visually immediate: `Superseded` fires
  right away, but the strip keeps showing the cancelled temporary until its natural duration would have elapsed.
  Cancelling the *active endless* is immediate — with the single slot cleared, the strip re-picks right away (the tier's
  temp queue, a lower tier, or empty).
