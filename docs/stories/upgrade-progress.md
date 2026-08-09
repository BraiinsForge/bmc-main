# Upgrade Progress

On-device feedback for firmware and package upgrades. A firmware upgrade takes over the screen with a full-screen
progress overlay; a package-only upgrade shows a small corner card while the widgets keep running. Both end in a clear
success or failure screen, including after the restart that finishes an upgrade.

## User stories

### See a firmware upgrade happening on the device

> As a user, I want the device itself to show that a firmware upgrade is running and how far along it is, so that I
> don't need the web UI to know what the device is doing.

- The moment a firmware upgrade starts, a full-screen overlay covers the scene and stays until the upgrade ends.
- The overlay names the current stage — downloading, verifying, or applying firmware; a combined run also shows its
  package stages.
- While downloading, a progress bar and a downloaded-of-total megabyte readout advance at least once per second, without
  flicker; before the first stage arrives the overlay shows "Preparing update".
- When the total download size is unknown, the overlay shows activity without inventing a percentage.
- The overlay is deliberately modal: the screen cannot be interacted with while the device is being upgraded.

### Keep using the device during a package upgrade

> As a user, I want package updates to stay out of my way, so that the clock remains usable while they install.

- A package-only upgrade shows a small card in the bottom-right corner naming the current stage — downloading,
  verifying, building, or activating packages.
- The card takes no input; touches go to the scene as usual and widgets stay live throughout.

### Recognize success and failure

> As a user, I want a clear final screen when an upgrade ends, so that I never mistake a failed upgrade for a running or
> successful one.

- A failure replaces any progress display immediately with a recognizable failure screen; it stays up for ten seconds
  and then returns the device to normal.
- A successful upgrade shows "Update Finished" for ten seconds in the same placement the upgrade used — full screen for
  firmware, corner card for packages.
- Touching the full-screen "Update Finished" screen dismisses it immediately; failure and running states cannot be
  dismissed by touch.

### Learn that the upgrade finished after the device restarts

> As a user, I want the device to confirm the upgrade after it reboots, so that I know the restart I watched was the
> upgrade completing.

- After the reboot that applies a firmware upgrade, the device shows the full-screen success screen once it is up again.
- A package upgrade that restarts the display as part of activation reports its success the same way after the restart,
  in the corner card.
- The confirmation appears only on an operational device — a device in setup or factory-default state skips it.

### No redundant startup screens after an upgrade

> As a user, I want the device to come back from an upgrade restart without a parade of screens, so that the
> confirmation is the one thing I see.

- After a package-upgrade restart the startup connection screen is skipped entirely — the network never dropped, so
  there is nothing to report.
- After a firmware upgrade the startup connection screen waits until the success screen has finished, then runs with its
  full display time.

## Constraints

- The on-device overlay is display-only: upgrades are started, configured, and cancelled from the web UI, never from the
  device.
- The device never shows internal error details; failures are recognizable but generic, and diagnostics belong to the
  web UI.
- The overlay projects the same upgrade state the web UI shows; it never infers progress from anything else.
- Firmware-containing runs always use the modal full-screen presentation; package-only runs never block interaction.
