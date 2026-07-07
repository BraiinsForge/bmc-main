# BDK-521 — SDK poll staleness & shared stale-data overlay

Ticket: [BDK-521](https://braiins.atlassian.net/browse/BDK-521) (Story, epic BDK-212 Extensibility Framework).

## Context

Stale-data signaling is hand-rolled per widget today. The mining widgets share `widgets-wasm/lib/mining/src/overlay.rs`
(a red warning-icon "Stale data" banner on a solid `GRAY_100` rectangle), the weather widget carries its own copy
(`widgets-wasm/weather/src/render.rs`), and the image widget added a third on this branch (`widgets-wasm/image`, via
`Badge::Stale`). Each is a static label — it does not say how old the shown data is, and each widget decides on its own
when it is stale.

The stable 26.02 branch solved this better with a "Last refresh … ago" pill. This ticket ports that model: staleness is
computed **once** in the SDK poll engine, and widgets only ask `is_stale()` and place a shared, reusable, Carbon-aligned
overlay.

**Scope.** Everything is contained in this `bmc-main` checkout. The ticket also names ticker-list /
ticker-single-sparkline, but those live in the separate `deckfeeder` repo and are out of scope here. Migration targets
in this repo: **weather, mining-info, mining-clock, image**.

## Key decisions

- The overlay is built from a reusable **Carbon `Tag`** component, not a bespoke pill.
- Components follow the established **host-rendered node** pattern: the wasm SDK emits a thin semantic node across the
  FFI; `bmc-render` holds the real styling + paint (femtovg). `bmc-render` is wasm-agnostic; the SDK adds the FFI glue.
- The changing "N ago" text is a **declarative, host-ticked `RelativeTimeLive` node** (not a guest timer) — it rides the
  existing Visible-gated animation-wake path, so it never re-runs WASM and never wakes off-screen.
- Carbon **color** tokens already exist in the Rust layer (`protocol/src/colors.rs`); spacing/type do not, so those stay
  as px consts annotated with their Carbon token name (no spacing-scale module).
- Interpolation ("Last refresh " + the live duration) is the **embedder's** concern via composition; the
  `RelativeTimeLive` node renders only the duration.

## Design-system alignment (IBM Carbon)

The company uses Carbon across products; the Deck's Rust render layer already uses Carbon color tokens by name and value
(`GRAY_100 = #161616`, `ORANGE_40 = #FE8431`, `RED_50`, `BLUE_50`), and `bmc-render`'s `notification` component mirrors
Carbon's InlineNotification. The overlay is modeled on Carbon's **Tag**. Reference values pulled from the Carbon source
(`packages/styles/scss/components/tag/_tag.scss`):

- Shape: pill, **`border-radius: 16px`**; `min-inline-size 32px`, `max-inline-size 208px`.
- Heights: sm 18px · **md 24px (default)** · lg 32px.
- Type: **`label-01`** = 12px / 16px, regular.
- Padding-inline: **`$spacing-03` = 8px** (lg → `$spacing-04` = 12px).
- Optional leading icon 16px with **`$spacing-02` = 4px** gap.

The Rust side has no spacing/type token scale; the values above land on Carbon's scale (8px insets = `$spacing-03`, 4px
= `$spacing-02`) and are written as local consts annotated with the token name.

## Architecture: host-rendered node pattern

Node discriminants live in `protocol/src/nodes.rs` (`NODE_COLUMN = 0x00` … `NODE_PROGRESS_BAR = 0x0A`; next free
`0x0B`). The guest builds a `Node`, `tree.rs` serializes it, `bmc-render` reads the discriminant, builds a `TreeNode`,
and dispatches to a `components/<x>.rs` renderer. `notification` is the reference: `sdk/src/notification.rs` is a
28-line builder emitting `Node::Notification { … }`; `bmc-render/src/components/notification.rs` holds the styling +
femtovg paint.

## Components (layered)

### 1. `RelativeTimeLive` node — foundational primitive

Declarative, self-updating relative time (counts up or down). The guest declares an anchor + format once; the host
computes the text from its own clock and re-renders the cached tree — no WASM rerun.

Spec crossing the FFI:

```rust
RelTimeSpec {
    anchor:  SystemTime,     // reference epoch-second (unix_secs, i64 on the wire)
    format:  RelTimeFormat,  // REQUIRED (no default). Two independent axes, packed into one
                             //   wire byte: length = Short (`7m`) | Long (`7 minutes`);
                             //   segments = Single (`7m`) | Double (`7m 30s`).
    clamp:   Clamp,          // Auto (sign-flip, default) | ElapsedOnly | RemainingOnly
}
```

- **Direction** is the sign of `now − anchor`: `> 0` → past ("7m ago"), `< 0` → future ("in 7m"), crossing zero flips
  naturally. `clamp` pins a direction for widgets that must not flip through "now".
- **No prefix/suffix.** The node renders only the duration; any surrounding label is composed by the embedder (see §4).
- **Style** comes from context: inside a `Tag` the Tag supplies it; standalone is
  `Node::RelTime { spec, style: TextStyle }` carrying its own style like `text()`.
- **Ticking / power.** The node registers through the frame-schedule animation path (`frame_callback_enabled` /
  `animation_wants_immediate` / `next_frame_delay`), so `bmc-render`'s cached-tree replay
  (`host_api.rs::is_animation_only_frame`) refreshes it with no WASM rerun. Wakes are gated to the **Visible** lifecycle
  (`bmc-wasm-host/src/main_loop.rs:124-133`): off-screen, neighbor, and transitioning widgets do not tick. Next-boundary
  delay follows the `segments` axis: `Single` wakes at the largest shown unit's boundary (1s / 60s / 3600s / 86400s by
  band), `Double` at the smaller segment's — so a `Single` pill wakes at most once a minute once past the seconds band.
- **Determinism.** The host formatter reads the same replay clock the animation system advances, so capture baselines
  stay deterministic.

Homes: `protocol/src/nodes.rs` (`NODE_RELTIME`); `bmc-wasm-runtime/sdk/src/relative_time.rs` (builder + wire write);
`bmc-render/src/components/relative_time.rs` (format logic + tick registration); `bmc-render/src/tree.rs` (read +
dispatch); `bmc-render/src/relative_time.stories.rs`.

Blast radius note: this is the one piece that reaches the runtime core — it touches the wire, the renderer, the frame
scheduler, and the capture harness.

### 2. `Tag` component — Carbon root component

```rust
tag(kind: TagKind, icon: Option<SvgId>, content: Node) -> Node
```

- The Tag owns the pill (background, `16px` radius, `$spacing-03` padding) + the leading icon; the **content is a `Node`
  composed and styled by the embedder** (so it can hold the live overlay label, a plain `text()`, or anything else).

- Theming mirrors Carbon's `tag-theme($bg, $color, $hover)` as a struct + per-kind map, exposed publicly so embedders
  style composed content consistently:

  ```rust
  pub struct TagTheme { background: Color, content: Color, icon: SvgId }  // hover dropped (no hover on Deck)
  pub fn tag_theme(kind: TagKind) -> TagTheme {
      Info    => { GRAY_100, BLUE_50,   INFO_FILLED }
      Warning => { GRAY_100, ORANGE_40, WARN_FILLED }   // exclamation-triangle (ICON_WARN_FILLED)
      Error   => { GRAY_100, RED_50,    ERROR_FILLED }
  }
  ```

  Solid `GRAY_100` background across variants because the tag floats over arbitrary widget content; Carbon's tinted
  pairings assume a solid surface and would wash out. Icon uses the per-kind default with an explicit `Option<SvgId>`
  override.

- Geometry consts (`TAG_RADIUS = 16.0`, `TAG_PAD = 8.0`, `TAG_ICON_GAP = 4.0`, …) sit in the component, each annotated
  with its Carbon token.

Homes: `protocol/src/nodes.rs` (`NODE_TAG`); `bmc-wasm-runtime/sdk/src/tag.rs` (builder + wire);
`bmc-render/src/components/tag.rs` (styling + measure + paint); `bmc-render/src/tree.rs` (read + dispatch);
`bmc-render/src/tag.stories.rs`.

### 3. Poll staleness — SDK

`bmc-wasm-runtime/sdk/src/poll.rs`:

- Per-poll `last_success_secs: Option<i64>` + `last_failed: bool`; `Config.stale_factor: f32` (default **1.5**,
  configurable).
- Clock by **injection** (mirrors the injected `FetchBackend`, keeps the registry unit-testable):
  `reschedule(handle, ok, now_secs, backend)` records `last_success_secs = now` on success and `last_failed = !ok`. The
  wasm trampoline passes `SystemTime::now().unix_secs`; tests pass a fake.
- `is_stale(now) = last_success_secs.is_some() && last_failed && age > stale_factor × interval_ms`. Requiring a prior
  success means a widget that never loaded doesn't flash "stale" (matches today). `last_success_epoch()` feeds the
  overlay's anchor.
- `Handle::is_stale()` / `Handle::last_success_epoch()` call `SystemTime::now()` internally.

### 4. Staleness overlay — feature layer

`bmc-wasm-runtime/sdk/src/stale_overlay.rs` composes the pieces:

```rust
// gated on poll.is_stale()
let s = tag_theme(TagKind::Warning).content;                 // ORANGE_40
let label = row(cross_align: Center, gap: 0, [
    text("Last refresh ", style(size: 12, color: s)),
    relative_time_live(anchor = poll.last_success_epoch(), RelTimeFormat { Short, Single }, style: (12, s)),
]);
with_placement(tag(Warning, /*default icon*/ None, label), placement)
```

- **Composition rationale.** `Span` is static-only (`sdk/src/text.rs:69`) and `CrossAlign` has no `Baseline` variant, so
  a true inline paragraph mix of static + live text is not available today. For a short, single-line, uniform-size pill
  label, a `row` of `[text, relative_time_live]` with `Center` cross-align is the clean-today path (center ≈ baseline at
  equal size). A live paragraph span (true inline flow) is deferred.
- **Placement** is a parameter with the existing default: bottom-left / 8px inset on rectangular faces,
  centered-and-lifted on round faces. Each migrated widget passes what fits, tuned visually.

Story: `bmc-wasm-runtime/sdk/src/stale_overlay.stories.rs` — knobs for age (slider spanning the thresholds), placement,
viewport shape.

## Re-render cadence — why host-ticked, not a guest timer

Three options were weighed for refreshing "N ago":

1. **Guest `request_frame_after`** — a full WASM rerun every second, self-waking an otherwise-static widget. Rejected.
2. **Piggyback on poll-retry renders** — correct and near-free (weather calls `request_frame()` on every reply,
   `weather/src/lib.rs:192`), but ticks only at the poll retry cadence.
3. **Declarative host-ticked `RelativeTimeLive`** (chosen) — rides the cached-tree animation-wake path, no WASM reruns,
   Visible-gated so off-screen cost is zero, and cheaper per tick than a WASM rerun. For the real widgets it is also
   lower-cadence in practice: weather goes stale only at `1.5 × 300 s = 450 s`, already the minutes band (60 s ticks);
   mining-clock already re-renders per second for its clock.

## Measured self-tick perf (on-device)

Measured on a real Deck (ARMv7, debug profile) with per-frame profiling — `RUST_LOG=info,bmc_wasm_host=debug` on the
compositor, reading `delta_ms` / `total_us` from the wasm-host render-frame log while the image widget was driven stale
(network offline-sealed via `ip route del default`).

- **Pre-fix** (a single `Short` format spelling `7m 30s`) — `next_change_delay_ms` aligned to the *seconds* boundary, so
  the host repainted the full 1280×480 frame **every 1 s** (~85 ms each) → **~8.5% core, continuous**, with no deep
  idle.
- **Post-fix** (`{ Short, Single }`, spelling `7m`) — cadence derives from `segments`, so `Single` aligns to the
  *minute* boundary. Over 170 s stale, 34 frames: `delta_ms` clustered at ~10 s (offline poll-retry), ~20 s (the image
  widget's own image-cycling), and ~50–60 s (minute-boundary tick) — **no sustained 1 s run**. ≈ **1.9% core**, and
  nearly all of that is the image widget's own repainting, not the pill.

Net: the stale-overlay self-tick drops from ~8.5% to a pill contribution near zero — **1 s → 60 s** once past the
seconds band. A residual ~0.8 s double-render appears ~once per 80 s (a settle/immediate frame paired with a retry, not
the old sustained repaint); negligible. The underlying full-frame repaint cost — any widget re-render paints the whole
frame on the 2-slot swapchain — is tracked separately as a dirty-region optimization, out of scope here.

## Migration

One commit per widget; **the last migration to stop using a shared banner deletes it.**

- weather — replace `render::with_stale_banner` (local `stale_banner()`), fed by the poll handle.
- mining-clock, mining-info — replace the `lib/mining/overlay` **stale** case (the auth-error case stays until its own
  commit).
- image — replace the local `render::with_stale_banner` (`Badge::Stale`) on this branch.
- mining auth-error — migrated to `Tag(Error)` in a tail-end commit that removes the final `lib/mining/overlay` remnant.

## Commit plan

1. `RelativeTimeLive` node + unit tests + Storybook.
2. `Tag` component + wasm glue + unit tests + Storybook.
3. Poll `is_stale()` API + staleness overlay + tests + overlay Storybook.
4. Per-widget migration, one commit each: weather, mining-clock, mining-info, image (last user of a shared banner
   deletes it).
5. Tail-end: mining auth-error → `Tag(Error)`, delete the remaining hand-rolled banner.

## Verification

- `just validate` (fmt + clippy + tests). Unit tests: `RelTimeFormat` output + direction sign-flip; `is_stale` /
  `last_success_age` with a fake clock; wire round-trips for the new nodes.
- Storybook (run locally, review by hand): `RelativeTimeLive` (direction, bands), `Tag` (variants × sizes × icon
  on/off), staleness overlay (age × placement × shape).
- Capture / visual regression: a stale-state fixture per migrated widget — \`200 → advance
  > stale_factor × interval → 503 →
  > capture`— driving the overlay on. Deterministic via the replay   clock; bless with`just update-baselines <widget>\`.
- **Self-tick perf + correctness (required gate, not just green unit tests):** measured on a real Deck — see **Measured
  self-tick perf (on-device)** above: the minute-boundary cadence holds and the pill's continuous 1 s repaint is gone
  (~8.5% → ~1.9% core). Off-screen/neighbor no-wake and no-drift updates rest on the Visible-gated animation path (§1)
  and the injected replay clock; not separately re-measured in that run.

## Deferred / reserved

- `RelTimeFormat::Custom(String)` — a duration/relative-time mini-language (d3-format and Python's format spec are
  *number* grammars and don't apply); the wire reserves the tag, implementation deferred until there's a real need.
- Live paragraph span (true inline flow mixing static + live text) — deferred; `row` composition covers the single-line
  pill.
