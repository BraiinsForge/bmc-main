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

Inspired by `bos-main/open/bosminer/bosminer-config`, which has
handled config migrations across four major versions and survived
without major rework. Two patterns adopted:

- **Lazy header peek.** A tiny `FormatHeader { version: u32 }` is
  parsed first to decide which arm to dispatch to. Avoids a full
  parse of an unknown schema.
- **Explicit rejection of unknown future versions.** A v999 config on
  a v1 binary errors out rather than silently overwriting. Prevents
  data loss on accidental downgrade.

Two patterns deliberately not adopted:

- **Trait chain (`Upgrade`/`Downgrade` with associated types).**
  Overkill for one migration step; introduce when v2 lands.
- **`#[serde(deny_unknown_fields)]`.** The current `Config` has many
  optional fields used by deployed devices; turning this on would
  break real configs.

`Config` gains a top-level `version: u32` field
(`#[serde(default)]`, so legacy configs deserialise as 0).
`Default::default()` and `Config::save()` pin it to `CONFIG_VERSION`
(currently 1).

Dispatch:

```
read /etc/bmc_config.json
parse FormatHeader (version only)
match version:
  0 (or missing) → backup → translate → write new config (version: 1)
  1              → no-op
  other          → bail with explicit error, do not overwrite
```

## Per-widget outcomes

Each legacy widget falls into one of three buckets. `position`,
`size`, scene assignment are preserved in every case.

| Bucket          | Trigger                          | New `widget_type_id` | New `params`                                                            |
|-----------------|----------------------------------|----------------------|-------------------------------------------------------------------------|
| `Translated`    | Native manifest exists today     | manifest UID         | re-shaped to match the new manifest                                     |
| `LegacyRemote`  | Legacy `kind == "remote_widget"` | `Uuid::nil()`        | `{"_legacy_remote": {name, description, widget_url, icon_url, params}}` |
| `Unavailable`   | Any other kind without a target  | `Uuid::nil()`        | `{"_legacy": {kind, params}}`                                           |

### Why two placeholder shapes

Old `remote_widget` entries already carried name + description + URL
+ icon — essentially a remote-manifest snapshot. A future WASM
remote-widget host wants exactly that shape. Squashing both into a
single `_legacy` would drop metadata and force users to re-enter URLs.

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
  written before the new config. Plain integer suffix sorts naturally,
  no locale surprises.
- **Atomic write** via `crate::utils::replace_file` (tmp + rename).
- **Per-widget failure is local.** A widget that errors during
  translation lands in `Unavailable`; the rest of the scene is
  unaffected.
- **Total parse failure** still produces a backup; the device boots
  with an empty config so support can recover from the backup.

## Compositor coordination

`Coordinator::spawn_widget` short-circuits with an `info!` log when
`widget.widget_type_id.is_nil()`. Without this, every placeholder
would log a "widget not found" error every boot.

## Testing

- **Unit:** per-translator inline JSON fixtures + version detection
  edge cases (missing field, explicit 1 / 2, malformed input).
- **Integration:** round-trips a captured device config
  (`bmc/tests/fixtures/legacy_config_sample.json`) through
  `migrate_in_place` and asserts version, scene count, placeholder
  payloads. Plus current-version no-op and unknown-future-version
  rejection.
- **CLI smoke:** `bmc-migrate-config` against the fixture exits 0
  and emits a counts report. Catches translator coverage regressions.

## Files

- `bmc/src/config.rs` — `version` field, `CONFIG_VERSION` constant.
- `bmc/src/config_migration.rs` — `FormatHeader` peek, dispatch,
  backup, atomic write.
- `bmc/src/config_migration/legacy.rs` — deserialise-only legacy types.
- `bmc/src/config_migration/translate.rs` — outcome enum, per-kind
  translators, report.
- `bmc/src/widget/coordinator.rs` — placeholder skip.
- `bmc/src/bin/migrate_config.rs` — offline CLI.
- `bmc/tests/config_migration.rs` — integration tests.
- `bmc/tests/fixtures/legacy_config_sample.json` — captured sample.
