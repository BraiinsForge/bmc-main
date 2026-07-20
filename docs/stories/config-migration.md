# Config Migration on BMC Application Upgrade

The device config carries a schema version. Whenever a BMC Application upgrade ships a newer config schema, the config
already on disk is migrated up to it automatically — once, on first start of the new version, with no prompt and no
manual step. The user never has to know that the schema changed, or that a file moved.

Migration is a chain of one-hop upgrade steps: each schema version knows how to read the version directly below it and
produce the next one. A config that is several versions behind is walked up one step at a time until it matches the
version the running BMC Application expects. Today the chain is a single hop — from the legacy slint-monolith config
(schema version `0`) to the first manifest-driven widget schema (version `1`) — but every later schema bump adds one
more step to the same chain, and the guarantees below (automatic, backed up, validated before write, downgrade-safe)
hold for every future upgrade the same way.

The one-time move into the `/etc/bmc/` directory is part of this first migration: the config file is copied from the
legacy `/etc/bmc_config.json` to `/etc/bmc/config.json` (and future backups, `config.json.backup.<timestamp>`, live next
to it in the new directory). The legacy file is left intact so a forced boot into the legacy slint-monolith can still
find its config. The directory-based layout lets OpenWRT's `sysupgrade` preserve every file under `/etc/bmc/` as a
conffile across firmware updates without needing an entry-per-file.

## User stories

### Transparent upgrade

> As a user, I want my scenes and widget layouts to survive a BMC Application upgrade without having to re-create them.

- The migration runs once on first boot of the new BMC Application. No prompt, no manual step.
- Device settings survive too: alarms, night mode, brightness, sound volume, localization, scene cycling, the LED and
  boot-sound switches, and auto-upgrade preferences all carry over unchanged.
- Scene IDs, widget positions, and widget sizes are preserved. The grid the user built still looks the same.
- Each upgrade step translates the widgets it has an equivalent for and drops the rest. In the current v0 → v1 step that
  means the clock, block height, and image widgets keep their settings, plus the Braiins Forge remote widgets that now
  have a WASM equivalent — weather, nameday, ISS position, random facts, and SpaceX launch (matched by their URL to the
  WASM widget's ID). Their positions and user-configured settings carry over and they work immediately.
- Any widget the step has no equivalent for is dropped, with a `warn!` line naming the unsupported kind or URL. For v0 →
  v1 this includes the legacy ticker, Braiins Pool, blockchain-data, and halving-countdown widgets, and the remote
  exchange-rate, Formula 1, NASA picture of the day, and ticker widgets — none of which have a WASM counterpart yet.
  Dropped widgets are not preserved as empty placeholders — see "Recovering from a bad migration" below.

### Backup before any change

> As a user, I want a copy of the original config kept around in case something goes wrong with the upgrade.

- The original config is copied to `/etc/bmc/config.json.backup.<timestamp>` before any change. The backup is never
  overwritten or deleted by the migration.
- The upgraded config is validated in memory before it is written, so a migration that would produce an invalid config
  is rejected without touching the file — the readable original is left in place rather than replaced by a broken
  result.

### Safe downgrade refusal

> As a user, I want to be told when something is wrong rather than have my config silently overwritten.

- If the on-disk config carries a schema version the BMC Application doesn't know how to read (e.g. accidentally booted
  an older BMC Application on top of a newer config), the BMC Application refuses to touch the file. It boots on default
  settings and logs how to recover, and the newer config on disk is never overwritten — not at boot, and not when a
  setting is changed afterwards (those changes apply for the session but are not written back). Roll the BMC Application
  forward, or restore a backup, to get the saved config back.

### Recovering from a bad migration

> As a user, I want a way back if the migration loses a widget I cared about or the new config misbehaves.

- Every migration leaves a timestamped backup. To restore, SSH into the device, copy the most recent
  `/etc/bmc/config.json.backup.<timestamp>` over `/etc/bmc/config.json`, and reboot. The device will re-migrate the
  restored file on the next boot; the backup of that rerun becomes the next snapshot.
- If a widget you expected to survive the upgrade is missing from your scene after migration, its old `kind` (or
  `widget_url` for remote widgets) was not in the current BMC Application's migration catalog. The system log records a
  `warn!` line for each dropped widget, naming the unsupported kind or URL. Either your BMC Application is older than
  the widget, or the widget comes from a custom deckfeeder the stock BMC Application doesn't know about.

## Behaviour at boot

| Config version on disk                                | What happens                                                  |
| ----------------------------------------------------- | ------------------------------------------------------------- |
| older than the running schema (today: missing or `0`) | backup → upgrade through the chain in memory → write new file |
| equal to the running schema (today: `1`)              | no-op                                                         |
| newer than the running schema                         | error, do not overwrite (downgrade refusal)                   |

## Tools

- `bmc-migrate-config <src> <dst>` runs the upgrade offline against a captured device config — useful for QA and CI
  without using the real device. Emits scene and widget counts (kept vs dropped) so a coverage regression is easy to
  spot in CI logs.
