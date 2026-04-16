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
- Where a legacy widget has a matching new widget today (the digital
  clock), it keeps working with the same settings.

### No silent data loss

> As a user, I want to be sure that nothing important is thrown away
> if the new firmware can't run an old widget yet.

- The original config is copied to
  `/etc/bmc_config.json.backup.<timestamp>` before any change. The
  backup is never overwritten or deleted by the migration.
- If a widget can't be translated yet (because its WASM replacement
  hasn't shipped), it leaves a placeholder in the same grid cell.
  The placeholder carries the original widget's data so a future
  firmware can restore it without asking the user to re-enter
  anything.
- A placeholder widget renders nothing on the display — the cell is
  visibly empty rather than showing a stale widget.

### Safe downgrade refusal

> As a user, I want to be told when something is wrong rather than
> have my config silently overwritten.

- If the on-disk config carries a schema version the firmware
  doesn't know how to read (e.g. accidentally booted older firmware
  on top of a newer config), the migration refuses to touch the
  file and the display subsystem fails to start with a clear log
  message. Other device subsystems (web UI, network) stay
  reachable so the user can recover over the network.

## Behaviour at boot

| Config version on disk | What happens                                 |
|------------------------|----------------------------------------------|
| missing or `0` (legacy)| backup → translate → write new config        |
| `1` (current)          | no-op                                        |
| anything else          | error, do not overwrite                      |

## Tools

- `bmc-migrate-config <src> <dst>` runs the translator offline
  against a captured device config — useful for QA and CI without
  flashing firmware.
