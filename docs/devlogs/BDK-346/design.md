# BDK-346: Config Migration

## Goal

Convert the BMC config (legacy `/etc/bmc_config.json`, new `/etc/bmc/config.json`) from the slint-monolith shape to the
manifest-driven shape on first boot of the new firmware, with no user action. Scene layouts and per-widget data for
widgets we recognise survive the upgrade; widgets we do not recognise are dropped with a warning rather than preserved
in a placeholder shape.

The user-facing behaviour is in [`docs/stories/config-migration.md`](../../stories/config-migration.md). This document
captures the design decisions.

## Versioning

Typed-per-version, borrowed from `bos-main/open/bosminer/bosminer-config`. Each on-disk schema version is a distinct
Rust type that implements a \[`Version`\] trait. Older versions implement \[`Upgrade`\] with a concrete `NextVersion`
associated type, so parsing any version and walking to the latest becomes a chain of trait method calls the compiler
enforces.

The chain today is short (v0 → current), but the shape pays off in three ways:

1. **In-memory upgrades.** `LoadedConfig::from_str` never touches the filesystem. Parsing any version produces both the
   upgraded `Config` and the preserved pre-upgrade struct; the caller decides whether, when, and how to persist.
2. **The original parse survives.** `LoadedConfig::original_v0()` gives callers (debug endpoints, rollback UIs, CI
   snapshot tests) the parsed `v0::Config` without re-reading disk. After persistence the on-disk file is already
   rewritten; boser-style keeps the original value live in memory.
3. **Adding v2 is mechanical.** Define `ConfigV2`, `impl Upgrade for ConfigV1 { type NextVersion = ConfigV2; … }`,
   extend the `FromStr` match arm.

`Config` carries a top-level `version: u32` field (`#[serde(default)]`, so v0 configs deserialise as 0). The migration
builds the upgraded config through `Config::from_migrated_parts(scenes, accounts, settings)`, which pins `version` to
`CONFIG_VERSION` (currently 1); `Config::save` pins it again on every write as a belt-and-braces guard.

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

Persistence is orthogonal: `save_with_backup(&Config, path)` writes the current shape and creates a timestamped
`.backup.<ts>` of the previous file. `migrate_on_disk(path)` composes the two — load, and persist if the load was a
migration.

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

| v0 `kind`      | Target manifest            | Param translation                                                                    |
| -------------- | -------------------------- | ------------------------------------------------------------------------------------ |
| `clock`        | `widgets-wasm/clock`       | style/booleans pass through, font vocabulary remap, `timezone` → `timezone_override` |
| `block_height` | `widgets-wasm/blockheight` | `show_timestamp` pass-through, font vocabulary remap                                 |
| `remote_image` | `widgets-wasm/image`       | `refresh_duration` (humantime) → `refresh_seconds`, `image_scale_mode` → `sizing`    |

Legacy `remote_widget` entries, dispatched by URL slug under `https://widgets.braiinsforge.com/<slug>`:

| Slug            | Target manifest              | Param translation                                          |
| --------------- | ---------------------------- | ---------------------------------------------------------- |
| `weather`       | `widgets-wasm/weather`       | `location` pass-through, `time_zone` pinned to default     |
| `nameday`       | `widgets-wasm/nameday`       | `country` enum-guarded, camelCase `showDate` → `show_date` |
| `iss-position`  | `widgets-wasm/iss-position`  | none (manifest has no params)                              |
| `random-facts`  | `widgets-wasm/random-facts`  | none (manifest has no params)                              |
| `spacex-launch` | `widgets-wasm/spacex-launch` | none (legacy `showSeconds` drops with the mapping)         |

Widgets with **no native counterpart drop**: native kinds `ticker_btc`, `braiins_pool`, `blockchain_data`,
`halving_countdown`; remote slugs `exchange-rate`, `formula-1`, `nasa-picture-of-the-day`, `ticker-list`,
`ticker-single-candlestick`, `ticker-single-sparkline`. (`braiins_pool` is *not* mapped to `mining-info` — the new
widget reads a LAN miner device via `miner_url`, a fundamentally different data source than a pool account.)

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

## Settings and accounts pass-through

The top-level settings kept their shape across the schema change: `scene_cycling`, `localization`, `data_collection`,
`brightness_pct`, `night_mode`, `sound_volume_pct`, `alarms`, `led_enabled`, `boot_sound_enabled`, `autoupgrade`. Each
is carried as raw JSON in `v0::Config` and re-parsed into its current typed form during the upgrade
(`passthrough_setting`) — a validate step, not a transformation. A value that fails the re-parse is dropped with a
`warn!` naming the field; a single bad setting never fails the migration, and the dropped field falls back to the same
default a field-less current config would use. Accounts go through the same lenient per-entry re-parse
(`deserialize_accounts_passthrough`).

## Safety

- **Backup first.** `<path>.backup.<unix_secs>` is written before the new config, by `save_with_backup`. Plain integer
  suffix sorts naturally, no locale surprises.
- **Atomic write** via `crate::utils::replace_file` (tmp + rename).
- **Total parse failure** surfaces as an explicit `anyhow::Error` from `LoadedConfig::from_str`; the existing file is
  left untouched (no partial rewrite).

## Boot integration

`ConfigHandle::init` runs `migrate_on_disk` on the runtime config path before anything else reads it, so the upgrade
happens once on first boot with no user action. Success (whether an already-current no-op or an applied v0 upgrade)
yields the current `Config`, which is then validated.

On failure the boot path distinguishes two cases:

- **Newer-than-known version.** If the on-disk file declares a schema version greater than `CONFIG_VERSION` (a
  `peek_version` check), it is left untouched — the firmware boots on in-memory defaults and logs how to recover. This
  is the "safe downgrade refusal" story: never clobber a config a newer firmware wrote.
- **Corrupt / missing.** Anything else (unreadable, invalid JSON) is backed up to `<path>.bcp` and replaced with a
  platform default, as before.

> Follow-up: the refusal path currently still boots the rest of the device on defaults rather than failing only the
> display subsystem while keeping the web UI reachable. Splitting boot so the display alone fails on an unreadable
> config is filed as a follow-up.

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

## Open items / follow-ups

- **Cap kept backups.** Keep at most N (≈10) `config.json.backup.<ts>` files, rotating the oldest out. Filed as
  technical debt.

## Testing

- **Unit:** per-widget upgrade tests (native kinds map to their manifest UID with every required param filled, param
  translations and enum guards, remote slugs with a native equivalent map and the rest drop, unknown kinds / URLs drop),
  plus header-dispatch edge cases (missing version, current version, unknown future version, empty v0 input).
- **Integration:** round-trips a captured device config through `migrate_on_disk` and asserts: version header, scene
  count, every post-upgrade widget carries a reserved UID, no placeholder shapes leak, backup file exists, `translated`
  counter matches the on-disk widget count. Plus a test for the pure `LoadedConfig::from_str` path verifying
  `original_v0()` survives the upgrade.
- **CLI smoke:** `bmc-migrate-config <src> <dst>` exits 0 and emits a counts report.
- **Boot path:** `ConfigHandle::init` migrating a seeded legacy config in place (version bumped, scenes preserved,
  backup written), and refusing to overwrite a newer-than-known config.

## Files

- `bmc/src/config.rs` — `version` field, `CONFIG_VERSION` constant, `Config::from_migrated_parts`, and the
  `ConfigHandle::init` boot wiring (`load_migrate_validate` / `recover_from_failed_load`).
- `bmc/src/config_migration.rs` — `Version`/`Upgrade` traits, `LoadedConfig`, `FromStr`, `load_any_version`,
  `save_with_backup`, `migrate_on_disk`, `peek_version`, `Report`.
- `bmc/src/config_migration/v0.rs` — deserialize-only v0 types + `impl Version`.
- `bmc/src/config_migration/upgrade_v0.rs` — manifest UID constants, remote-widget slug dispatch, per-widget translators
  (required-param filling, enum guards), and `params_from_value` (legacy JSON params → typed param map).
- `bmc/src/widget/coordinator.rs` — defensive nil-UID skip (the migration no longer produces nil UIDs; the guard is
  harmless).
- `bmc/src/bin/migrate_config.rs` — offline CLI.
- `bmc/tests/config_migration.rs` — integration tests.
- `bmc/tests/fixtures/legacy_config_sample.json` — captured sample.
