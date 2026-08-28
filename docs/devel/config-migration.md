# Config Migration on BMC Application Update

## Goal

Convert the BMC config (legacy `/etc/bmc_config.json`, new `/etc/bmc/config.json`) from the slint-monolith shape to the
manifest-driven shape on first boot of the new firmware, with no user action. Scene layouts and per-widget data for
widgets we recognise survive the upgrade; widgets we do not recognise are dropped with a warning rather than preserved
in a placeholder shape.

The user-facing behaviour is in [`docs/stories/config-migration.md`](../stories/config-migration.md). This document
captures the design decisions.

## Versioning

Typed-per-version, borrowed from `bos-main/open/bosminer/bosminer-config`. Each on-disk schema version is a distinct
Rust type. `LoadedConfig::from_str` reads the `version` header, dispatches to the matching parser, and upgrades to the
current schema in memory.

The chain has two hops today, both landing on the current schema: v0 → current via `upgrade_v0::upgrade_with_report`
(returning the upgraded `Config` plus a `Report` of what was translated and dropped) and v1 → current via
`upgrade_v1::upgrade` (an account reshape, no widget report). The shape pays off in two ways:

1. **In-memory upgrades.** `LoadedConfig::from_str` never touches the filesystem. Parsing any version produces the
   upgraded `Config` (and, for a legacy file, its migration `Report`); the caller decides whether, when, and how to
   persist.
2. **Each hop is localized.** v1 → v2 is exactly this: a `from_str` arm for version 1 and `upgrade_v1`, which reshapes
   only the accounts (the sole v1 → v2 change) and re-parses the rest as current. v0 → current stays one function that
   composes the widget translation with the shared account reshape (`upgrade_v1::reshape_and_collect_accounts`). A
   future v3 adds one more arm the same way — no trait chain imposed up front, designed per hop as real hops appear.

`Config` carries a top-level `version: u32` field (`#[serde(default)]`, so v0 configs deserialise as 0). The migration
builds the upgraded config through `Config::from_migrated_parts(scenes, accounts, settings)`, which pins `version` to
`CONFIG_VERSION` (currently 2); `Config::save` pins it again on every write as a belt-and-braces guard.

### Dispatch

```
read raw JSON
parse FormatHeader (version only)
match version:
  0 (or missing) → parse as v0::Config → upgrade_v0::upgrade_with_report(v0)
                   → LoadedConfig::MigratedFromV0 { current, report }
  1              → reshape accounts → upgrade_v1::upgrade → LoadedConfig::MigratedFromV1 { current }
  2 (current)    → parse as Config → LoadedConfig::AlreadyCurrent
  other          → bail with explicit error, do not read as config
```

Persistence is orthogonal and deferred: `save_with_backup(&Config, path)` writes the current shape and creates a
timestamped `.backup.<ts>` of the previous file, but the boot path does **not** call it. Boot loads and validates in
memory (`load_any_version`) and leaves the on-disk file untouched; a migrated config is written back only when the user
first changes a setting (see [Boot integration](#boot-integration)). The offline `bmc-migrate-config` tool is the one
caller that loads, validates, and persists in a single step.

## Per-widget upgrade policy

The upgrade is a **total function from known v0 widgets to current widgets**. Every v0 widget either maps to the
`widget_type_id` of a shipped native widget or drops out of the upgraded config entirely. There is no intermediate
"placeholder" shape — the review conclusion was that users migrate all their widgets at once, so an unmappable widget is
an edge case, not a state to preserve.

Every mapped widget targets the **real `uid` declared in a shipped `widgets-wasm/*/manifest.json`** — there are no
reserved dummies and no derived UIDs. (Earlier iterations reserved sequential UIDs for not-yet-shipped native kinds and
derived UUID v5 values for remote-widget slugs; both schemes were dropped once the native widget set stabilised — a
widget either has a shipped manifest to map to, or it drops.)

### Mapping table

Native v0 kinds:

| v0 `kind`           | Target manifest                  | Param translation                                                                    |
| ------------------- | -------------------------------- | ------------------------------------------------------------------------------------ |
| `clock`             | `widgets-wasm/clock`             | style/booleans pass through, font vocabulary remap, `timezone` → `timezone_override` |
| `block_height`      | `widgets-wasm/blockheight`       | `show_timestamp` pass-through, font vocabulary remap                                 |
| `halving_countdown` | `widgets-wasm/halving-countdown` | required font weight takes the manifest default                                      |
| `braiins_pool`      | `widgets-wasm/braiins-pool`      | style pass-through, chart window respell, `account_id` → `pool` credential binding   |
| `remote_image`      | `widgets-wasm/image`             | `refresh_duration` (humantime) → `refresh_seconds`, `image_scale_mode` → `sizing`    |

Legacy `remote_widget` entries, dispatched by URL slug under `https://widgets.braiinsforge.com/<slug>`:

| Slug                      | Target manifest                   | Param translation                                          |
| ------------------------- | --------------------------------- | ---------------------------------------------------------- |
| `weather`                 | `widgets-wasm/weather`            | `location` pass-through, `time_zone` pinned to default     |
| `nameday`                 | `widgets-wasm/nameday`            | `country` enum-guarded, camelCase `showDate` → `show_date` |
| `iss-position`            | `widgets-wasm/iss-position`       | none (manifest has no params)                              |
| `random-facts`            | `widgets-wasm/random-facts`       | none (manifest has no params)                              |
| `spacex-launch`           | `widgets-wasm/spacex-launch`      | none (legacy `showSeconds` drops with the mapping)         |
| `nasa-picture-of-the-day` | `widgets-wasm/picture-of-the-day` | both required params take the manifest default             |

Widgets with **no native counterpart drop**: native kinds `ticker_btc`, `blockchain_data`; remote slugs `exchange-rate`,
`formula-1`, `ticker-list`, `ticker-single-candlestick`, and `ticker-single-sparkline`.

### Required params are always filled

The boot-load path hands stored params to the widget verbatim — unlike the gRPC add-widget path
(`validate_widget_params`), it injects **no manifest defaults**. A widget's generated param reader panics on a missing
required key. The migration therefore emits **every required param of the target manifest**: a present, valid v0 value
passes through (translated where the schema changed); anything absent or malformed gets the manifest default written
explicitly. Enum-typed params are guarded against out-of-vocabulary values for the same reason (an out-of-enum string
also panics the typed read). Optional params (`timezone_override`) are emitted only when the legacy value is valid, and
stay unset otherwise.

### Drop policy (no placeholders)

- Unknown top-level `kind` → drop with `warn!`.
- `remote_widget` missing `widget_url` → drop with `warn!`.
- `remote_widget` with a host outside `widgets.braiinsforge.com` → drop with `warn!`.
- `remote_widget` whose slug has no native equivalent → drop with `warn!`.

Malformed *param values* on a mappable widget never drop the widget — the value falls back to the manifest default (e.g.
a `clock_style` outside the enum migrates as `digital`).

Drops are counted in `Report.dropped_widgets`. The scene itself always survives; only the individual widget entry
disappears (a scene left with zero widgets is dropped and counted in `Report.dropped_scenes`).

### Active widget limit

The v0 migration counts enabled scenes in stored order against `crate::config::MAX_RUNNING_WIDGETS`. If all widgets in
the next enabled scene would exceed the remaining budget, migration retains but disables the whole scene, increments
`Report.deactivated_scenes`, and continues scanning so a later smaller scene can still fit. The offline migration CLI
reports this count as `deactivated_scenes`.

The capacity clamp is specific to v0 migration. Later configuration versions are not rewritten on startup; runtime gRPC
operations prevent further capacity increases while disable and removal operations remain available.

### Malformed scene geometry

The v0 schema stored each widget's `size` independently of its scene kind, so it could express layouts the current
schema forbids — most commonly a `full` (or unknown, hence full-defaulted) size inside a `combined` scene, which becomes
a fullscreen placement that `Config::validate_scenes` rejects. Such geometry is migrated as-is, not salvaged: the
resulting config fails validation, so the migration is never persisted and the boot path drops the whole config (backing
the original up and replacing it with the platform default). This is only reachable from a hand-edited or corrupt v0
file — a config written by an older BMC app is always internally consistent — so dropping the whole config is preferred
over the added complexity of per-scene geometry salvage.

## Settings pass-through and account reshape

The top-level settings kept their shape across the schema change: `scene_cycling`, `localization`, `data_collection`,
`brightness_pct`, `night_mode`, `sound_volume_pct`, `alarms`, `led_enabled`, `boot_sound_enabled`, `autoupgrade`. Each
is carried as raw JSON in `v0::Config` and re-parsed into its current typed form during the upgrade
(`passthrough_setting`) — a validate step, not a transformation. A value that fails the re-parse is dropped with a
`warn!` naming the field; a single bad setting never fails the migration, and the dropped field falls back to the same
default a field-less current config would use.

**Accounts are transformed, not passed through** (as of v2). A v1 account
(`{ type: "braiins_pool", authentication: { api_key } }`) becomes a typed credential instance
(`{ type_id: "braiins-pool", field_values: { token } }`) via `upgrade_v1::reshape_legacy_account`. v0 carries its
accounts as raw JSON of the same pre-typed shape, so the v0 → current path runs the same reshape
(`reshape_and_collect_accounts`); an entry that doesn't match is dropped with a `warn!`.

## Safety

- **Backup first.** `<path>.backup.<unix_secs>` is written before the new config, by `save_with_backup`. Plain integer
  suffix sorts naturally, no locale surprises.
- **Atomic write** via `crate::utils::replace_file` (tmp + rename).
- **Total parse failure** surfaces as an explicit `anyhow::Error` from `LoadedConfig::from_str`; the existing file is
  left untouched (no partial rewrite).
- **Validate before persist.** The boot path validates the upgraded config in memory and, because it never writes at
  boot, a bad migration cannot overwrite the on-disk original at all — the original stays in place and the device
  recovers onto a default (backing the unreadable file up to `<path>.bcp`). The offline `bmc-migrate-config` tool
  applies the same validate-before-write rule to `<dst>`.

## Boot integration

`ConfigHandle::init` runs `load_and_validate` on the runtime config path before anything else reads it, so the upgrade
happens once on first boot with no user action. Loading upgrades a legacy config **in memory** and validates it; the
on-disk file is **not** rewritten. An already-current config is a plain no-op.

**Migration is committed on the first genuine change, not at boot.** The handle carries a `migrated` flag; the first
`ConfigHandle::save` from a real settings change writes one timestamped backup of the pre-migration file and then
persists the upgraded config, clearing the flag. Until then the on-disk file keeps its original version. This is what
keeps a would-be downgrade safe: if the user upgrades the BMC application and rolls back before changing anything, the
older application still finds its own config on disk.

On a load failure the file is backed up to `<path>.bcp` and replaced with a platform default. **Downgrades are not
supported**, so a config whose version is newer than this application understands is treated the same as any other
unreadable file — backed up and replaced, not preserved in place. There is deliberately no read-only mode and no
newer-version write guard: the review concluded the precaution wasn't warranted, since the on-disk-stays-old property
above already covers the benign downgrade case and an unsupported downgrade landing on defaults is acceptable.

> Follow-up: an unreadable config currently boots the rest of the device on defaults rather than failing only the
> display subsystem while keeping the web UI reachable. Splitting boot so the display alone fails is filed as a
> follow-up.

## Compositor coordination

`Coordinator::spawn_widget` short-circuits with an `info!` log for a widget carrying a nil `widget_type_id`. With
placeholders gone the migration never emits nil UIDs, so the guard is purely defensive (against a hand-edited or
malformed config). Every UID the migration emits belongs to a shipped `widgets-wasm` manifest, so a migrated widget
always resolves to a registered widget.

## Config path layout

The runtime config and its timestamped backups live under `/etc/bmc/`:

```
/etc/bmc/config.json
/etc/bmc/config.json.backup.<unix_secs>
```

This is a directory-based layout so OpenWRT's `sysupgrade` can preserve everything under `/etc/bmc/` with a single
conffile rule (the sibling change in the bos-main packaging layer adds `/etc/bmc` to the conffile set).

The directory is created lazily by the config-save path itself: `crate::utils::replace_file` runs `create_dir_all` on
the target's parent before writing. So a factory-fresh device with no `/etc/bmc/` still persists its first default
config (and every later edit) rather than failing every save with `ENOENT`; nothing outside the binary needs to
pre-create the directory.

### Legacy path relocation

On first boot of the new firmware the old file at `/etc/bmc_config.json` is **copied** (not moved) to
`/etc/bmc/config.json` (see `relocate_legacy_config_if_present` in `bmc/src/config_migration.rs`). Triggered implicitly
by `load_any_version`:

- new path exists → leave everything alone
- new path missing, legacy present → create `/etc/bmc/`, copy the legacy file in, leave the original untouched; the
  version dispatch then runs against the new path as normal
- neither present → fresh install, nothing to relocate

**Copy, not move.** The legacy path is preserved deliberately so a forced boot into the older firmware (debugging,
emergency rollback) still finds its config at the path it expects. That snapshot goes stale the moment the new firmware
writes an edit, but the "boot old firmware at pre-upgrade state" fallback stays available indefinitely at the cost of a
few KB of on-disk redundancy. Aligns with the rest of the migration's "never silently destroy user data" stance.

The pattern is path-shape based (`<parent>/bmc/<name>` looks for `<parent>/bmc_<name>`) so tests using tmp dirs exercise
the same code path.

### Support-archive collection

A bad migration is diagnosed from the pre-migration state, so the support archive (`bmc-support`) collects the whole
`/etc/bmc/` directory (current config plus every `config.json.backup.<ts>`) **and** the deliberately-kept legacy
`/etc/bmc_config.json`. Credential censoring matches the whole config family — the current config, its backups, and the
legacy file — via a path predicate (`BmcConfigCensor::matches`) rather than a single fixed path, so a newly-created
backup is never archived uncensored. Note: the censor currently matches only the legacy `"api_key"` key; the v2 account
reshape moves secrets into `field_values` (`token`/`password`), so broadening it is tracked as a Phase-G follow-up.

## Open items / follow-ups

- **Cap kept backups.** Keep at most N (≈10) `config.json.backup.<ts>` files, rotating the oldest out. Filed as
  technical debt.

## Testing

- **Unit:** per-widget upgrade tests (native kinds map to their manifest UID with every required param filled, param
  translations and enum guards, remote slugs with a native equivalent map and the rest drop, unknown kinds / URLs drop),
  plus header-dispatch edge cases (missing version, current version, unknown future version, empty v0 input).
- **Integration:** round-trips a captured device config through the load→validate→persist chain and asserts: version
  header, scene count, every post-upgrade widget carries a non-nil manifest `widget_type_id`, no retired placeholder
  param shape (`_legacy` / `_legacy_remote`) leaks, backup file exists, `translated` counter matches the on-disk widget
  count. An oversized v0 fixture verifies whole-scene deactivation and continued scanning up to the active-widget
  boundary. Plus an invalid-migration test asserting the original file is left intact when validation fails, and a test
  for the pure `LoadedConfig::from_str` path (`load_is_pure_without_persist`) verifying it upgrades in memory without
  touching disk.
- **CLI:** `bmc-migrate-config <src> <dst>` exits 0, writes `<dst>`, and emits a counts report
  (`cli_smoke_migrates_fixture_and_reports_counts`); and refuses — non-zero exit, no `<dst>` written — a config that
  fails validation (`cli_refuses_to_write_a_config_that_would_fail_validation`), matching the boot path's
  validate-before-persist rule.
- **Boot path:** `ConfigHandle::init` migrating a seeded legacy config in memory (on-disk file left at its old version,
  then committed with one backup on the first save), and treating a newer-than-known config as unreadable (backed up to
  `.bcp` and replaced with a default).

## Files

- `bmc/src/config.rs` — `version` field, configuration constants, `Config::from_migrated_parts`, and the
  `ConfigHandle::init` boot wiring (`load_and_validate` / `recover_from_failed_load`) plus the `migrated`-flag
  commit-on-first-save.
- `bmc/src/config_migration.rs` — `LoadedConfig`, `FromStr` version dispatch, `load_any_version`, `save_with_backup` /
  `backup_existing`, `Report`.
- `bmc/src/config_migration/v0.rs` — deserialize-only v0 types.
- `bmc/src/config_migration/upgrade_v0.rs` — active-widget limiting, manifest UID constants, remote-widget slug
  dispatch, per-widget translators (required-param filling, enum guards), and `params_from_value` (legacy JSON params →
  typed param map).
- `bmc/src/config_migration/upgrade_v1.rs` — the v1 → current account reshape (`reshape_legacy_account`), shared with
  the v0 path.
- `bmc/src/widget/coordinator.rs` — defensive nil-UID skip (the migration no longer produces nil UIDs; the guard is
  harmless).
- `bmc/src/bin/migrate_config.rs` — offline CLI.
- `bmc/tests/config_migration.rs` — integration tests.
- `bmc/tests/fixtures/legacy_config_sample.json` — captured sample.
- `bmc/tests/fixtures/active_widget_limit_v0.json` — oversized active-widget sample.
