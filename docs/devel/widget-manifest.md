# Widget Manifest Specification

This document covers the system-level concerns around widget manifests: where they live on disk, how the compositor
discovers them, how parsed manifests reach the runtime, and the rationale behind the validation rules.

**The field grammar is not duplicated here.** It is generated from the Rust types in `bmc-widget-manifest` and committed
as `bmc-widget-manifest/manifest.schema.json`. Open that file (or `cargo doc -p bmc-widget-manifest`) for the per-field
reference — type, length caps, regex patterns, allowed `ParamKind` variants, etc. The rustdoc on each public type and
field is propagated into the schema's `description` properties, so editor hover help and the schema agree by
construction.

Editor support: every shipping example `manifest.json` carries a
`"$schema": "../../../bmc-widget-manifest/manifest.schema.json"` reference. An editor with JSON Schema support (VS Code,
Helix with `taplo`, IntelliJ) lights up autocomplete on top-level fields and `ParamKind` variants, hover help sourced
from the rustdoc, and red-squiggles on structurally-invalid values.

## Overview

Each widget is distributed as a Nix package containing:

- `manifest.json` — the manifest (this specification's subject).
- Binary executable — Wayland client that talks the `deck_widget_v1` protocol.
- Assets — icons and preview images.

The main Deck application scans widget directories, reads manifests, and presents available widgets to users. When a
user creates a widget instance, the application spawns the binary as a Wayland client.

## Manifest Location

Widgets are installed into the Nix store and symlinked to a known location:

```
/nix/store/<hash>-<widget-name>/lib/bmc-widgets/<widget-name>/manifest.json
```

The system configuration symlinks installed widgets to a standard scan directory with separate subdirectories for
official and third-party widgets:

```
/usr/lib/bmc-widgets/
  official/
    <widget-name>/ -> /nix/store/<hash>-<widget-name>/...
  third-party/
    <widget-name>/ -> /nix/store/<hash>-<widget-name>/...
```

This separation enables easy factory reset by removing the entire `third-party` directory.

The main Deck application scans both `/usr/lib/bmc-widgets/official/` and `/usr/lib/bmc-widgets/third-party/` to
discover available widgets.

## Widget Directory Structure

```
/usr/lib/bmc-widgets/
  official/
    <widget-name>/
      manifest.json
      bin/
        <binary-name>
      assets/
        icon.png
        preview-small.png    # if small size supported
        preview-medium.png   # if medium size supported
        preview-large.png    # if large size supported
        preview-full.png     # if full size supported
  third-party/
    <widget-name>/
      ...
```

## Validation rules and rationale

Loading a manifest goes through two layers, intentionally separated.

**Layer 1 — structural constraints expressible in JSON Schema.** Encoded directly on the Rust types via `schemars`
attributes and enforced by any JSON Schema validator. Examples:

- `name` ≤ 50 characters, `description` ≤ 200 characters — keep the operator UI legible.
- `sizes` has at least one entry — the compositor cannot render a widget with no declared size.
- `ParamKey` matches `^[A-Za-z][A-Za-z0-9_-]*$` — keys must be stable identifiers safe to translate into Rust field
  names in generated typed accessors.
- `ParamKind::Integer.step` and `ParamKind::Double.step` are strictly positive — a zero step makes the operator UI's
  stepper meaningless.
- `default_value` literal types match the declared `ParamKind` — `default_value: 3.14` on a `boolean` is a typo, not a
  value the runtime should defend against.

**Layer 2 — cross-field invariants the schema cannot express.** Enforced by `ParamDefinition::validate` in
`bmc_widget_manifest`. Examples:

- `default_value` ∈ `enum_values` when both are present — the operator UI offers a closed set; the default must be one
  of them.
- `default_value` ∈ `[min, max]` for numeric variants — same reasoning at the range level.
- `min` ≤ `max`, `step > 0`, finite f64 bounds — guard against manifests that compile structurally but produce a UI the
  operator cannot use.
- `+0.0` / `-0.0` collide in `Double` `enum_values` dedup — JSON Schema treats them as distinct numbers; the runtime
  treats them as the same selection.
- Required params (i.e. `optional: false`) **must** declare a `default_value`. The compositor always delivers a complete
  params object to the widget on every `init` and `on_params_update`; this rule guarantees there is always a value to
  deliver. The widget never has to handle a missing required key.

The split lets editor-side tooling catch the structural errors as the operator types (red-squiggle on
`default_value: 3.14` for a boolean) while leaving the cross-field semantics where the load-time error message can name
the specific manifest field and reason.

## Runtime Behavior

When the compositor loads a widget instance:

1. `bmc-widget-manifest::Manifest::from_str` parses and validates the manifest; failures are surfaced before the widget
   binary is spawned.
2. The compositor merges manifest-declared defaults with operator-supplied overrides into a full
   `BTreeMap<ParamKey, ParamValue>` — every declared key has a value (the operator's, the manifest's default, or `Null`
   for optional keys without a default).
3. The widget binary is spawned as a Wayland client.
4. The compositor sends the full params object as JSON via the `deck_widget_v1.params` event on initial configure.
   Re-emission on operator-driven changes is tracked in [BDK-405](https://braiins.atlassian.net/browse/BDK-405).
5. The widget receives the params at startup (and, on a BDK-405-enabled branch, on subsequent operator changes) through
   whatever language-binding wraps the wayland event. The wasm host runtime exposes them via
   `bmc_wasm_sdk::params::current()` / `previous()` and the `on_params_update` lifecycle hook — see
   [BDK-432](https://braiins.atlassian.net/browse/BDK-432).
