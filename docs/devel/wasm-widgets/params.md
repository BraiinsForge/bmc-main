# WASM Widget Params

Params are per-widget-instance values chosen by the operator. They belong in the widget's `manifest.json`, are edited in
the scene/widget UI, and are delivered to the running widget as complete snapshots.

Use params for widget-specific display choices, API endpoints, public account identifiers, locations, labels,
thresholds, and toggles. Do not use params for deck-wide values such as the device timezone or night mode; read those
from [system settings](system-settings.md). Never use params for secrets; declare a [credential slot](credentials.md)
instead.

## Never Put a Secret in a Param

A param is not a private channel, in three separate ways:

- its value is stored in the device configuration file, which the support archive collects — the archive censors only
  the one historical `api_key` shape, so a token under any other key is bundled verbatim and mailed to support;
- its value is delivered into the widget's own memory, so it is readable by whoever wrote the widget;
- nothing constrains where the widget then sends it.

Declare a [credential slot](credentials.md) instead. The operator binds a saved account to the slot, and the device
substitutes the secret into outbound requests at the moment they leave, so the widget never holds it and a
service-pinned credential cannot be sent anywhere else.

## Declare Params

Declare params in the manifest's `params` object. The key becomes the stable wire key and, after codegen, the Rust field
name.

```json
{
  "params": {
    "city": {
      "name": "City",
      "type": "string",
      "default_value": "Prague"
    },
    "show_seconds": {
      "name": "Show seconds",
      "type": "boolean",
      "default_value": true
    },
    "accent_tz": {
      "name": "Accent timezone",
      "type": "timezone",
      "optional": true
    }
  }
}
```

Supported param kinds are `string`, `integer`, `double`, `boolean`, `timezone`, and `array`. Strings, integers, and
doubles may also declare `enum_values`; generated code turns those enum values into Rust enum wrappers. Their
`enum_control` picks how the operator UI offers them: `dropdown`, the default, or `radio`, a radio group that suits a
few short options. Numeric params may declare `min`, `max`, `step`, and a `unit` such as "s" or "%", which the operator
UI shows with the field's name. String params may declare UI `format` hints, and `min_length` / `max_length` counted in
characters (Unicode code points, as JSON Schema counts them) up to 1024. Strings, integers, doubles, and timezones may
declare a `placeholder`, as params, list items, or object fields alike: example text the operator UI shows in the empty
input, such as "e.g. BTC or AAPL". It is never a value, so the widget never receives it.

Required params must declare `default_value`, except an `array`, whose omitted default is the empty list. Optional
params may omit a default; when unset, the generated Rust field is `Option<T>` and evaluates to `None`.

Use `widgets-wasm-examples/params-demo/manifest.json` as the reference example. It exercises every `ParamKind`, enum
values, ranges, string formats, optional-without-default params, and a list of every item kind.

### Lists

An `array` param is an ordered list the operator can add to, remove from, and reorder. Its `items` says what every item
is: any scalar kind, with the same options a scalar param takes, or an `object` whose `fields` are scalars.

```json
{
  "params": {
    "symbols": {
      "name": "Ticker symbols",
      "type": "array",
      "items": { "type": "string", "placeholder": "e.g. BTC or AAPL" },
      "min_items": 1,
      "max_items": 8,
      "unique_items": true,
      "default_value": ["NVDA", "AAPL"]
    },
    "links": {
      "name": "Links",
      "type": "array",
      "max_items": 4,
      "unique_items": ["label"],
      "items": {
        "type": "object",
        "fields": {
          "label": { "name": "Label", "type": "string" },
          "url": { "name": "URL", "type": "string", "format": "uri", "optional": true }
        }
      }
    }
  }
}
```

`max_items` is required and at most 100; `min_items` defaults to 0. The list's `default_value` seeds a new widget;
omitting it starts the list empty, so a list with `min_items` above 0 needs a `default_value` that fills it. An item's
own `default_value`, or each field's for an object, seeds the items the operator adds. In a list's `default_value`, an
omitted optional field is stored unset (`null`); a field's own `default_value` only seeds the rows the operator adds.

`unique_items: true` refuses an item equal to an earlier one, the list's `default_value` included; an object row repeats
only when every field matches. A list of field keys such as `["label"]` compares object rows on those fields alone.
Either way, an unset optional field matches an unset one. Every item the operator adds starts at the same defaults, so
under `unique_items` an item declares no `default_value`, and neither does a field `unique_items` names.

The web UI reports a single repeated key on its field, and any other repeat once for the whole row.

A list is never `null` and neither is any item in it, so array params cannot be `optional`; declare `min_items: 0` for a
list that may be empty. An `optional` object field is what can be unset, and it is `null` within its row.

## Generate Typed Accessors

Widget params are generated from `manifest.json` into `src/manifest_params.rs`. Do not edit that file by hand.

From the repository root:

```bash
just wasm::gen <widget-name>
```

For example:

```bash
just wasm::gen params-demo
```

A change to the generator itself drifts every widget at once; `just wasm::gen-all` regenerates the lot rather than
naming each one.

The generated file contains:

- A `Params` struct with one Rust field per manifest key.
- `Params::current()` for the latest typed snapshot.
- `Params::previous()` for the snapshot before the latest update, or `None` before the first runtime update.
- `Params::changed_keys(&previous)` for update-hook diffing.
- Enum wrapper types for manifest `enum_values`.
- For an `array` param `links`, a `LinksItem` type when its items need one: an enum for items with `enum_values`, or a
  struct with one field per object field. An enum-valued object field gets its own enum, such as `LinksItemTone`.

See `widgets-wasm-examples/params-demo/src/manifest_params.rs` for the generated shape.

## Read Params In A Widget

Import the generated module and read `Params::current()` from `init`, `render`, or `on_params_update`.

```rust
mod manifest_params;

use bmc_wasm_sdk::*;
use manifest_params::Params;

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let params = Params::current();
    let viewport = widget_viewport();

    let label = if params.show_seconds {
        "seconds enabled"
    } else {
        "seconds disabled"
    };

    render_ui(viewport.width, viewport.height, text(label, style!(size: 24)));
    request_frame_after(1_000);
}
```

Required manifest params become plain fields such as `bool`, `i32`, `f64`, or `String`. Optional params become
`Option<T>`:

```rust
let subtitle = params
    .accent_tz
    .as_deref()
    .unwrap_or("device timezone");
```

A list param is a `Vec` of its item type, in the order the operator arranged it. An object item is a generated struct,
with `Option<T>` for its optional fields:

```rust
let ticker = params.symbols.join(" · ");
for link in &params.links {
    let target = link.url.as_deref().unwrap_or(&link.label);
}
```

If a widget is small or intentionally dynamic, it can read the raw SDK snapshot through the generic typed reader:

```rust
use bmc_wasm_sdk::params::typed::ParamRead;

let params = bmc_wasm_sdk::params::current();
let city = <String as ParamRead>::read_required(&params, "city");
let show_seconds = <bool as ParamRead>::read_required(&params, "show_seconds");
let accent_tz = <String as ParamRead>::read_optional(&params, "accent_tz");
let symbols = <Vec<String> as ParamRead>::read_required(&params, "symbols");
```

Use `read_required` for manifest-required params. It traps if the host snapshot is missing the value: saving a scene
stores every declared key, defaults included, but params stored before an upgrade that added a key lack it until the
widget migrates them (BDK-723). Use `read_optional` for manifest-optional params; it returns `None` for missing or
`null` values. Either read traps on a value or list item of another type, or an enum value outside its options, since
the host validates every value against the manifest.

`ParamRead` reads any type that implements `ValueRead`, and a `Vec` of one. The scalar types and the generated enums and
row structs all implement it; a hand-written row type can implement it with the `required_field` and `optional_field`
helpers.

Prefer generated params for normal widgets. They keep the widget code aligned with the manifest and remove repeated
stringly-typed lookups.

## React To Updates

The host delivers params as full snapshots, not patches. `on_params_update` fires only after the initial values have
already been delivered; it does not fire for the initial `init` snapshot.

Delivery updates the snapshot and invokes the optional hook, but does not itself schedule a render. Call
`request_frame()` or `request_frame_after()` when the change should repaint; otherwise the new snapshot is observed on
the next naturally scheduled render.

Use the hook when a params change needs to update cached state, start debounced work, or repaint changed UI.

```rust
#[unsafe(no_mangle)]
pub extern "C" fn on_params_update() {
    let Some(previous) = Params::previous() else {
        return;
    };

    let changed = Params::current().changed_keys(&previous);
    if changed.contains(&"city") {
        // Repaint the visible city output.
        request_frame();
    }
}
```

Omitting the hook is correct only when the widget already guarantees another render cadence and accepts the resulting
latency. A static widget whose visible output depends on params must export `on_params_update` and request a frame.
Mining-clock, for example, exports the hook to refresh authentication state but deliberately does not request a frame:
its existing one-second cadence displays the new values on the next tick without shifting the second hand's even steps.

The operator UI can send live-preview updates before saving. Treat preview and committed updates the same way. Debounce
network fetches and other expensive side effects triggered by params changes.

## Use The Testbed

The WASM testbed reads the widget manifest and shows a Params panel in the right sidebar when the manifest declares
params. Each control maps to the manifest type: text fields for strings, numeric inputs for numbers, dropdowns or radio
groups for `enum_values` as `enum_control` asks, checkboxes for booleans, and clear-to-null controls for optional
params. Each list param gets a full-width block below them: a row per item with move-up, move-down, and remove buttons,
then an add button, with `min_items` and `max_items` disabling remove and add at the bounds. An enum inside an item is
always a dropdown, keeping each row one line tall.

Like the device, the testbed delivers only params the shared validator accepts. While an edit fails it, the widget keeps
its last valid params and the panel lists each violation by its path, such as `["zones"][0]: Must be a valid timezone`.

Run the params demo from `bmc-wasm-runtime/`:

```bash
just dev params-demo
```

Or run another widget:

```bash
just dev <widget-name>
```

From the repository root, use:

```bash
just wasm::dev <widget-name>
```

On branches with production WASM widgets under `widgets-wasm/` (the BDK-293 layout), the same recipe accepts production
widget package names as well as SDK examples:

```bash
just wasm::dev <production-widget-name>
```

If the local branch does not have that multi-root recipe yet, build the production widget and run the testbed directly
with the `.wasm` file plus its manifest. Cargo writes hyphenated package names with underscores in the wasm filename:

```bash
cargo build --manifest-path widgets-wasm/Cargo.toml \
  -p <widget-name> --target wasm32-unknown-unknown

cargo run --features testbed --bin testbed \
  --manifest-path bmc-wasm-runtime/Cargo.toml -- \
  widgets-wasm/target/wasm32-unknown-unknown/debug/<widget_binary>.wasm \
  --manifest=widgets-wasm/<widget-name>/manifest.json
```

Changing a value in the Params panel calls `deliver_params_update` for every previewed size and immediately invokes an
exported `on_params_update` hook. Visible changes repaint immediately only when the hook requests a frame. The
`params-demo` widget highlights changed rows, which makes it the best example to inspect when wiring update behavior.
