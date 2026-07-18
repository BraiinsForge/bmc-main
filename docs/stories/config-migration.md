# Config Migration on Firmware Upgrade

When a device upgrades from the slint-monolith firmware to the manifest-driven widget system, its existing config is
converted automatically. The user does not need to know that the schema changed, or that the file moved.

On first boot of the new firmware the config file is copied from the legacy `/etc/bmc_config.json` to
`/etc/bmc/config.json` (and future backups, `config.json.backup.<timestamp>`, live next to it in the new directory). The
legacy file is left intact so a forced boot into the older firmware can still find its config. The directory-based
layout lets OpenWRT's `sysupgrade` preserve every file under `/etc/bmc/` as a conffile across firmware updates without
needing an entry-per-file.

## User stories

### Transparent upgrade

> As a user, I want my scenes and widget layouts to survive a firmware upgrade without having to re-create them.

- The migration runs once on first boot of the new firmware. No prompt, no manual step.
- Device settings survive too: alarms, night mode, brightness, sound volume, localization, scene cycling, the LED and
  boot-sound switches, and auto-upgrade preferences all carry over unchanged.
- Scene IDs, scene names, widget positions, and widget sizes are preserved exactly. The grid the user built still looks
  the same.
- Widgets this firmware knows how to translate keep their settings: the clock, block height, and image widgets, plus the
  Braiins Forge remote widgets that now have a native equivalent — weather, nameday, ISS position, random facts, and
  SpaceX launch (matched by their URL to the native widget's ID). Their positions and user-configured settings carry
  over and they work immediately.
- Any other widget is dropped, with a `warn!` line naming the unsupported kind or URL. This includes the legacy ticker,
  Braiins Pool, blockchain-data, and halving-countdown widgets, and the remote exchange-rate, Formula 1, NASA picture of
  the day, and ticker widgets — none of which have a native counterpart yet. Dropped widgets are not preserved as empty
  placeholders — see "Recovering from a bad migration" below.

### Backup before any change

> As a user, I want a copy of the original config kept around in case something goes wrong with the upgrade.

- The original config is copied to `/etc/bmc/config.json.backup.<timestamp>` before any change. The backup is never
  overwritten or deleted by the migration.
- If a migration pass produces an unreadable result, the original is still on disk next to the rewritten file.

### Safe downgrade refusal

> As a user, I want to be told when something is wrong rather than have my config silently overwritten.

- If the on-disk config carries a schema version the firmware doesn't know how to read (e.g. accidentally booted older
  firmware on top of a newer config), the migration refuses to touch the file and the display subsystem fails to start
  with a clear log message. Other device subsystems (web UI, network) stay reachable so the user can recover over the
  network.

### Recovering from a bad migration

> As a user, I want a way back if the migration loses a widget I cared about or the new config misbehaves.

- Every migration leaves a timestamped backup. To restore, SSH into the device, copy the most recent
  `/etc/bmc/config.json.backup.<timestamp>` over `/etc/bmc/config.json`, and reboot. The device will re-migrate the
  restored file on the next boot; the backup of that rerun becomes the next snapshot.
- If a widget you expected to survive the upgrade is missing from your scene after migration, its old `kind` (or
  `widget_url` for remote widgets) was not in the current firmware's migration catalog. The system log records a `warn!`
  line for each dropped widget, naming the unsupported kind or URL. Either your firmware is older than the widget, or
  the widget comes from a custom deckfeeder the stock firmware doesn't know about.

## Behaviour at boot

| Config version on disk  | What happens                                |
| ----------------------- | ------------------------------------------- |
| missing or `0` (legacy) | backup → upgrade in memory → write new file |
| `1` (current)           | no-op                                       |
| anything else           | error, do not overwrite                     |

## Tools

- `bmc-migrate-config <src> <dst>` runs the upgrade offline against a captured device config — useful for QA and CI
  without flashing firmware. Emits scene and widget counts (kept vs dropped) so a coverage regression is easy to spot in
  CI logs.
