# WASM Widget Best Practices

Read this before writing or changing a WASM widget. It collects the conventions and the easy-to-miss host behaviors
every widget under `widgets-wasm/` (and the SDK examples) should follow. The other pages in [WASM Widgets](README.md) go
deeper on individual topics.

## Split pure logic from the wasm boundary

Keep formatting, layout decisions, data models, and payload parsing in plain functions that take values and return
values. Gate behind `#[cfg(target_arch = "wasm32")]` only the code that calls what the SDK offers on `wasm32` alone.

A plain function is what a `#[wasm_test]` covers cheaply: the test hands it values, with no widget state or host traffic
to set up. When a host-bound helper has to feed such logic, split it so the pure part — sign, currency symbol, unit
assembly, "unavailable" handling — stays a plain function and only the host's result crosses the boundary. See
[Unit Testing](unit-testing.md) for writing and running the tests.

## Format numbers with `format_number!`

Do not hand-roll digit grouping or decimal separators. Use the SDK `format_number!` macro so the device `number_format`
system setting (grouping symbol and decimal mark) is applied; otherwise the same value prints identically in every
locale and looks unlocalized.

```rust
use bmc_wasm_sdk::format_number;

let price = format_number!(104_250.0, 0); // "104,250" / "104 250" / "104.250"
let rate = format_number!(93.42, 2); //      "93.42"   / "93,42"
```

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
  The label grows and pushes the value to the trailing edge. On a text, `flex` only grows it — unlike a container's, it
  keeps the text's own width as its basis — so it does not decide which side gives way when the row runs out. See
  [Let the renderer cut server strings](#let-the-renderer-cut-server-strings).
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

## Let the renderer cut server strings

Never budget a string by counting its characters: in a proportional font the glyph mix, not the count, decides what
fits. Set `text_overflow` on the text instead, and the renderer cuts it at its box.

- `TextOverflow::Ellipsis` keeps the text on one line and ends it in a shaped `…` where the box runs out. Prefer it for
  right-aligned text: once cut, the line fits its box, so it stays right-aligned.
- `TextOverflow::Clip` keeps the text on one line and cuts it at the box's inline edge. An overflowing line starts at
  the box's left edge whatever its `align`, as in CSS, so right-aligned text loses its tail.
- Neither mode soft-wraps. Hard line breaks still break, and `max_width` still caps the box.
- The `…` takes the first span's style.
- Canvas text (`Draw::text`, `Draw::autofit_text`) ignores `text_overflow`.

Both modes shrink in a flex row — to nothing under `Clip`, to the `…` under `Ellipsis` — and so do the containers around
them. Whether a text gets a box narrower than itself depends on where it sits:

- A direct child of a column is bounded by the column's width.
- An auto-width row whose parent does not stretch it (a `cross_align` other than `Stretch`, or a `center`) sizes to its
  content, so nothing in it is ever cut.
- A row that overflows shrinks every child that can shrink, in proportion to its width. Multi-word `Wrap` text wraps to
  its widest word, and an empty fixed-size box — an image placeholder — shrinks to nothing. Only a canvas with an
  explicit size, a single word, and a box around a fixed-size child keep their width.

Three tools decide who gives way:

- **Yielding run**: put the text that should give way in `row(props!(flex: 1.0), [...])`. A container with `flex` starts
  from zero and grows into what its siblings leave, so only the run is cut.
- **Cap**: `max_width` in the text's style bounds it even where its parent offers more.
- **Fixed text**: pass a label that must keep its width through `typography::unbroken`. It can no longer wrap, so the
  row takes its overrun out of the text beside it.

```rust
row(props!(cross_align: CrossAlign::Center, justify_content: Justify::SpaceBetween), [
    text(typography::unbroken(label), style!(color: TITLE)),
    text(value, style!(weight: FontWeight::BOLD, align: TextAlign::Right, text_overflow: TextOverflow::Ellipsis)),
])
```

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

Use the SDK `fmt!` macro, not `std`'s `format!`/`write!`. The `no-fmt-in-wasm` ast-grep rule rejects the allocating
`std` formatting macros in widget code.

## Match the source design

When porting an existing screen, take field sets, labels, units, and spacing structure from the reference design — for
the BMM screens that is the BOSer Slint source, not a browser mockup. Consistency with the shipped product beats local
taste.

## Verify before committing

Run `just validate`. After a change to what a widget draws, also run `just wasm::verify <widget>`; see
[Regression Testing](regression-testing.md).
