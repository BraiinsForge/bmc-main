# SDK units model — why dimension-named quantities

Written while promoting the ISS widget (BDK-304). We needed a localised length (altitude shown as km or miles per the
operator's unit system). The host had no length formatter and the SDK had no length type, so we added `Length` and
`Speed` to `bmc-wasm-runtime/sdk/src/units.rs`. This note records the model we chose and why it differs from the
existing `widgets-wasm/lib/units` crate, so the choice is explainable rather than a silent divergence.

## The model we adopted

- **One newtype per physical *dimension*** — `Length`, `Speed` (and later `Temperature`, `Angle`, …) — each storing
  **one canonical SI value** internally (metres, m/s).
- **Units are I/O only**: `from_kilometers` / `from_miles` constructors and `as_kilometers` / `as_miles` accessors. The
  unit never appears in the type.
- **`.format(decimals)`** renders per the operator's `unit_system` / `number_format` by delegating to the host
  formatters — one localisation path, shared with `format_number!` / `format_speed!` / the new `format_distance!`.

## Grievances with the existing `lib/units` types

These were added during my vacation and I had not reviewed them; I would raise the same points in review. The crate
names types after **units**, not **dimensions**:

- **`DegreeCelsius`** — that's a unit, not a quantity. It should be `Temperature`, with `from_celsius` /
  `from_fahrenheit` and `as_celsius` / `as_fahrenheit`. As written, the *type* hard-codes one unit, so accepting a
  Fahrenheit input or converting means a different type or ad-hoc arithmetic at the call site.
- **`KilometerPerHour`** — should be `Speed`. The unit leaks into every signature; a function that takes a speed should
  not care whether it arrived as km/h, m/s, or mph.
- **`Degree`** — degree of *what*? Ambiguous. Should be `Angle` (`from_degrees` / `from_radians`).
- **`Availability`** — ambiguous name; it appears to be an `Option`-flavoured wrapper for maybe-missing sensor values.
  Unless it encodes a state that `Option<T>` cannot, it should just be `Option<Quantity>`.

## Why dimension-named is better

- **No combinatorial type explosion.** Unit-named modelling needs a type per (quantity × unit): `KilometerPerHour`,
  `MilePerHour`, `MeterPerSecond`, … Dimension-named collapses all of those into one `Speed`.
- **The unit stops leaking into APIs.** Signatures read `Speed` / `Length`; which unit a value came in as is a local
  concern at construction, and which unit it's shown in is a local concern at formatting.
- **Adding a unit is additive** — a new `from_`/`as_` method, not a new type plus conversions threaded through every
  consumer.
- **Single localisation path.** Display follows operator settings uniformly because every dimension formats through the
  same host formatters, instead of each unit-type re-implementing its own rendering.

## Status / migration

`Length` + `Speed` are the slice the ISS widget needed. `lib/units` still backs `weather`, `braiins-pool`,
`bitcoin-mining-data` and `fleet-management`; those migrate onto the SDK types as a separate step, after which the
`units` crate and its unit-named types retire. Until then the SDK module is kept private (types re-exported) so the
`units` *name* doesn't collide with the `lib/units` crate in widgets that use both.
