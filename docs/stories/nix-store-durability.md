# Nix Store & Profile Power-Loss Safety

The device keeps its software — the Nix store and its profile generations — on internal storage. Losing power at any
moment, whether during first-time installation, an upgrade, or a recovery wipe, must never leave the device believing
that corrupt or incomplete software is valid. Anything the device reports as complete is durably on storage first, and
anything that survives a power loss is trustworthy.

## User stories

### Interrupted first-time installation

> As a user, I want an interrupted software installation to restart cleanly on the next boot so that my device never
> runs from a half-installed store.

- Installation reports success only after the installed software is durably written to internal storage.
- If power is lost before installation completes, the next boot sees the device as uninitialized and installation
  restarts from a clean state.
- The device never mistakes a partially installed store for a valid one.

### Interrupted upgrade

> As a user, I want a power loss during an upgrade to leave my device on a working software version so that an upgrade
> can never break the device.

- An upgrade reported as successful stays applied across a power loss.
- If power is lost before the upgrade completes, the device boots the previous working version and the upgrade can be
  retried.
- An upgrade staged to activate on the next boot survives a power loss once it has been reported as staged; a staged
  upgrade that was superseded or already applied cannot reappear and downgrade the device.
- Software downloaded during an upgrade is durably written before it is recorded as available, so a power loss cannot
  leave the device trusting downloads that never reached storage.

### Recovery from corruption

> As a user, I expect to be able to recover my device in case its software storage is corrupted so that corruption is
> never permanent.

- Wiping and reinstalling the device software is the recovery path, and it is itself power-loss safe: an interrupted
  wipe leaves the device cleanly uninitialized, never with a partially deleted store that still looks valid.
- Storage errors encountered while installing, upgrading, or activating software fail the operation visibly instead of
  being hidden behind a success report.
- If activating a new software version fails at boot, the device falls back to its current working version and records
  the failure in the system log.

## Constraints

- Durable-download protection for upgrades applies to devices initialized with this firmware or later;
  already-provisioned devices keep their existing Nix configuration and gain it only after a reinstall.
- The guarantees cover power loss and crashes; they do not repair storage hardware that silently corrupts data at rest.
