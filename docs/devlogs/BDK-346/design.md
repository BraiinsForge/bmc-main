# BDK-346: Config Migration

## Goal

Convert the BMC config (legacy `/etc/bmc_config.json`, new
`/etc/bmc/config.json`) from the slint-monolith shape to the
manifest-driven shape on first boot of the new firmware, with no user
action. Scene layouts and per-widget data for widgets we recognise
survive the upgrade; widgets we do not recognise are dropped with a
warning rather than preserved in a placeholder shape.

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
   whether, when, and how to persist.
2. **The original parse survives.** `LoadedConfig::original_v0()`
   gives callers (debug endpoints, rollback UIs, CI snapshot tests)
   the parsed `v0::Config` without re-reading disk. After persistence
   the on-disk file is already rewritten; boser-style keeps the
   original value live in memory.
3. **Adding v2 is mechanical.** Define `ConfigV2`, `impl Upgrade for
   ConfigV1 { type NextVersion = ConfigV2; … }`, extend the `FromStr`
   match arm.

`Config` carries a top-level `version: u32` field (`#[serde(default)]`,
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

## Per-widget upgrade policy

The upgrade is a **total function from known v0 widgets to current
widgets**. Every v0 widget either maps to a reserved
`widget_type_id` (with a real UUID) or drops out of the upgraded
config entirely. There is no intermediate "placeholder" shape —
the review conclusion was that users migrate all their widgets at
once, so an unmappable widget is an edge case, not a state to
preserve.

Two flavours of reserved UID:

- **Sequential UIDs for native widget kinds** (`clock`,
  `ticker_btc`, `block_height`, …). These continue the
  `550e8400-e29b-41d4-a716-44665544000N` pattern already used by
  shipped manifests (digital-clock = `…0001`, flip-clock = `…0002`).
- **Deterministic UUID v5 for Braiinsforge remote widgets**,
  derived from the URL slug under a dedicated namespace. Adding a
  new remote widget to the catalog is one line in
  `REMOTE_WIDGET_SLUGS`; the UUID falls out of the derivation.

### Reserved native UIDs

Continuing the manifest numbering convention. Widgets already
shipped use the UID from their `manifest.json`; reserved dummies
are allocated in advance so the eventual manifest can adopt the
same value without migrating again.

| v0 `kind` (+ variant)         | UID suffix | Status                        |
|-------------------------------|------------|-------------------------------|
| `clock` + `clock_style:digital` | `0001`    | shipped (digital-clock)       |
| flip-clock                    | `0002`    | shipped                       |
| `clock` + `clock_style:analog_round` | `0003` | reserved dummy            |
| `clock` + `clock_style:analog_rect`  | `0004` | reserved dummy            |
| `ticker_btc`                  | `0005`     | reserved dummy                |
| `block_height`                | `0006`     | reserved dummy                |
| `braiins_pool`                | `0007`     | reserved dummy                |
| `remote_image`                | `0008`     | reserved dummy                |
| `blockchain_data`             | `0009`     | reserved dummy                |
| `halving_countdown`           | `000a`     | reserved dummy                |

### Reserved Braiinsforge remote UIDs

URL slug under `https://widgets.braiinsforge.com/<slug>`. UID is
`Uuid::new_v5(&BRAIINSFORGE_WIDGETS_NS, slug.as_bytes())`.

| Slug                           |
|--------------------------------|
| `exchange-rate`                |
| `formula-1`                    |
| `iss-position`                 |
| `nameday`                      |
| `nasa-picture-of-the-day`      |
| `random-facts`                 |
| `spacex-launch`                |
| `ticker-list`                  |
| `ticker-single-candlestick`    |
| `ticker-single-sparkline`      |
| `weather`                      |

Any remote widget outside this catalog (third-party URL, or a
future Braiinsforge slug we haven't added yet) is dropped with a
`warn!` that includes the URL.

### Deep vs shallow translation

The param translators are split by whether the target manifest is
already shipped:

- **Deep** (rewrite param names to the new manifest's schema) —
  used today only for `clock + clock_style:digital`, which maps
  to the shipped digital-clock manifest (`show_seconds → showSeconds`,
  `show_timezone → showTimezone`, `numbers_font_style → fontStyle`,
  `show_date` dropped with a warning).
- **Shallow** (pass `params` through unchanged) — used for
  everything else. The future widget's manifest is authoritative
  over its own param schema and can migrate internally when it
  loads; we don't guess at schemas that don't exist yet.

Adding a deep translator = ship a manifest widget + add a match arm
next to the reserved UID line. The translator list is a living
index of what's shipped.

### Drop policy (no placeholders)

- Unknown top-level `kind` → drop with `warn!`.
- `clock` with an unknown `clock_style` → drop with `warn!`.
- `remote_widget` missing `widget_url` → drop with `warn!`.
- `remote_widget` with a host outside `widgets.braiinsforge.com` →
  drop with `warn!`.
- `remote_widget` whose slug is not in `REMOTE_WIDGET_SLUGS` →
  drop with `warn!`.

Drops are counted in `Report.dropped_widgets`. The scene itself
always survives; only the individual widget entry disappears.

## Safety

- **Backup first.** `<path>.backup.<unix_secs>` is written before
  the new config, by `save_with_backup`. Plain integer suffix sorts
  naturally, no locale surprises.
- **Atomic write** via `crate::utils::replace_file` (tmp + rename).
- **Total parse failure** surfaces as an explicit `anyhow::Error`
  from `LoadedConfig::from_str`; the existing file is left untouched
  (no partial rewrite).

## Compositor coordination

`Coordinator::spawn_widget` short-circuits with an `info!` log when
`widget.widget_type_id` has no matching entry in the widget
registry. With placeholders gone, nil UIDs cannot leak; a reserved
UID for a not-yet-shipped widget produces a "no registered widget
with this UID" log until that widget lands.

## Config path layout

The runtime config and its timestamped backups live under
`/etc/bmc/`:

```
/etc/bmc/config.json
/etc/bmc/config.json.backup.<unix_secs>
```

This is a directory-based layout so OpenWRT's `sysupgrade` can
preserve everything under `/etc/bmc/` with a single conffile rule
(the sibling change in the bos-main packaging layer adds
`/etc/bmc` to the conffile set).

### Legacy path relocation

On first boot of the new firmware the old file at
`/etc/bmc_config.json` is **copied** (not moved) to
`/etc/bmc/config.json` (see `relocate_legacy_config_if_present`
in `bmc/src/config_migration.rs`). Triggered implicitly by
`load_any_version`:

- new path exists → leave everything alone
- new path missing, legacy present → create `/etc/bmc/`, copy
  the legacy file in, leave the original untouched; the version
  dispatch then runs against the new path as normal
- neither present → fresh install, nothing to relocate

**Copy, not move.** The legacy path is preserved deliberately so
a forced boot into the older firmware (debugging, emergency
rollback) still finds its config at the path it expects. That
snapshot goes stale the moment the new firmware writes an edit,
but the "boot old firmware at pre-upgrade state" fallback stays
available indefinitely at the cost of a few KB of on-disk
redundancy. Aligns with the rest of the migration's "never
silently destroy user data" stance.

The pattern is path-shape based (`<parent>/bmc/<name>` looks
for `<parent>/bmc_<name>`) so tests using tmp dirs exercise
the same code path.

## Open items / follow-ups

- **Cap kept backups.** Keep at most N (≈10)
  `config.json.backup.<ts>` files, rotating the oldest out.
  Filed as technical debt.

## Testing

- **Unit:** per-widget upgrade tests (native kinds, every remote
  slug resolves, deterministic v5 derivation is stable, unknown
  kinds / URLs / slugs drop), plus header-dispatch edge cases
  (missing version, current version, unknown future version, empty
  v0 input).
- **Integration:** round-trips a captured device config through
  `migrate_on_disk` and asserts: version header, scene count,
  every post-upgrade widget carries a reserved UID, no placeholder
  shapes leak, backup file exists, `translated` counter matches the
  on-disk widget count. Plus a test for the pure
  `LoadedConfig::from_str` path verifying `original_v0()` survives
  the upgrade.
- **CLI smoke:** `bmc-migrate-config <src> <dst>` exits 0 and emits
  a counts report.

## Files

- `bmc/src/config.rs` — `version` field, `CONFIG_VERSION` constant.
- `bmc/src/config_migration.rs` — `Version`/`Upgrade` traits,
  `LoadedConfig`, `FromStr`, `load_any_version`, `save_with_backup`,
  `migrate_on_disk`, `Report`.
- `bmc/src/config_migration/v0.rs` — deserialize-only v0 types +
  `impl Version`.
- `bmc/src/config_migration/upgrade_v0.rs` — reserved UID tables,
  remote-widget slug dispatch, per-widget translators.
- `bmc/src/widget/coordinator.rs` — placeholder skip (legacy;
  nil UIDs no longer produced but the guard is harmless).
- `bmc/src/bin/migrate_config.rs` — offline CLI.
- `bmc/tests/config_migration.rs` — integration tests.
- `bmc/tests/fixtures/legacy_config_sample.json` — captured sample.
