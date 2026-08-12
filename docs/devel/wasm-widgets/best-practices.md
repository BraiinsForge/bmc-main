# WASM Widget Best Practices

Read this before writing or changing a WASM widget. It collects the conventions and the easy-to-miss host behaviors
every widget under `widgets-wasm/` (and the SDK examples) should follow. Pair it with [Params](params.md),
[System Settings](system-settings.md), [Display Geometry](display-geometry.md), and
[Regression Testing](regression-testing.md), which go deeper on individual topics.

## Split pure logic from the wasm boundary

Keep formatting, layout decisions, data models, and payload parsing in ordinary modules that compile and unit-test on
the host. Gate only the code that touches host imports — `render`, fetch handling, and anything using `render_ui`,
`widget_viewport`, `fetch`, `log_warn!`, or `format_number!` — behind `#[cfg(target_arch = "wasm32")]`.

This keeps the bulk of the widget testable with `cargo test` while the host-only surface stays thin. When a host helper
has to feed pure logic (number formatting is the common case), split the function so the pure part — sign, currency
symbol, unit assembly, "unavailable" handling — stays host-tested and only the magnitude crosses the boundary.

## Format numbers through the host

Do not hand-roll digit grouping or decimal separators. Use the SDK `format_number!` macro so the device `number_format`
system setting (grouping symbol and decimal mark) is applied; otherwise the same value prints identically in every
locale and looks unlocalized.

```rust
use bmc_wasm_sdk::format_number;

let price = format_number!(104_250.0, 0); // "104,250" / "104 250" / "104.250"
let rate = format_number!(93.42, 2); //      "93.42"   / "93,42"
```

`format_number!` is a host call and only runs on `wasm32`. Give native/test builds a plain fallback for the magnitude
and compose sign, symbol, and unit around it in pure code.

## Read system settings and react to changes

Read `system::current()` on the render path. Because values are formatted from raw state on each frame, a setting change
shows on the next frame. Export `on_system_update` and call `request_frame()` when a change should appear immediately
instead of waiting for the next data tick. See [System Settings](system-settings.md).

## Align both axes CSS-style

The layout engine mirrors CSS flexbox on both axes: `cross_align` (`CrossAlign`) across the container, and
`justify_content` (`Justify::Start`/`Center`/`End`/`SpaceBetween`) along it — e.g.
`props!(justify_content: Justify::Center)` vertically centers a column's content.

`flex` and `spacer(...)` still compose well where a single stretching cell reads better than container-level
distribution:

- Right-align a value in a label/value row: give the label `flex: 1.0` and set the value's `align: TextAlign::Right`.
  The label grows and pushes the value to the trailing edge.
- Push one row of many to the bottom: a single `spacer(1.0)` above it, where `SpaceBetween` would spread all rows.

```rust
row(props!(cross_align: CrossAlign::Center), [
    text(label, style!(color: TITLE, flex: 1.0)),
    text(value, style!(weight: FontWeight::BOLD, align: TextAlign::Right)),
])
```

## Keep typography stable

Do not scale font sizes by viewport width. Pick layout bands — column count, spacing, which fields are visible — from
the actual width and height, but hold font sizes fixed. When space runs out, hide secondary fields rather than shrinking
or overlapping text.

## Model missing data explicitly

Represent each field as available-or-not (an `Availability`/`Option`-style type), never a sentinel like `0` or an empty
string. Render a clear placeholder (`N/A`) for any value that is unavailable, whether it is not yet loaded or its source
failed. Keep independent data sources independent: one source failing must not blank fields owned by another.

## Do not panic on expected failures

Network, API, and auth failures are normal operating conditions. Log a warning and keep the last good data; never panic.
Reserve `expect("BUG: ...")` for genuine internal invariants.

## Keep stack use bounded

Use `Vec<T>`, `String`, or `Box<[T]>` for large and runtime-sized values. Their small descriptors live on the stack,
while their payloads live on the guest heap. Avoid large local arrays and large structs composed of inline arrays;
`Box::new([value; N])` can still construct the array on the stack before moving it, so build large buffers with `Vec`
and convert to a boxed slice when fixed ownership is useful.

Do not recurse to a depth controlled by fetched data, params, collection length, or UI input. Use an explicit
heap-backed worklist for tree and graph traversal. Keep UI trees shallow as well as bounded: sibling nodes collected in
a `Vec<Node>` do not add recursive depth, but nested containers deepen the SDK's recursive tree serialization.

See [Memory](memory.md) for the 64 KiB stack budget, measured high-water marks, and scaling implications.

## Build strings with the SDK macro

Use the SDK `fmt!` macro, not `std`'s `format!`/`write!`. The `no-fmt-in-wasm` CI gate rejects the allocating `std`
formatting macros in widget code.

## Match the source design

When porting an existing screen, take field sets, labels, units, and spacing structure from the reference design — for
the BMM screens that is the BOSer Slint source, not a browser mockup. Consistency with the shipped product beats local
taste.

## Verify before committing

Run these from the repository root unless noted:

- `nix develop -c cargo test -p <widget>` (from `widgets-wasm/`) — host unit tests.
- `nix develop -c cargo clippy -p <widget> --target wasm32-unknown-unknown -- -D warnings` — the lint gate that actually
  compiles the gated render code.
- `just validate` — among its gates, rejects allocating `fmt` macros in widget code.
- `just wasm::verify <widget>` — the visual-regression gate. Headless rendering and baseline capture need a GPU
  (`/dev/dri`); without one this only runs in CI.
