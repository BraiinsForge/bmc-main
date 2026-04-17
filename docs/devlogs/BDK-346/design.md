# BDK-346: Config Migration

## Goal

Convert `/etc/bmc_config.json` from the slint-monolith shape to the
manifest-driven shape on first boot of the new firmware, with no user
action. Scene layouts and per-widget data are preserved as completely
as possible; widgets without a destination today leave placeholders
that a future firmware can promote.

The user-facing behaviour is in
[`docs/stories/config-migration.md`](../../stories/config-migration.md).
This document captures the design decisions.

## Versioning

Typed-per-version, borrowed from
`bos-main/open/bosminer/bosminer-config`. Each on-disk schema version
is a distinct Rust type that implements a [`Version`] trait. Older
versions implement [`Upgrade`] with a concrete `NextVersion`
associated type, so parsing any version and walking to the latest
becomes a chain of trait method calls the compiler enforces.

The chain today is short (v0 → current), but the shape pays off in
three ways:

1. **In-memory upgrades.** `LoadedConfig::from_str` never touches the
   filesystem. Parsing any version produces both the upgraded
   `Config` and the preserved pre-upgrade struct; the caller decides
   whether, when, and how to persist. This decouples the "read and
   understand a config" concern from the "back up + rewrite the
   file" concern.
2. **The original parse survives.** `LoadedConfig::original_v0()`
   gives callers (debug endpoints, rollback UIs, CI snapshot tests)
   the parsed `v0::Config` without re-reading disk. After persistence
   the on-disk file is already rewritten; boser-style keeps the
   original value live in memory.
3. **Adding v2 is mechanical.** Define `ConfigV2`, `impl Upgrade for
   ConfigV1 { type NextVersion = ConfigV2; fn upgrade_to_next_version
   ... }`, extend the `FromStr` match arm. No dispatcher rework, no
   reshuffling of the ingest path.

`Config` gains a top-level `version: u32` field (`#[serde(default)]`,
so v0 configs deserialise as 0). `Default::default()` and saves pin
it to `CONFIG_VERSION` (currently 1) via the `Version` trait.

### Dispatch

```
read raw JSON
parse FormatHeader (version only)
match version:
  0 (or missing) → parse as v0::Config → v0.upgrade_to_next_version()
                   → preserve v0 inside LoadedConfig::MigratedFromV0
  1 (current)    → parse as Config → LoadedConfig::AlreadyCurrent
  other          → bail with explicit error, do not read as config
```

Persistence is orthogonal: `save_with_backup(&Config, path)` writes
the current shape and creates a timestamped `.backup.<ts>` of the
previous file. `migrate_on_disk(path)` composes the two — load, and
persist if the load was a migration.

### Patterns deliberately not adopted

- **`#[serde(deny_unknown_fields)]`.** The current `Config` has many
  optional fields used by deployed devices; turning this on would
  break real configs.
- **`Downgrade` trait.** bos-main ships it as a symmetric counterpart
  to `Upgrade` for backward-compatible writes. BDK's product stance
  is "newer schema on disk = refuse, never touch" — we don't
  downgrade configs. If that stance changes the trait can be added
  without reshaping the upgrade chain.

## Per-widget outcomes

Each v0 widget falls into one of three buckets. `position`, `size`,
scene assignment are preserved in every case. The upgrade function
produces a fully-typed `Config`; the on-disk shape of placeholders
remains a reserved-key convention so a future firmware can promote
placeholders without schema churn.

| Bucket          | Trigger                          | New `widget_type_id` | New `params`                                                            |
|-----------------|----------------------------------|----------------------|-------------------------------------------------------------------------|
| `Translated`    | Native manifest exists today     | manifest UID         | re-shaped to match the new manifest                                     |
| `LegacyRemote`  | v0 `kind == "remote_widget"`     | `Uuid::nil()`        | `{"_legacy_remote": {name, description, widget_url, icon_url, params}}` |
| `Unavailable`   | Any other kind without a target  | `Uuid::nil()`        | `{"_legacy": {kind, params}}`                                           |

### Why two placeholder shapes

Old `remote_widget` entries already carried name + description + URL
+ icon — essentially a remote-manifest snapshot. A future WASM
remote-widget host wants exactly that shape. Squashing both into a
single `_legacy` would drop metadata and force users to re-enter
URLs.

### Today's translators

- `clock` + `clock_style: "digital"` → `digital-clock` manifest.
  Param mapping: `show_seconds → showSeconds`,
  `show_timezone → showTimezone`,
  `numbers_font_style → fontStyle`. `show_date` is dropped (no
  equivalent on the new manifest); a warning is logged.
- `remote_widget` → `LegacyRemote` (preservation, no native target).
- everything else → `Unavailable`.

Adding a translator = one match arm + one pure function.

## Safety

- **Backup first.** `/etc/bmc_config.json.backup.<unix_secs>` is
  written before the new config, by `save_with_backup`. Plain integer
  suffix sorts naturally, no locale surprises.
- **Atomic write** via `crate::utils::replace_file` (tmp + rename).
- **Per-widget failure is local.** A widget that errors during
  translation lands in `Unavailable`; the rest of the scene is
  unaffected.
- **Total parse failure** surfaces as an explicit `anyhow::Error`
  from `LoadedConfig::from_str`; the existing file is left untouched
  (no partial rewrite).

## Compositor coordination

`Coordinator::spawn_widget` short-circuits with an `info!` log when
`widget.widget_type_id.is_nil()`. Without this, every placeholder
would log a "widget not found" error every boot.

## Testing

- **Unit:** per-translator inline JSON fixtures exercising the typed
  upgrade directly (`upgrade_widget`), plus header-dispatch edge
  cases (missing field, current version, unknown future version,
  empty v0 input).
- **Integration:** round-trips a captured device config
  (`bmc/tests/fixtures/legacy_config_sample.json`) through
  `migrate_on_disk` and asserts version, scene count, placeholder
  payloads, backup file presence, plus a test for the pure
  `LoadedConfig::from_str` path verifying `original_v0()` survives
  the upgrade.
- **CLI smoke:** `bmc-migrate-config <src> <dst>` exits 0 and emits
  a counts report. Catches translator coverage regressions.

## Files

- `bmc/src/config.rs` — `version` field, `CONFIG_VERSION` constant.
- `bmc/src/config_migration.rs` — `Version`/`Upgrade` traits,
  `LoadedConfig`, `FromStr`, `load_any_version`, `save_with_backup`,
  `migrate_on_disk`.
- `bmc/src/config_migration/v0.rs` — deserialize-only v0 types +
  `impl Version`.
- `bmc/src/config_migration/upgrade_v0.rs` — `impl Upgrade for
  v0::Config` producing a typed current `Config` directly.
- `bmc/src/widget/coordinator.rs` — placeholder skip.
- `bmc/src/bin/migrate_config.rs` — offline CLI.
- `bmc/tests/config_migration.rs` — integration tests.
- `bmc/tests/fixtures/legacy_config_sample.json` — captured sample.
