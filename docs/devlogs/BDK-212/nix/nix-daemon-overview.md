# Nix Integration — Architectural Overview

This document describes the high-level changes needed to integrate
Nix-based package management into the BMC system. For detailed
file-level changes, see [nix-daemon-details.md](nix-daemon-details.md).

## New Components

### `bmc-nix` Library Crate

A library crate providing all Nix operations as async functions: index
fetching, store initialization, manifest diffing, profile building,
hooks, activation, upgrade orchestration, and garbage collection.

No binary of its own — called by the main app and the initializer.
Described in detail in [bmc-nix.md](bmc-nix.md).

### `bmc-nix-initializer` Binary

A small binary that runs at boot **before** the main `bmc` app. Its
sole purpose is ensuring the Nix store is ready.

Flow:
1. Check if the Nix store is initialized
2. If yes — exit immediately, `bmc` starts normally
3. If no — check for network connectivity
4. If no WiFi — enter AP mode, present WiFi setup (same pattern as
   the main app's initial setup)
5. Fetch the factory index, download the initial store tarball
6. Extract and activate the initial profile
7. Exit, `bmc` starts normally

The initializer uses `bmc-display` with a minimal Slint UI to
communicate progress to the user on the physical screen:

| State | Screen |
|---|---|
| Checking | Nothing on screen yet. Only if init is necessary. |
| No WiFi | AP mode instructions + SSID |
| Connecting | "Connecting to WiFi..." |
| Downloading | Progress bar (downloaded / total MB) |
| Extracting | "Installing packages..." + progress |
| Activating | "Finalizing..." |
| Error | Error message + retry info |

The binary must be statically linked (it runs before Nix packages
exist on disk). It depends on `bmc-nix` for store init logic,
`bmc-display` for the screen, and `ii-net-drv` for WiFi management.

## Changed Workflows

### Upgrade Flow (Web UI + Auto-Upgrade)

Currently the upgrade flow only handles BOS firmware: check index →
download image → verify → sysupgrade → reboot.

With Nix, the upgrade becomes a combined operation. The user sees a
single "Upgrade" action that handles both BOS and Nix packages in the
correct order:

```
1. Fetch Nix package indexes from all servers
2. Check firmware index (existing flow)
3. Present combined result to user:
   - Whether application upgrade is required
   - Whether system upgrade is needed
   - Whether a reboot is required (BOS) or not (Nix-only)
   - Estimate time (based on the data usage necessary)
   - In details, which Nix packages have updates (name, old → new version)
4. User confirms upgrade
5. Build an upgrade plan (includes added/removed packages if requested)
6. Apply the upgrade plan (copy store paths → build profile)
7. If BOS upgrade needed: run sysupgrade FIRST (reboot happens here)
8. Activate new profile
9. Garbage collect old generations
```

Key points from the concept doc:
- BOS upgrade happens **before** profile switch
- Nix-only upgrades don't require reboot
- Upgrading always updates **all** installed packages — no partial upgrades
- Installing new packages is separate from upgrading

### Auto-Upgrade

The auto-upgrade scheduler stays the same (cron-based via
`bmc-scheduler`), but its triggered action now runs the combined
flow above instead of firmware-only.

### Frontend

The upgrade settings section needs to show richer information:
- What's being upgraded (system or application, or both)
- In details, show what packages are being upgraded (right now only 'core' package, and possibly 'nix')
- Multi-phase progress (download → preparation (profile build, should be quick) → upgrade)
- Whether reboot is involved (for system - BOS - only)

Package browsing/installation UI is a future addition, not part of the
initial integration.

### Physical Display

The Slint upgrade screens in `bmc-display` need new states for the
Nix phases (copying packages, building profile, activating). The
existing download/upgrade/success/failure screens remain for the BOS
portion.

### Mock Server

`bmc-mock` needs to simulate the Nix upgrade flow so the frontend can
be developed without real hardware or a real Nix store.
