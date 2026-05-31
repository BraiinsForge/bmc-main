# Mining widget — round display support (BDK-501)

## Goal

Add round-display support to the `widgets-wasm/mining-info` widget for the BFM100 device (480×480 round). The widget
receives its viewport shape from the host; today it ignores `.shape` and always renders the rectangular layout. We add
round variants for three of the four screens — Mining, Geek, Info Overload — and defer Network (it keeps falling back to
the rectangular renderer for now).

The Mining and Geek screens share a single circular-gauge design from the designer mockup; only their field assignments
differ. Info Overload keeps its existing content but reshuffles it into a circular three-band layout with the BTC price
bar moved to the center.

## Architecture

### Shape dispatch (option A)

`render()` in `lib.rs` already calls `widget_viewport()` but ignores `.shape`. It will branch on shape first, then on
`View`:

- `ViewportShape::Round` → new `round` module: `round::mining()`, `round::geek()`, `round::info_overload()`.
  `View::Network` falls back to the existing rectangular `network()`.
- `ViewportShape::Rectangular` → existing `render::*` renderers, unchanged.

A dedicated `round` render module keeps the circular-gauge layout fully separate from the rectangular flex layout rather
than scattering shape checks through the existing renderers. The `round` module owns its shared `gauge` helper.

### Data model additions

One field on `MinerData` (`model.rs`):

- `efficiency_j_th: Availability<f64>` — the Geek top-right readout (J/TH). Mining does not use it.

There is no `sticker_hashrate_ths` field. The gauge is driven by the existing `MinerData.mcr_percent`, computed in
`parse_hashboards` (`miner_api.rs:46`) as `real_hashrate / nominal_hashrate * 100` from `/miner/hw/hashboards`. MCR is
the gauge driver on both screens, and is *also* the Mining top-right readout (Geek shows efficiency there instead).

### API parsing

One new parse, no new endpoints, and no new poll gating. `parse_stats` (`/miner/stats`) additionally reads
`power_stats.efficiency.joule_per_terahash` into `efficiency_j_th` (no client-side division; unit J/TH). `/miner/stats`
is gated `[Mining, Geek, InfoOverload]`, so Geek already polls it — efficiency needs no gating change.

Every other field each round screen renders is already produced by an endpoint whose `views` list includes that screen:

- `/miner/stats` (`[Mining, Geek, InfoOverload]`) → hashrate, power, efficiency.
- `/miner/hw/hashboards` (`[Mining, Geek]`) → temperature, `mcr_percent`.
- `/cooling/state` (`[Mining]`) → fan speed.
- `public_price` (`[Geek, Network, InfoOverload]`) → BTC price (Geek bottom-right, Info Overload middle).
- `public_block` / price-history / etc. (`[InfoOverload]`) → block height, sparkline (Info Overload).

This is why driving the gauge from `mcr_percent` rather than a sticker reference matters: `/miner/details` (the only
source of a sticker hashrate) is gated to `[Geek, InfoOverload]` and is **not** fetched on Mining (`lib.rs:65-69`), so a
sticker-based gauge would be permanently neutral on the primary screen. `mcr_percent` comes from `/miner/hw/hashboards`,
which *is* polled on Mining, so the gauge works on both Mining and Geek with zero gating changes.

### The gauge component

Shared by Mining and Geek, owned by the `round` module. It takes a center, radius, a lit-tick count, and a `GaugeState`
(Overclocked / Good / Underclocked / Off / UnknownScale) that selects the accent colors. `UnknownScale` is the "hashing
but `mcr_percent` unavailable" fallback (see Gauge state & fill below): no lit sweep, gray-only base ring, neutral label
— it is not OFF and is not representable by the four accent states. It draws two `Draw::arc` sweeps over the full circle
(`0..TAU`) using the renderer's native segmented-arc primitive.

**Tick geometry.** The ring is 28 ticks, evenly spaced clockwise starting at 12 o'clock. `Draw::arc` angles run
clockwise from 12 o'clock (`protocol/src/arc.rs:6-8`), so segment 01 sits at angle `0` and the fill grows clockwise —
matching the OFF design's single lit tick at the top (segment 01). The widget computes the 28 absolute tick spans once
(an even partition of `0..TAU` into 28 slots with a small inter-tick gap) and stores them; both sweeps draw from that
same span list so the lit ticks register exactly over the base ticks.

The gauge draws two `Draw::arc` sweeps, both spanning `0..TAU`:

1. **Base sweep** — `Draw::arc(.., 0.0, TAU, .., ArcFill::Solid(gray), ArcSegments::Explicit(all_28_spans))` — the full
   unlit ring.
2. **Lit sweep** — `Draw::arc(.., 0.0, TAU, .., ArcFill::gradient(from, to), ArcSegments::Explicit(lit_spans))`, drawn
   on top, where `lit_spans` is the first `lit_count` entries of the same `all_28_spans` list (the absolute spans, not a
   recomputed compressed set).

The lit sweep MUST reuse the base sweep's absolute spans, sliced to `lit_count`. It must NOT call
`ArcSegments::uniform(0, fill·TAU, 28, …)` or use a shorter sweep angle: in the normal render path the renderer does not
clip or remap explicit spans — `arc_spans` (renderer.rs:1159) clones `ArcSegments::Explicit` verbatim, and
`remap_arc_segments` runs only during animation interpolation, on host-private types the widget cannot reach
(`ArcOverride`/`remap_arc_segments`, draw.rs:32/68, invoked at draw.rs:556-563). Compressing 28 ticks into a shorter arc
would mis-register them against the base ring. Alignment is therefore the widget's responsibility, achieved by sharing
one span list. The gradient `t` is evaluated by the renderer across the sweep's `start..end` (here `0..TAU`), so each
lit tick's color tracks its absolute position — the designer's "dark at start → bright at leading edge". The gauge is
redrawn statically each frame from the current data; no transition/animation machinery is used.

`lit_count = round(28 * fill_fraction)`, `fill_fraction = clamp(mcr_percent / 130.0, 0.0, 1.0)`, so the ring reaches
28/28 at the overclock threshold (see below).

#### Gauge state & fill

`GaugeState` and the ring are driven by `mcr_percent` (the already-available real/nominal ratio as a percentage), with
OFF determined independently by actual hashing so missing data never masquerades as OFF:

- **OFF is determined solely by actual hashing.** `GaugeState::Off` iff `hashrate_ths` is `Unavailable` or `<= 0.0`. A
  stopped miner reports exactly `0.0` real hashrate, so no epsilon is used. It does not depend on `mcr_percent`. Renders
  the OFF design: red accent, a single lit red tick at segment 01, the rest unlit. A live miner is never shown as OFF.

- **When the miner is hashing but `mcr_percent` is `Unavailable`**, the scale is unknown: the center hashrate is still
  shown, the ring is drawn fully unlit (gray base only, no lit sweep), and the `Hashrate` status label uses a neutral
  color (`TITLE`/white) rather than a state accent. This is the "unknown scale" fallback, distinct from OFF.

- **When the miner is hashing and `mcr_percent` is available**, it selects the state (boundaries inclusive at the lower
  edge):

  - `mcr_percent >= 130.0` → **Overclocked** (purple `#8b7cff`, see `docs/design/mining-stats-widget/overclocked.md`),
    ring full (28/28).
  - `85.0 <= mcr_percent < 130.0` → **Good** (green `#34c06a`, `good.md`).
  - `0.0 < mcr_percent < 85.0` → **Underclocked** (amber `#feba53`, `underclocked.md`).
  - OFF accent and ring detail: `off.md`.

  Both boundaries are confirmed. `130%` is the product-chosen overclock edge (bos-main defines no overclock UI state).
  `85%` matches bos-main's chip under-performing threshold `underperforming_mcr: 0.85`
  (`bosminer-plus-tuner/src/miner.rs:1043-1044`), which uses the same `real/nominal` ratio as `mcr_percent`
  (`open/boser/.../mining_data.rs:122-129`); it is also consistent with the design sample states (Good `4.02` ≈ 87% MCR,
  Underclocked `3.02` ≈ 66% MCR against a ~`4.6` nominal).

### Manifest

Add a round viewport entry to `supported_viewports`. `WidgetViewportConstraint` has no `width`/`height` keys; an exact
size is pinned via `min_* == max_*` (`manifest.schema.json:424-490`), matching the existing rectangular entry's shape
(`manifest.json:15-23`):

```json
{ "type": "round", "min_width": 480, "max_width": 480, "min_height": 480, "max_height": 480 }
```

No dpi bounds, per house style.

## Screens

### Mining & Geek (shared gauge + four-quadrant clusters)

Both share the gauge and quadrant-cluster layout from the mockup; only the field assignments differ. Design tokens come
from `docs/design/mining-stats-widget`:

- 480×480 canvas, centered 28-segment ring (Ø460, ~10px inset).
- Center: Hashrate value, 64px/700, with unit below.
- Four stat clusters in the quadrants, separated by faint quadrant dividers (#ffffff @10%). Each cluster = label
  (16px/400) + value (32px/600).
- `GaugeState` thresholds and per-state accents are defined under "Gauge state & fill" above, citing the per-state
  design files.

**Mining field map:**

- Top-left: Power Cons. (W) — `power_w`
- Top-right: MCR (%) — `mcr_percent`
- Center: Hashrate (TH/s) — `hashrate_ths`
- Bottom-left: Temperature (board–chip range) — `temperature`
- Bottom-right: Fan Speed (%) — `fan_percent`

**Geek field map** (vs Mining: top-right MCR → Efficiency, bottom-right Fan → BTC Price):

- Top-left: Power Cons. (W) — `power_w`
- Top-right: Efficiency (J/TH) — `efficiency_j_th`
- Center: Hashrate (TH/s) — `hashrate_ths`
- Bottom-left: Temperature (board–chip range) — `temperature`
- Bottom-right: BTC Price (money, current currency) — `btc_price`

Unavailable values render `N/A` (device) or `--` (public), consistent with the rectangular renderers.

### Info Overload (round three-band layout, option A)

Three horizontal bands on the circle, each a three-cell horizontal row at the same stat-cluster typography as the
rectangular renderer (fonts are not scaled for the round viewport). Both the upper and lower bands sit within the wide
central portion of the circle — at a vertical offset of ~120px from center the chord is ~415px (`2·√(240²−120²)`), wider
than the rectangular three-field row — so three fields fit without shrinking. Six stats total, plus the middle bar:

- **Upper band** — three miner-output fields, left → right:
  1. **Hashrate** — `format::fixed(miner.hashrate_ths, 2)`, unit `TH/s`, label `Hashrate`.
  2. **Power Consump.** — `format::fixed(miner.power_w, 0)`, unit `W`, label `Power Consump.`.
  3. **Block Height** — `format::public_integer(block_height)`, no unit, label `Block Height`.
- **Middle band** — the relocated top bar, left → right: 24h change `Bitcoin (24h)`
  (`format::signed_percent(btc_change_24h_percent, 2)` + `%`, colored green/red/neutral as in the rectangular header),
  the `price_chart` sparkline, then the BTC price (`format::money(btc_price, 0)`). Spans the widest part of the circle.
- **Lower band** — three fields, left → right (the existing rectangular bottom row):
  1. **Miner Uptime** — `format::uptime(miner.uptime_s)`, no unit, label `Miner Uptime`.
  2. **Fees (144 Blocks)** — `~ {}` + `format::fixed(avg_fee_percent, 1)`, unit `%`, label `Fees (144 Blocks)`.
  3. **Hashvalue** — `format::fixed_strip_zero_fraction(hashvalue_sat_th_day, 2)`, unit `SAT/TH/Day`, label `Hashvalue`.

Unavailable values render `N/A` (device) / `--` (public) as elsewhere. The existing `price_chart` sparkline and the
`format::*` helpers are reused; only the arrangement is new. The remaining rectangular fields (Prev. Diff. Adjust., Est.
Diff. Adjust., Epoch Progress, Network Hashrate, Hashprice) are dropped on round.

### Network

Deferred. Manifest viewport support is widget-wide, so `View::Network` remains selectable on round hardware. Round
dispatch falls back to the existing rectangular `network()` renderer drawn into the 480×480 round viewport. Expected
degraded behavior: the rectangular layout renders centered on the round canvas and its corners fall outside the visible
circle, so edge content may be clipped by the bezel. This is an accepted interim state until a dedicated round Network
screen is designed; it is not a regression of the rectangular layout itself.

## Testing

- `miner_api.rs` — extend the `MapJson`-based `parse_stats` tests to cover `power_stats.efficiency.joule_per_terahash` →
  `efficiency_j_th`, including the absent case. `mcr_percent`, hashrate, power, temperature, fan, and the public fields
  are already covered by existing tests.
- Gauge math — unit tests for `GaugeState` selection and lit-tick count, driven by `mcr_percent` and `hashrate_ths`: OFF
  when `hashrate_ths` is `Unavailable`/`<=0` (regardless of `mcr_percent`), the three MCR bands
  (Underclocked/Good/Overclocked) at and around the `85%` and `130%` boundaries, `lit_count` clamping at
  `mcr_percent >= 130%` → 28 ticks, and the "unknown scale" fallback (hashing but `mcr_percent` Unavailable → no lit
  sweep, neutral status label). Also assert `lit_spans` is a prefix slice of the shared `all_28_spans` list (alignment
  guarantee), not a recomputed set.
- Verification — `nix develop -c cargo test -p mining-info`,
  `cargo clippy -p mining-info --target wasm32-unknown-unknown -- -D warnings`, `just validate-wasm-no-fmt`, and
  `just wasm::verify mining-info` (GPU-gated) for visual confirmation on the round viewport.

## Out of scope

- Network round screen.
- Changing the rectangular renderers.
- New endpoints or poll gating.
