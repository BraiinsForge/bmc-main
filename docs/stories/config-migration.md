# Config Migration on Firmware Upgrade

When a device upgrades from the slint-monolith firmware to the
manifest-driven widget system, its existing `/etc/bmc_config.json`
is converted automatically. The user does not need to know that the
schema changed.

## User stories

### Transparent upgrade

> As a user, I want my scenes and widget layouts to survive a
> firmware upgrade without having to re-create them.

- The migration runs once on first boot of the new firmware. No
  prompt, no manual step.
- Scene IDs, scene names, widget positions, and widget sizes are
  preserved exactly. The grid the user built still looks the same.
- Widgets known to this firmware keep their settings. Where the new
  firmware has a matching widget that's already shipped (the
  digital clock today), the widget keeps working immediately with
  the same settings.
- Widgets whose new firmware implementation is still in preparation
  (the rest) keep their slot in the scene layout and their user-
  configured params; the cell stays empty on screen until the
  implementation lands, after which the widget starts working
  without further user action.

### Backup before any change

> As a user, I want a copy of the original config kept around in
> case something goes wrong with the upgrade.

- The original config is copied to
  `/etc/bmc_config.json.backup.<timestamp>` before any change. The
  backup is never overwritten or deleted by the migration.
- If a migration pass produces an unreadable result, the original
  is still on disk next to the rewritten file.

### Safe downgrade refusal

> As a user, I want to be told when something is wrong rather than
> have my config silently overwritten.

- If the on-disk config carries a schema version the firmware
  doesn't know how to read (e.g. accidentally booted older firmware
  on top of a newer config), the migration refuses to touch the
  file and the display subsystem fails to start with a clear log
  message. Other device subsystems (web UI, network) stay
  reachable so the user can recover over the network.

### Recovering from a bad migration

> As a user, I want a way back if the migration loses a widget I
> cared about or the new config misbehaves.

- Every migration leaves a timestamped backup. To restore, SSH into
  the device, copy the most recent
  `/etc/bmc_config.json.backup.<timestamp>` over
  `/etc/bmc_config.json`, and reboot. The device will re-migrate
  the restored file on the next boot; the backup of that rerun
  becomes the next snapshot.
- If a widget you expected to survive the upgrade is missing from
  your scene after migration, its old `kind` (or `widget_url` for
  remote widgets) was not in the current firmware's migration
  catalog. The system log records a `warn!` line for each dropped
  widget, naming the unsupported kind or URL. Either your firmware
  is older than the widget, or the widget comes from a custom
  deckfeeder the stock firmware doesn't know about.

## Behaviour at boot

| Config version on disk | What happens                                |
|------------------------|---------------------------------------------|
| missing or `0` (legacy)| backup → upgrade in memory → write new file |
| `1` (current)          | no-op                                       |
| anything else          | error, do not overwrite                     |

## Tools

- `bmc-migrate-config <src> <dst>` runs the upgrade offline against
  a captured device config — useful for QA and CI without flashing
  firmware. Emits `scenes / translated / dropped` counts so a
  coverage regression is easy to spot in CI logs.
