# Mining widget — round display support (BDK-501)

## Goal

Add round-display support to the `widgets-wasm/mining-info` widget for the BFM100 device (480×480 round). The widget receives its viewport shape from the host; today it ignores `.shape` and always renders the rectangular layout. We add round variants for three of the four screens — Mining, Geek, Info Overload — and defer Network (it keeps falling back to the rectangular renderer for now).

The Mining and Geek screens share a single circular-gauge design from the designer mockup; only their field assignments differ. Info Overload keeps its existing content but reshuffles it into a circular three-band layout with the BTC price bar moved to the center.

## Architecture

### Shape dispatch (option A)

`render()` in `lib.rs` already calls `widget_viewport()` but ignores `.shape`. It will branch on shape first, then on `View`:

- `ViewportShape::Round` → new `round` module: `round::mining()`, `round::geek()`, `round::info_overload()`. `View::Network` falls back to the existing rectangular `network()`.
- `ViewportShape::Rectangular` → existing `render::*` renderers, unchanged.

A dedicated `round` render module keeps the circular-gauge layout fully separate from the rectangular flex layout rather than scattering shape checks through the existing renderers. The `round` module owns its shared `gauge` helper.

### Data model additions

Two new fields on `MinerData` (`model.rs`), both `Availability<T>`:

- `sticker_hashrate_ths: Availability<f64>` — full-scale reference for the gauge fill-fraction.
- `efficiency_j_th: Availability<f64>` — shown on Mining and Geek.

### API parsing

No new endpoints and no new poll gating — both source endpoints are already polled.

- `parse_details` (`/miner/details`) reads `sticker_hashrate.gigahash_per_second`, converted via `ths_from_ghs`.
- `parse_stats` (`/miner/stats`) reads `power_stats.efficiency.joule_per_terahash` directly (no client-side division), unit J/TH.

### The gauge component

Shared by Mining and Geek, owned by the `round` module. It takes a center, radius, a fill-fraction, and a `GaugeState` (Overclocked / Good / Underclocked / Off) that selects the accent colors. It draws two `Draw::arc` sweeps using the renderer's native segmented-arc primitive:

1. **Base sweep** — full circle (`0..TAU`), `ArcSegments::uniform(...)` for all 28 ticks, `ArcFill::Solid` gray (unlit).
2. **Lit sweep** — the same segment geometry over the filled angular range, `ArcFill::gradient(from, to)` in the state accent, drawn on top.

The renderer's `remap_arc_segments` normalizes segment spans within each sweep's `start..end`, so the lit segments align with the base segments. Transitions on data change come for free via `ArcOverride` if we choose to enable them.

Fill-fraction = `hashrate_ths / sticker_hashrate_ths`, clamped to `0..1`. When `sticker_hashrate_ths` is `Unavailable` or zero, the lit sweep is skipped (neutral ring) — this avoids a divide-by-zero and is exactly what the OFF state needs.

### Manifest

Add a round viewport entry to `supported_viewports`: `{ type: round, width: 480, height: 480 }`. No dpi bounds, per house style.

## Screens

### Mining & Geek (shared gauge + four-quadrant clusters)

Both share the gauge and quadrant-cluster layout from the mockup; only the field assignments differ. Design tokens come from `docs/design/mining-stats-widget`:

- 480×480 canvas, centered 28-segment ring (Ø460, ~10px inset).
- Center: Hashrate value, 64px/700, with unit below.
- Four stat clusters in the quadrants, separated by faint quadrant dividers (#ffffff @10%). Each cluster = label (16px/400) + value (32px/600).
- `GaugeState` thresholds default to the boundaries the design docs describe (Overclocked / Good / Underclocked / Off); pinned exactly during implementation.

**Mining field map:**

- Top-left: Power Cons. (W)
- Top-right: Efficiency (J/TH)
- Center: Hashrate (TH/s)
- Bottom-left: Temperature (board–chip range)
- Bottom-right: Fan Speed (%)

**Geek field map** (swaps Fan → BTC Price):

- Top-left: Power Cons. (W)
- Top-right: Efficiency (J/TH)
- Center: Hashrate (TH/s)
- Bottom-left: Temperature (board–chip range)
- Bottom-right: BTC Price (money, current currency)

Unavailable values render `N/A` (device) or `--` (public), consistent with the rectangular renderers.

### Info Overload (round three-band layout, option A)

Three horizontal bands on the circle, with a curated field subset that respects the circular safe-area:

- **Middle band** — the relocated top bar: BTC price + 24h change (signed %, colored) + the sparkline chart (`price_chart`). It spans the widest part of the circle, so it gets the most horizontal room.
- **Upper band** — a small curated set of stats above the middle (narrower; circle curves in), ~2–3 fields.
- **Lower band** — a small curated set below, same width constraint.

The exact field subset (from the rectangular set: network hashrate, diff adjust, epoch progress, avg fee, block height, hashprice, hashvalue, plus miner stats) is chosen when planning this screen — the highest-value fields that fit the upper/lower bands legibly, rather than cramming all of them. The existing `price_chart` sparkline and the public/miner formatting helpers are reused; only the arrangement is new.

### Network

Deferred. Round dispatch falls back to the existing rectangular `network()` renderer.

## Testing

- `miner_api.rs` — extend the `MapJson`-based unit tests to cover `sticker_hashrate` (in `parse_details`) and `efficiency.joule_per_terahash` (in `parse_stats`), including the absent and zero cases.
- Gauge math — unit test for fill-fraction clamping and `GaugeState` selection across the four states, including `sticker_hashrate` Unavailable → neutral ring.
- Verification — `nix develop -c cargo test -p mining-info`, `cargo clippy -p mining-info --target wasm32-unknown-unknown -- -D warnings`, `just validate-wasm-no-fmt`, and `just wasm::verify mining-info` (GPU-gated) for visual confirmation on the round viewport.

## Out of scope

- Network round screen.
- Changing the rectangular renderers.
- New endpoints or poll gating.
