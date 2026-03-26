# Nix Store Initializer

The Nix store initializer (`bmc-nix-init`) is a last-resort recovery component
that runs on every boot.  When the device has no initialized Nix store — most
commonly after a factory reset — it takes over the display and guides the user
through WiFi setup, store download, and profile activation.  Because it runs
when the rest of the system is absent, reliability is paramount: it must work.

## When it runs

The initializer is an OpenWrt init service (`bmc-nix-init`) that
starts on every boot, in parallel with the main BMC compositor. It is
shipped with the firmware itself and has no dependency on
`/nix/store`.

On most boots the initializer exits immediately — it checks that a profile has
been activated and that the store exists.  The user never sees it.

It takes action only when the store is missing or no profile is activated.

## User stories

### Factory reset recovery

> As a user, I want my device to recover automatically after a factory reset so
> that I do not need to manually flash firmware or connect a serial console.

- After factory reset the `nix_init` U-Boot variable is cleared.  A separate
  `nix-factory-reset` service (which runs earlier in boot) detects this and
  wipes `/mnt/data/nix`.
- The initializer sees that no profile is activated, takes over the framebuffer,
  and begins the recovery flow.
- A development escape hatch exists: placing `/mnt/data/NIX_INHIBIT_INIT`
  prevents the initializer from running, so developers can manage the store
  manually.

### WiFi setup

> As a user, I want to connect my device to WiFi during recovery so that it can
> download the software it needs — even if I have never configured WiFi on this
> device before.

- The initializer first waits up to 30 seconds for an existing WiFi connection
  (in case credentials survived the reset or Ethernet is available).
- If no connection is established, the device creates an open WiFi access point
  named **Braiins Deck XXX** (where XXX are the last three characters of the
  MAC address).
- A captive portal redirects common domains to the device IP (`10.0.0.21`).
- The display shows the AP name, a QR code pointing to the setup page, and
  instructions to connect.
- A web UI (the standard frontend served over HTTP on port 80) lets the user
  scan for networks and enter credentials.
- Once the user submits credentials, the device switches to station mode and
  attempts to connect.  The display updates to show connection progress.

### Store download

> As a user, I want the device to download and install its software
> automatically once internet is available, showing me clear progress.

- Once online, the initializer fetches a factory index from the configured
  server (default: Braiins cache) to find a Nix store tarball matching the
  current BOS firmware version.
- Download progress is shown on screen (e.g. "Downloading: 150.5 / 500.0 MB")
  with a progress bar.
- The tarball is extracted to `/nix/store` and the latest profile generation is
  activated.
- TLS certificate validation is intentionally disabled because the system clock
  is not yet synchronized (no NTP before network setup).  Integrity is ensured
  by cryptographic signature verification of the downloaded tarball.
- After successful activation the initializer writes the `nix_init` U-Boot
  variable and signals the BMC daemon to perform first-time setup (timezone,
  password).

### Firmware upgrade fallback

> As a user, I want my device to upgrade its firmware if the current version
> does not have a matching software bundle, so that recovery still succeeds.

- If no tarball exists for the current BOS version, the initializer looks for a
  firmware upgrade path.
- If an upgrade is available, it downloads the sysupgrade image (with progress
  shown on screen) and initiates a firmware upgrade.
- The device reboots into the new firmware version, and the initializer runs
  again — this time finding a matching tarball.
- If no upgrade path exists either, an error is shown to the user.

### Error handling and retry

> As a user, I want to be able to retry or reconfigure WiFi when something goes
> wrong, without having to power-cycle the device.

- Network errors (download failure, WiFi dropout, server unreachable) show an
  error screen with two buttons: **Retry download** and **Reconfigure WiFi**.
- "Retry" re-attempts the download without re-entering AP mode.
- "Reconfigure WiFi" brings back the access point so the user can enter
  different credentials.
- On retry, partially downloaded store data is preserved to avoid re-downloading
  from scratch.
- Non-network errors (disk failure, corrupt config) show the error detail and a
  retry button, but no WiFi reconfigure option — since WiFi is not the problem.

## Display

The initializer renders directly to the framebuffer using Slint with a software
renderer.  It does not depend on any compositor or window manager.

Two screen layouts exist:

1. **WiFi setup** — split layout with AP name and instructions on the left, QR
   code on the right.
2. **Status / progress** — centered layout with a status message, optional
   progress bar, and optional error detail with action buttons.

The display is only activated when initialization is actually needed.  On normal
boots (store already present), no UI is shown.

## Servers configuration

The initializer resolves its download server in order of preference:

1. `/etc/nix-upgrade/servers.json` — user or system override.
2. `/etc/nix-upgrade/servers.json.default` — shipped with firmware.
3. Compiled-in default — the Braiins cache URL embedded in the binary.

Invalid config files are backed up to `.bcp` and the next source is tried.

## Platform variants

| Aspect              | OpenWrt (`bmc-nix-init-openwrt`)      | Mock (`bmc-nix-init-mock`)         |
|---------------------|---------------------------------------|------------------------------------|
| Display             | Linux DRM + evdev touch               | minifb virtual window (X11)        |
| WiFi                | Real `OpenwrtWifiManager`, captive portal | Simulated (fake networks, `--no-wifi` flag) |
| Store mount         | Bind-mount `/mnt/data/nix` → `/nix`  | Temporary directory                |
| Profile activation  | Real activation scripts               | No-op                              |
| Init marker         | `fw_setenv nix_init 1`               | No-op                              |
| Firmware upgrade    | Real `sysupgrade` + reboot            | No-op                              |
