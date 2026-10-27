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
- Binary executable — Wayland client that talks the `deck_widget` protocol.
- Assets — optional files referenced by the widget implementation or package.

The main Deck application scans widget directories, reads manifests, and presents available widgets to users. When a
user creates a widget instance, the application spawns the binary as a Wayland client.

## Manifest Location

Each widget is a package in the device's Nix profile, carrying its widget folder under `lib/bmc-widgets/`:

```
/nix/store/<hash>-<widget-name>/lib/bmc-widgets/<widget-name>/manifest.json
```

A profile generation merges the packages it selects into one symlink tree (see [`profiles.md`](profiles.md)). The BMC
application discovers widgets by scanning that tree's `/run/current-profile/lib/bmc-widgets/`:

- a folder holding a `manifest.json` at most three levels down is a widget; the walk follows links and does not descend
  past a widget;
- an official widget sits at `<widget-name>/`;
- an uploaded third-party widget sits under its maintainer's folder, at `<maintainer>/<widget-name>/`, and its binary is
  `bin/<widget-name>` (see [`uploading-wasm-widgets.md`](../uploading-wasm-widgets.md)).

## Widget Directory Structure

```
/run/current-profile/lib/bmc-widgets/
  <widget-name>/
    manifest.json
    bin/
      <binary-name>
      <binary-name>-migrate-params   # optional, see Programs
    assets/
      icon.png
      ...
  <maintainer>/
    <widget-name>/
      manifest.json
      bin/
        <widget-name>
      ...
```

## Validation rules and rationale

Loading a manifest goes through two layers, intentionally separated.

**Layer 1 — structural constraints expressible in JSON Schema.** Encoded directly on the Rust types via `schemars`
attributes and enforced by any JSON Schema validator. Examples:

- `name` ≤ 50 characters, `description` ≤ 200 characters — keep the operator UI legible.
- `supported_viewports` has at least one entry — the compositor cannot add a widget when no declared constraint matches
  the active viewport.
- `ParamKey` matches `^[A-Za-z][A-Za-z0-9_-]*$` — keys must be stable identifiers safe to translate into Rust field
  names in generated typed accessors.
- `ParamKind::Integer.step` and `ParamKind::Double.step` are strictly positive — a zero step makes the operator UI's
  stepper meaningless.
- `default_value` literal types match the declared `ParamKind` — `default_value: 3.14` on a `boolean` is a typo, not a
  value the runtime should defend against. This holds for scalar kinds; a list's items are checked against `items` in
  Layer 2.
- An `array` param's `max_items` is within `1..=100` (`MAX_ARRAY_ITEMS`), and its `items` is a scalar kind or an
  `object` whose `fields` are all scalars — so a list never nests another list or object.

**Layer 2 — cross-field invariants the schema cannot express.** Enforced by `ParamDefinition::validate` in
`bmc_widget_manifest`. Examples:

- Every `default_value`, whether on a param, a list item kind or an object field, passes the same checks an operator's
  value must: enum options, `[min, max]`, finiteness, the timezone list, the string length cap and length bounds —
  except that a required param may default to `""`, as below.
- `enum_control: radio` only alongside `enum_values` — a radio group with no options has nothing to offer.
- `min` ≤ `max`, `step > 0`, finite f64 bounds — guard against manifests that compile structurally but produce a UI the
  operator cannot use.
- Every number `enum_values` option lies within `min` and `max` — an option outside them could never be saved.
- A number's `unit`, when set, is not blank.
- An edit never saves a required string empty, whether a param, a list item or a required object field: the operator UI
  reads empty text as no value. A list item's or required object field's default is never empty either. A required param
  may still default to `""`, as 26.09 manifests do; a new instance starts with it, and the widget receives `""` until an
  edit saves a value.
- `min_length` ≤ `max_length` ≤ 1024 (`MAX_PARAM_STRING_LENGTH`), and every `enum_values` option fits those bounds — an
  option outside them could never be saved.
- `+0.0` / `-0.0` collide in `Double` `enum_values` dedup — JSON Schema treats them as distinct numbers; the runtime
  treats them as the same selection.
- Required params (i.e. `optional: false`) **must** declare a `default_value`, so a scene saved against the current
  manifest stores a value for every key. Params stored before the widget required a key get its default at launch, or
  from the migrator's result when the package ships one ([Programs](#programs)). A failed migration, or a stored value
  the package rejects, launches the params as stored, so the key stays missing. An `array` param is the exception: an
  omitted `default_value` is the empty list.
- `array` params cannot be `optional` — `min_items: 0` is how a list says it may be empty, so the widget always receives
  a list, never `null`.
- `min_items` ≤ `max_items`, and an array's `default_value` holds between `min_items` and `max_items` items, each valid
  against `items` and, under `unique_items`, none repeated — the same check an operator's list must pass.
- `unique_items` keys go only on object items, and name at least one field, each a declared field and none twice; this
  one is checked while the manifest is parsed.
- Under `unique_items`, neither a scalar item nor a named key field declares a `default_value`, and an object row keeps
  at least one required, non-boolean field without one: every item the operator adds starts at those defaults, so the
  second would start as a repeat.
- An `object` item declares at least one field, and no field key twice.

Viewport constraints are also validated after parsing:

- `supported_viewports` must not be empty.
- Min/max width, height, and DPI bounds must be nonzero when present.
- Min bounds must not exceed max bounds.
- Duplicate constraints are rejected.

The split lets editor-side tooling catch the structural errors as the operator types (red-squiggle on
`default_value: 3.14` for a boolean) while leaving the cross-field semantics where the load-time error message can name
the specific manifest field and reason.

## Programs

`programs` lists helper programs the package ships beside `binary`. Each entry has:

- `path`: a non-empty relative path of plain names — no leading `/` or `./`, no `..` — so an uploaded manifest cannot
  name a program the package does not ship;
- `protocol` (default `0`): a protocol the BMC application does not implement is skipped with a warning, and the package
  still loads.

```json
{
  "programs": {
    "params_migrator": { "path": "bin/<name>-migrate-params", "protocol": 0 }
  }
}
```

Only index packages built by this repository may declare `programs`. The installer refuses an uploaded package that
does, until the device can verify the programs.

### `params_migrator`

Rewrites one instance's stored params when their stamp predates the installed package version, and restamps them with
that version on success. Discovery refuses the package when the declared file is missing, not a regular file or not
executable.

Protocol 0:

- **argv:** none.
- **cwd:** the widget directory.
- **env:** the BMC application's environment.
- **stdin:** one JSON object: `params_version` (the stamp, or `null` for a shape older than the package's first migrator
  or for code defaults), `widget_version` (the installed version), the instance's stored `params`, and the `width`,
  `height`, `viewport_shape` and `display` it is spawned with. No credentials, no token and no settings.
- **stdout:** one JSON object, an overlay of params keys. `{}` means the stored shape is current. The result is the
  stored params minus keys the manifest no longer declares, with the overlay applied, validated against the manifest
  with defaults filling what is still missing.
- **stderr:** logged line by line, up to 64 KiB; the rest is drained unread.
- **failure:** a non-zero exit, stdout that is not a JSON object, stdout above 256 KiB, a run above 5 seconds (the
  program is killed), or a result the manifest rejects. The stored params stay as they were and the attempt repeats at
  the next gate. The limits are constants in `bmc/src/widget/params_migration.rs` and can be raised when a shipped
  migrator needs more.
- **current shape:** answer `{}`. Code defaults and scenes written before the stamp existed carry no `params_version`,
  so the program sees `null` for both an old shape and the current one.

#### Gates

- They run at boot before any widget starts, and on a package reload before the changed instances are replaced.
- They run the selected instances four at a time (`MIGRATOR_CONCURRENCY`, beside the limits above). The cap keeps a
  scene set's worth of migrator processes from running at once while bounding the gate's wall time.
- A gate holds the scenes. An edit through the scene-management RPCs waits for it, up to eight seconds
  (`SCENE_EDIT_WAIT` in `bmc/src/widget/coordinator.rs`), and is then refused as `unavailable` so the client retries. An
  edit never lands between the registry swap and the instance replacement, where it would validate against the new
  manifest while the old instance still runs.
- A gate in turn waits for the edits already running, widget stops and spawns included, so an edit queued behind it
  waits for both.
- A migrated result the device cannot write to disk stays in memory, like any other
  [config save](config-persistence.md); the next boot runs the migrator again from the stamp the file still holds.

#### Failed Migrations

After a failure the instance starts on its stored params. A widget that requires a key the stored shape lacks then fails
to start until a later gate succeeds. The BMC application deliberately does not fill in manifest defaults instead:

- Defaults are chosen per key and know nothing of the other keys, so a filled shape can be internally inconsistent (a
  unit that no longer matches its value, a list whose rows no longer match their selector).
- A package that ships a migrator has said its params need more than defaults. Filling them in would run the widget on
  params nobody chose and nothing validated as a whole.
- A correct migration is the package's responsibility: a widget that does not start after a package upgrade is a bug in
  its migrator, not in the gate.

#### Packages Without A Migrator

Nothing runs at the gates. The instances launch on their stored params read through the manifest, as the settings form
reads them:

- undeclared keys drop and missing ones take their defaults;
- the read is never saved: the config changes only when an edit saves the widget, which stamps it;
- params the manifest rejects launch as stored, logged.

A package whose params need more than defaults, such as a renamed key or a changed unit, ships a migrator.

#### Downgrades

A package downgrade leaves stamps newer than the installed version. No migrator runs for them and nothing is saved, so
the newer package finds its params intact when it is reinstalled. Meanwhile the older package launches on them read
through its own manifest, as above.

#### Writing One

- Write a Rust binary crate under `migrators/<name>/`, a member of the main workspace: the Deck has no jq, and serde
  reads the input as the BMC application writes it. `migrators/ticker-list` is the reference.
- Declare it with `defineCrate` in `nix/crates.nix` and list it in `widgetPrograms` in `nix/wasm-widgets.nix`. That
  builds it for the package's target and installs it into the package's `bin/`; the package build fails when a declared
  program is missing there.
- Test it against the shipped manifest from `bmc-widget-manifest-tests`, through its entry in the root
  `[workspace.dependencies]`, never from `bmc`: a migrator in the BMC application's dependencies, even a dev-dependency,
  rebuilds the application on every migrator change.

## Runtime Behavior

When the compositor loads a widget instance:

1. `bmc-widget-manifest::Manifest::from_str` parses and validates the manifest; failures are surfaced before the widget
   binary is spawned.
2. The scene-management path checks the selected placement against the manifest's `supported_viewports`. The derived
   descriptor contains viewport shape, width, height, and DPI; each field must fall inside one of the manifest's
   inclusive constraints.
3. Saving a scene checks the params against the manifest and stores every declared key, defaults included. Adding a
   widget stamps them with the installed widget version (`params_version`); an update restamps only when the values
   change. For every instance whose stamp predates the installed version, at boot and after a package reload and before
   the widget spawns, the BMC application runs the package's `programs.params_migrator` when it declares one. The
   compositor sends the stored params read through the manifest, without saving that read; an instance whose migration
   failed gets its stored params as they are, see [Programs](#programs).
4. The widget binary is spawned as a Wayland client.
5. The compositor sends viewport/display geometry and the params from step 3 as JSON via the `deck_widget` initial
   configure batch.
6. Geometry-stable params changes re-emit the complete params object on the existing widget surface. The wasm host
   runtime exposes params via `bmc_wasm_sdk::params::current()` / `previous()` and the `on_params_update` lifecycle
   hook.
