# WASM Widget System Settings

System settings are host-defined deck-wide values. They are not declared per widget and they are not generated from
`manifest.json`. Every WASM widget reads the same fixed system snapshot through `bmc_wasm_sdk::system`.

Use system settings when the operator expects one global value to affect all widgets. Use [params](params.md) only when
each widget instance should be configurable independently.

## Available Settings

The SDK exposes these fields on `system::Snapshot`:

| Accessor              | Type                                | Meaning                                                |
| --------------------- | ----------------------------------- | ------------------------------------------------------ |
| `timezone()`          | `Option<&str>`                      | IANA timezone identifier, for example `Europe/Prague`. |
| `time_format()`       | `Option<system::TimeFormat>`        | `Hour12` or `Hour24`.                                  |
| `date_format()`       | `Option<system::DateFormat>`        | Operator-selected date display format.                 |
| `number_format()`     | `Option<system::NumberFormat>`      | Digit grouping and decimal separator format.           |
| `first_day_of_week()` | `Option<system::Weekday>`           | First day used by calendar-like widgets.               |
| `temperature_unit()`  | `Option<system::TemperatureUnit>`   | `Celsius` or `Fahrenheit`.                             |
| `unit_system()`       | `Option<system::UnitSystem>`        | `Metric` or `Imperial` for non-temperature units.      |
| `next_alarm()`        | `Option<system::NextAlarmView<'_>>` | Host-resolved next-to-fire alarm, if any.              |
| `night_mode()`        | `Option<bool>`                      | Resolved deck-wide night-mode state.                   |

Except for `next_alarm()`, these are expected to be present on a real device after the initial host delivery. They
return `Option` because the SDK decodes a host-provided byte snapshot defensively: a native test may have an empty
snapshot, a host bug could send malformed bytes, or a future protocol transition could omit a field temporarily. Treat
`None` as "the snapshot did not contain a usable value", not as "this setting is optional per widget".

`next_alarm()` returns the soonest scheduled alarm after host-side resolution. Its `fire_at_utc_ms` field is UTC
milliseconds since the Unix epoch; pair it with `timezone()` when rendering local time.

`night_mode()` is already resolved by the host from the operator settings and the wall clock. Widgets only see the
active/inactive boolean.

## Read Settings In A Widget

Read `system::current()` from `init`, `render`, or `on_system_update`.

```rust
use bmc_wasm_sdk::system;
use bmc_wasm_sdk::*;

#[unsafe(no_mangle)]
pub extern "C" fn render(delta_ms: u32) {
    let sys = system::current();
    let viewport = widget_viewport();

    let timezone = sys.timezone().unwrap_or("UTC");
    let mode = if sys.night_mode().unwrap_or(false) {
        "night"
    } else {
        "day"
    };

    render_ui(
        viewport.width,
        viewport.height,
        col(props!(padding: 16.0, gap: 8.0), [
            text(timezone, style!(size: 22)),
            text(mode, style!(size: 18)),
        ]),
    );
    request_frame_after(1_000);
}
```

Keep fallbacks local to the rendering decision rather than encoding sentinel values in widget state.

## React To Updates

`on_system_update` fires after the initial snapshot when the host delivers a new deck-wide snapshot. It is separate from
`on_params_update`; params changes do not rotate `system::previous()`, and system changes do not rotate
`params::previous()`.

Use the hook when the widget caches derived values, starts work in response to a setting, or wants to highlight changed
system fields.

```rust
use bmc_wasm_sdk::system;

#[unsafe(no_mangle)]
pub extern "C" fn on_system_update() {
    let current = system::current();
    let previous = system::previous();

    if current.timezone() != previous.timezone()
        || current.time_format() != previous.time_format()
    {
        // Recompute cached local-time labels.
    }

    if current.night_mode() != previous.night_mode() {
        // Rebind colors or request a new frame.
    }
}
```

The initial system snapshot is available before `init` runs. The initial delivery does not call `on_system_update`.

## Formatting Choices

The SDK gives widgets the selected enum values; each widget still decides how to apply them. A clock widget might use
`time_format()` to choose 12-hour or 24-hour text. A weather widget might use `temperature_unit()` for Celsius vs
Fahrenheit and `unit_system()` for wind speed. A calendar widget should use `first_day_of_week()` for week layout.

Handle unknown or missing values by rendering a simple fallback:

```rust
let format = system::current()
    .time_format()
    .unwrap_or(system::TimeFormat::Hour24);
```

## Use The Testbed

The WASM testbed always shows a System panel in the right sidebar. It is below the Params panel and applies to every
previewed size because system settings are global.

The panel contains controls for the fixed system snapshot:

- `timezone`
- `time_format`
- `date_format`
- `number_format`
- `first_day_of_week`
- `temperature_unit`
- `unit_system`
- `next_alarm`
- `night_mode`

Run a widget in the testbed from `bmc-wasm-runtime/`:

```bash
just dev <widget-name>
```

From the repository root, use the namespaced recipe:

```bash
just wasm::dev <widget-name>
```

On branches with production WASM widgets under `widgets-wasm/` (the BDK-293 layout), the same recipe accepts either an
SDK example widget or a production widget. The recipe resolves the widget's workspace, builds the widget for
`wasm32-unknown-unknown`, starts the testbed, and watches the widget crate for rebuilds:

```bash
just wasm::dev <production-widget-name>
```

Use the release-built variant when hot reload is not needed:

```bash
just wasm::run <production-widget-name>
```

If the local branch does not have the multi-root recipe yet, build the production widget and run the testbed directly
with the built `.wasm` file and the widget manifest. Cargo writes hyphenated package names with underscores in the wasm
filename:

```bash
cargo build --manifest-path widgets-wasm/Cargo.toml \
  -p <widget-name> --target wasm32-unknown-unknown

cargo run --features testbed --bin testbed \
  --manifest-path bmc-wasm-runtime/Cargo.toml -- \
  widgets-wasm/target/wasm32-unknown-unknown/debug/<widget_binary>.wasm \
  --manifest=widgets-wasm/<widget-name>/manifest.json
```

Changing a System control calls `deliver_system_update` for every previewed size. A widget that exports
`on_system_update` should react immediately. The `params-demo` example renders system fields next to params and
highlights changed system rows, so it is also the reference example for system-setting update behavior.
