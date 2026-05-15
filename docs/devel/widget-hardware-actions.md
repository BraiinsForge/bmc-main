# Widget hardware actions via `deck_widget_v1`

## Description

Widgets request system-level effects — playing a sound, driving the LED strip — through the `deck_widget_v1` Wayland
protocol. They have no direct hardware access; every effect a widget asks for travels over the wire and is realised by
the system on the widget's behalf.

LED requests are **acknowledged**: each `led_temporary`/`led_endless` carries a widget-allocated `request_id` and the
system replies with `led_request_status` events as the request advances (accepted, completed, or superseded). Sound
requests are fire-and-forget.

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
                           │  deck_widget_v1 requests
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

`duration_ms = None` → endless effect; `Some(n)` → temporary for `n` ms (including `Some(0)` — fires and immediately
expires). `stop()` cancels every LED request this widget has outstanding (both local and global); there is no
`LedEffect::None` variant — `stop()` is the canonical off path. The SDK does not auto-stop a previous endless on a fresh
`set_effect`: the protocol already supersedes endlesses and queues temporaries (see *Effect lifecycle* below), and the
SDK does not hide that.

`set_effect_global` is the companion for *ambient* signals — effects that should fall to the foreground only when no
scene-specific effect is competing for the strip (charging indicator, OTA-in-progress pulse, idle-network glow). See
*Scope: local vs global* under *Effect lifecycle*.

The `Color` and `LedEffect` types come from `bmc-wasm-protocol`. `LedEffect` is `#[repr(u8)]` with discriminants pinned
to the wire enum; a `discriminants_match_protocol` test catches drift.

`request_id`s are allocated by the host runtime on behalf of guests; the guest SDK never sees them. `stop()` always
cancels every outstanding LED request from the calling widget, in both tiers.

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
`deck-widget-v1.xml`: `chase=0, knight_rider=1, scan=2, snake=3, breathe=4, solid=5`. `period_ms` controls per-effect
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
├── Accepted    queued or activated; followed eventually by Completed or Superseded
├── Rejected    reserved; no current code path emits this
├── Superseded  cancelled before completing (stop_led, replaced endless, widget disconnect)
└── Completed   led_temporary ran for its full duration
```

`led_endless` requests never receive `Completed` — they end with `Superseded` once cancelled or pushed down the endless
stack. `stop_led` itself is always accepted and never produces its own reply; cancelled requests receive `Superseded`
under their original ids.

## Effect lifecycle

The rules below are observable behavior of the protocol — what a widget sees in response to its calls.

### Scope: local vs global

Every LED start request carries a `scope`. The two tiers run side by side with a fixed priority rule: **local always
wins over global.** Local effects drive the strip whenever they have something to play; global effects only become
visible when the active scene's local tier is empty.

The two tiers each have their own endless stack and temporary queue. A widget can use both tiers concurrently — its
`request_id` namespace covers both — and `stop()` / `stop_led(0)` clears the widget from both.

Cross-tier displacement is **a display state, not a lifecycle state.** If a local effect arrives while a global one is
on the strip, the global temp/endless stays in its tier's state but is not drawn. A running global temporary in this
position **pauses** — its remaining time is preserved — and resumes when local activity clears. No `Superseded` is
emitted; the global request will run for its full duration whenever priority allows. `Superseded` is reserved for
*same-tier* displacement: a new endless pushing the previous one down the stack, an explicit `stop_led`, or widget
disconnect.

Permissions are not enforced today. Any widget can flag a request as global; a manifest-driven capability filter is a
follow-up.

### Endless effects: stack

Each tier has its own endless stack. For local, the stack is per-scene. Successive `led_endless` requests on the same
tier push onto its stack — the new top supersedes the previous top (which receives `Superseded`); only the top is
considered for display. `stop_led` removes the matching entry from whichever tier holds it; if the removed entry was the
top, the next-most-recent entry becomes the new top.

### Temporary effects: queue

Each tier has its own temporary queue. `led_temporary` requests run for their `duration_ms`. If a temporary is already
running or queued *on the same tier* (for local, on the widget's scene; for global, anywhere), the new one waits its
turn. When a temporary completes, its `Completed` event fires and the next queued temporary on that tier starts; if the
tier's queue is empty, its endless-stack top is reapplied.

### Scene gating

Local effects only drive the strip while the widget's scene is the active scene. When the user navigates away:

- A running *local* temporary is **paused**; its remaining duration is preserved.
- When the scene becomes active again, the paused temporary resumes; otherwise the next queued local temporary starts;
  otherwise the scene's local endless-stack top is reapplied; otherwise the chain falls through to the global tier
  (paused global temporary resumes, queued global temp starts, or global endless-stack top applies); otherwise the strip
  is cleared.

Globals have no scene affinity — they apply regardless of which scene is active, subject only to the local-wins rule.

A widget that issues `led_temporary` or `led_endless` **with `scope = local`** before its surface has been associated
with any scene gets the request dropped with a warning. Global requests bypass this check and land regardless of widget
placement.

### Widget disconnect

When a widget's wayland client goes away, every outstanding LED request from that widget is cancelled — across both
tiers — equivalent to `stop_led(0)` from that widget, and each cancelled request receives `Superseded`.

## Sound

Sound playback is a single shared resource. A new `play_sound` cancels any in-progress clip immediately; widgets do not
wait for the previous clip to finish. `stop_sound` cancels the active clip without starting a replacement. Sound names
must match an entry in the system's known sounds set; unknown names are silently dropped.

WASM widgets typically use the runtime's audio path (register a sample, then `audio_play(id, volume)`), which plays
locally in the widget runtime rather than emitting `play_sound` over `deck_widget_v1`. The wire-level
`PlaySound`/`StopSound` surface is shared with non-WASM widgets and remains available for system-sound playback.

## Constraints

- LED start requests with `request_id == 0` are dropped.
- A widget that issues *local* LED requests before its surface has been attached to a scene gets the request dropped.
  Global requests are unaffected.
- The global tier is unguarded today — any widget may use `set_effect_global` / `scope = global`. A manifest-driven
  capability filter is a follow-up.
- Sound names not in the system's known sounds set are silently dropped (the widget is authoritative on what it asks
  for; the system is authoritative on what it ships).
- Cancelling the *active* temporary on the active scene is acknowledged but not visually immediate: `Superseded` fires
  right away, but the strip keeps showing the cancelled temporary until its natural duration would have elapsed.
  Cancelling the *active endless* (via stack removal) is immediate — the new top, or empty, takes effect on the strip
  right away.
