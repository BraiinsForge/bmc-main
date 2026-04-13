# BDK-346: Config Migrations (PoC)

## Goal

When a device running the old slint-monolith firmware upgrades to the new
manifest-driven widget system, its existing `/etc/bmc_config.json` must be
automatically converted to the new schema. No user action required;
scene layouts, widget positions, and params are preserved where a
matching manifest exists, and anything untranslatable is retained as a
placeholder so a later migration pass can promote it.

## Status

**PoC.** Scope limited to the one widget we can translate today
(`clock` + `clock_style: "digital"` → `digital-clock` manifest). Every
other old widget kind lands as an "unavailable" placeholder. As new
manifests are written, add entries to the translation registry and
they'll migrate automatically.

## Non-goals

- Schema-versioning the new config (future improvement).
- Migrating alarm / account / global settings (only `scenes` are
  structurally different; other fields pass through or have no old-side
  equivalent to migrate).
- Rollback. The backup is the only escape hatch; we do not keep the
  device dual-booted between formats.

## Architecture

```
bmc::config_migration
├── legacy::*      — serde-Deserialize structs for the old format
├── translate::*   — pure fns: old kind+params → MigrationOutcome
└── migrate()      — orchestrator: detect → backup → translate → write
```

Entry points:

1. `bmc::config_migration::migrate_in_place(path)` — called from
   `ConfigHandle::init()` before load. Idempotent: if the file is
   already in the new format, the function is a no-op.
2. `bmc-migrate-config <src> <dst>` — a new `[[bin]]` in the `bmc`
   crate (or `bmc-openwrt`, TBD). Reads `<src>`, writes `<dst>`, never
   touches `/etc/`. Lets reviewers and CI exercise the translator
   against captured samples without flashing a device.

## Detection

Shape-based. `migrate_in_place` reads the file, tries
`serde_json::from_str::<NewConfig>(…)`. If it succeeds, config is
already new — return immediately. If it fails, tries
`serde_json::from_str::<LegacyConfig>(…)`. If that succeeds, it's old
— migrate. If both fail, the file is malformed; fall back per the
error-handling section.

No schema version field is added to the new `Config`; we may add one
later if we do a second migration.

## Translation registry

A function table keyed by the old `kind` string:

```rust
fn translate_widget(legacy: &LegacyWidget) -> MigrationOutcome {
    match legacy.kind.as_str() {
        "clock" => translate_clock(legacy),
        // ticker_btc, block_height, etc. -> Unavailable for now
        _ => MigrationOutcome::Unavailable(legacy.snapshot()),
    }
}
```

Each translator is a pure function — easy to unit-test in isolation
with real param fixtures from the captured device sample.

### `translate_clock`

- `clock_style == "digital"` → `digital-clock` manifest
  (`550e8400-e29b-41d4-a716-446655440001`).
  - `show_seconds: bool` → `showSeconds: bool`
  - `show_timezone: bool` → `showTimezone: bool`
  - `numbers_font_style: "light"|"medium"|"bold"` → `fontStyle` (direct)
  - `show_date` → dropped (new digital-clock has no date; logged at
    `warn` level)
- `clock_style != "digital"` → `Unavailable` (analog_rect, analog_round,
  etc. have no matching manifest yet).

## Unavailable placeholder

A widget marked "unavailable" is written to the new config with:

- `widget_type_id: Uuid::nil()` (sentinel `00000000-…`).
- `params: { "_legacy": { "kind": "...", "params": { ... } } }` —
  original data preserved verbatim.
- `position` and `size` kept as-is.

The compositor sees `Uuid::nil()` and must not spawn a widget process
for it (renders nothing). The frontend scene editor can eventually
detect the sentinel and show a "widget no longer available" card.
Neither of those touches is in this PoC's scope.

## Data flow

```
┌─────────────────────────┐
│ /etc/bmc_config.json    │  (old shape on disk)
└───────────┬─────────────┘
            │
            ▼
  ┌──────────────────┐     read + try-new → fail
  │ migrate_in_place │     try-legacy → ok
  └────────┬─────────┘
           │
           ▼
  ┌─────────────────────────────────────┐
  │ /etc/bmc_config.json.backup.<unix>  │  copy, never moved
  └─────────────────────────────────────┘
           │
           ▼
  ┌──────────────────┐
  │  translate(cfg)  │  per-scene, per-widget
  └────────┬─────────┘
           │
           ▼  (NewConfig value)
  ┌─────────────────────────┐
  │ utils::replace_file     │  atomic rename: .tmp → /etc/bmc_config.json
  └─────────────────────────┘
```

## Error handling

**Total failure** (neither new nor legacy shape parses): log error,
keep the original file untouched on disk, write a default `NewConfig`
to `/etc/bmc_config.json`. Device boots functional but with empty
scenes; the unparseable original remains on disk (also copied to
`.backup.<ts>` so multiple attempts don't overwrite each other). User
can ship the file to support for forensic recovery.

**Partial failure** (old file parses, but some scenes or widgets
error during translation): the failing scene or widget lands as a
placeholder (`Uuid::nil()` with `_legacy` payload for widgets; empty
scene with `_legacy_error` annotation for scenes). Migration continues;
the final write succeeds.

All errors logged at `warn` or `error` with the affected scene/widget
ID and the original kind.

## Backup filename

`{original_path}.backup.{unix_seconds}` — e.g.
`/etc/bmc_config.json.backup.1745002245`. Plain integer so sorting is
trivial and no locale/timezone surprises.

## Testing

- **Unit**: per-translator tests with inline JSON fixtures covering
  each branch (digital clock with full params; digital clock with
  missing optional params; non-digital clock → unavailable; unknown
  kind → unavailable).
- **Integration**: round-trip test that reads
  `bmc/tests/fixtures/legacy_config_sample.json` (the file we pulled
  from the live device), runs the migrator, and asserts the output
  deserializes as a valid new `Config` with the expected placeholder
  count and the one translated widget.
- **CLI smoke**: the `bmc-migrate-config` binary runs against the
  fixture, exits 0, and produces the same output as the integration
  test.
