# Host-side Formatting with User Preferences

## Context

The ISS widget (BDK-304) needed locale-aware formatting — the static deckfeeder widget uses
`format.speed(kmh, {decimals: 0})` which respects user preferences (unit system, number separators). The WASM runtime
had no preference mechanism.

This adds host-side formatting functions so widgets call `format_speed!(velocity, 0)` and get back a
preference-formatted string like "27 565 km/h" or "17,126 mph". The host owns the formatting logic (using `formato`
crate) and preferences; widgets just consume results.

The existing `LocalizationConfig` in `bmc/src/config.rs` defines all preference types. The protocol crate has
self-contained equivalents (no dependency on bmc/bmc-shared) so the WASM boundary stays clean. The real system maps from
`LocalizationConfig`; the testbed provides defaults.

## Design

### Preference types (protocol crate)

`protocol/src/format.rs` with three `#[repr(u8)]` enums and a preferences struct:

```rust
pub enum NumberFormat { SpaceComma=0, CommaDot=1, DotComma=2, SpaceDot=3 }
pub enum UnitSystem  { Metric=0, Imperial=1 }
pub enum TemperatureUnit { Celsius=0, Fahrenheit=1 }

pub struct FormatPreferences {
    pub number_format: NumberFormat,
    pub unit_system: UnitSystem,
    pub temperature_unit: TemperatureUnit,
}
```

Default: `SpaceComma` + `Metric` + `Celsius` (matches existing `LocalizationConfig` defaults).

### Host functions (runtime crate)

Three host functions following the `host_json_get_str` write-to-buffer pattern:

| Host function             | WASM signature               | Behavior                                     |
| ------------------------- | ---------------------------- | -------------------------------------------- |
| `host_format_number`      | `(f64, u32, u32, u32) → i32` | Format number with preference separators     |
| `host_format_speed`       | `(f64, u32, u32, u32) → i32` | Convert if imperial, format with unit suffix |
| `host_format_temperature` | `(f64, u32, u32, u32) → i32` | Convert if °F, format with unit suffix       |

Args: `(value, decimals, out_ptr, out_len) → actual_length`. Negative return = error.

The host reads `FormatPreferences` from `HostState` and uses `formato` for number formatting.

Conversion constants:

- Speed: km/h × 0.621_371_192 = mph
- Temperature: °C × 9/5 + 32 = °F

Unit suffixes: "km/h" / "mph", "°C" / "°F".

### SDK macros (sdk crate)

Three macros in `sdk/src/format.rs`, all using `$value as f64` for generic numeric input:

```rust
format_number!(value, decimals);       // "27 565" or "27,565"
format_speed!(value, decimals);        // "27 565 km/h" or "17,126 mph"
format_temperature!(value, decimals);  // "20,5 °C" or "68,9 °F"
```

Each macro calls an internal host function that declares a 64-byte stack buffer, calls the host, and returns a `String`
from the buffer slice.

### Runtime boundary

`WasmWidgetRuntime::new()` takes a `prefs: FormatPreferences` parameter, stored in `HostState` and read by formatting
host functions. Testbed passes `FormatPreferences::default()`.

### Files modified

- `protocol/src/format.rs` — `NumberFormat`, `UnitSystem`, `TemperatureUnit`, `FormatPreferences`
- `protocol/src/lib.rs` — `pub mod format` + re-export
- `Cargo.toml` — `formato.workspace = true`
- `src/runtime.rs` — `prefs` parameter, 3 host function registrations
- `src/bin/testbed.rs` — pass `FormatPreferences::default()`
- `sdk/src/format.rs` — host extern declarations, wrapper functions, macros
- `sdk/src/lib.rs` — re-export
- `examples/iss-position/src/lib.rs` — uses `format_speed!` and `format_number!` macros
