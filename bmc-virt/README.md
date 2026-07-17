# bmc-virt

OpenWrt QEMU virtual machine for BMC development. Boots a full OpenWrt system with the BMC application, WiFi simulation,
DRM display, and SPI LED capture — no hardware required.

## Quick start

```bash
./scripts/run.sh --config combined-flip-clocks  # build + boot + deploy + connect
./scripts/display.sh            # open VNC viewer
./scripts/stop.sh               # kill the VM
```

Inside the VM, type `just help` for available commands.

## Architecture

The guest architecture matches the host:

| Host           | Guest   | Acceleration | Display               |
| -------------- | ------- | ------------ | --------------------- |
| x86_64-linux   | x86_64  | KVM          | Native console (egui) |
| aarch64-linux  | aarch64 | KVM          | Native console (egui) |
| aarch64-darwin | aarch64 | HVF          | Native console (egui) |

On macOS, all Linux builds delegate to a custom NixOS builder VM (`darwin-ensure-builder.sh`) that auto-starts when
needed. The builder has `boot.binfmt.emulatedSystems = ["x86_64-linux"]` to run the x86_64 OpenWrt ImageBuilder binary
via emulation.

## Build pipeline

```
openwrtFeedCache   FOD: downloads .ipk packages from OpenWrt feeds (keyed by output hash)
     |
 vmImageBase       Pure sandboxed: assembles rootfs offline using cached packages
     |
customKernel       Custom kernel: OpenWrt patches + DRM/VKMS/SPI (aarch64 or x86_64)
     |
  vmImage          x86_64: swap kernel in boot partition
     |             aarch64: pass-through (direct-boot via QEMU -kernel)
     |
    run            QEMU boots the image, deploys binaries, connects SSH
```

The package download (slow, network) is isolated in a Fixed-Output Derivation keyed by **output hash**, so
overlay/script changes only rebuild `vmImageBase` (fast, offline). The FOD only rebuilds when its hash is updated
(package list or OpenWrt version change).

## OpenWrt version

**OpenWrt 24.10.6** (kernel 6.6.127, GCC 13.3).

Upgraded from 23.05.5 to align the GCC version between the OpenWrt kmod packages (GCC 12.3) and our custom kernel build
(nixpkgs GCC 13.4). On aarch64, the GCC 12/13 ABI mismatch caused fatal kernel panics when loading kmod packages
(`nfnetlink`, `evdev`, `mac80211`). OpenWrt 24.10 uses GCC 13.3 which resolves the mismatch.

## Kernel customizations

The custom kernel is derived at build time from the **stock OpenWrt kernel config** (extracted from the ImageBuilder
tarball) plus a small delta file (`kernel-config-delta.nix`). The resolved config is a build output, never committed.

The delta adds on top of the stock config:

- `CONFIG_PROC_PAGE_MONITOR` — rr time-travel debugger
- `CONFIG_IKCONFIG` + `CONFIG_IKCONFIG_PROC` — `/proc/config.gz`
- `CONFIG_DRM_VKMS` — software DRM for VNC framebuffer capture
- `CONFIG_DRM_VIRTIO_GPU` — QEMU display (with TTM, KMS deps)
- `CONFIG_VIRTIO_INPUT` — touch input from QEMU tablet device
- `CONFIG_SPI_SPIDEV` + `CONFIG_SPI_BMC_VIRT` — fake SPI for LED data capture
- `CONFIG_MAC80211` + `CONFIG_CFG80211` + `CONFIG_MAC80211_HWSIM` — WiFi emulation
- x86_64: disables hardware GPU/NIC/USB/storage drivers not needed in QEMU

### Adding a kernel config option

1. Add a `scripts/config` line to `kernel-config-delta.nix`
2. Rebuild and boot: `./scripts/run.sh`

### Upgrading the kernel version

1. Update `linuxVersion` in `flake.nix` and fix the kernel source `hash` (set to `pkgs.lib.fakeHash`, build, paste)
2. Check `kernel-patches/` for conflicts with the new version
3. Boot and verify — the stock config auto-updates from the new ImageBuilder

### Upgrading OpenWrt

1. Update `openwrtVersion` in `flake.nix`
2. Fix the `imageBuilder` fetch `hash` (set to `pkgs.lib.fakeHash`, build, paste)
3. Fix the `openwrtFeedCache` `outputHash` the same way
4. Fix the `openwrtSrc` `hash` the same way
5. Boot and verify — the stock kernel config is re-extracted automatically from the new ImageBuilder

### Modifying pre-installed packages

The `packageList` in `flake.nix` controls which `.ipk` packages are included in the rootfs. To add or remove packages:

1. Edit the list (prefix with `-` to remove a default package, e.g. `"-luci"`)
2. Set `openwrtFeedCache`'s `outputHash` to `pkgs.lib.fakeHash`
3. Build — the FOD will download the new package set and print the correct hash
4. Paste the hash and rebuild

## WiFi emulation

Two `mac80211_hwsim` radios simulate WiFi without real hardware:

| Radio  | Interface | Role                   | Managed by        |
| ------ | --------- | ---------------------- | ----------------- |
| radio0 | wlan0     | App's radio (AP ↔ STA) | The app           |
| radio1 | wlan1     | Fake upstream AP       | VM infrastructure |

Radio1 runs as a permanent AP (`BMC-VIRT-UPLINK`, WPA2-PSK, password `braiins-virt`) on a separate subnet (10.99.0.0/24)
with DHCP and NAT to eth0. When the app switches radio0 to STA mode and connects, it gets real nl80211 state, real
signal, real DHCP lease, and internet through the WiFi path.

**Boot params:** `mac80211_hwsim.radios=2 mac80211_hwsim.channels=2`

- `radios=2` creates two virtual radios (the app only manages radio0 via syspath `hwsim0`)
- `channels=2` selects hwsim's multi-channel ops which include `hw_scan` — this allows AP-mode interfaces to scan for
  networks, matching real hardware behavior. Without this, hwsim uses software scan which rejects AP-mode scans.
  `kernel-patches/002-mac80211-ap-scan.patch` (ported from OpenWrt's `210-ap_scan.patch`) bypasses the kernel's
  beaconing check that would otherwise block AP-mode scans.

**Dashboard toggle:** Press `w` in the dashboard to connect/disconnect radio0 to BMC-VIRT-UPLINK (only available when
radio0 is in STA mode). The WiFi state is polled every 3 seconds and reflects changes made via the web UI too.

## SPI kernel patch

`kernel-patches/001-spi-bmc-virt.patch` adds a synthetic SPI controller that:

1. Registers `/dev/spidev0.0` (the app writes LED data here)
2. Mirrors TX bytes to `/proc/bmc_virt_spi0` (the LED visualizer reads from here)

Reference source files are in `kernel-patches/ref/` for future porting.

## Ports

| Service   | Host port | Guest port |
| --------- | --------- | ---------- |
| SSH       | 2222      | 22         |
| HTTP/gRPC | 50080     | 80         |
| IPC       | 5910      | 5910       |

gRPC-Web shares the HTTP listener on port 80 — `bmc/src/web.rs` routes by `Content-Type: application/grpc` rather than
running a separate listener.

Port numbers are defined once in `flake.nix` (`ports = { ... }`).

## Console app

The native console app (`bmc-virt-console`) replaces VNC. It connects to the relay daemon inside the VM via a custom TCP
IPC protocol (port 5910) and provides:

- **Live framebuffer display** with backlight simulation and device frame rendering
- **Touch input injection** — click/drag on the screen area
- **LED strip visualization** with glow effects on a virtual desk surface
- **Control panel** — LED effect presets, volume override, GPIO reset button
- **GPIO reset button** — simulates the physical USR_BTN via netlink uevent injection. A 1-second arming delay acts as a
  safety gate (the real button requires a safety pin). Hold durations map to ButtonManager thresholds: 0–2s reboot, 2–5s
  ignored, 5s+ factory reset.
- **Ping/pong keepalive** — console sends pings every 500ms, relay replies with pongs. A 2-second read timeout detects
  dead connections even when no frames are flowing (static scene).
- **Log viewer** — tails bmc.log, syslog, dmesg, and relay logs

## Boot-time services

Init scripts in the rootfs overlay ensure the VM recovers automatically on reboot:

| Script               | START | Purpose                                                                           |
| -------------------- | ----- | --------------------------------------------------------------------------------- |
| `a-bmc-virt-setup`   | 80    | Device nodes, DRM/VKMS, SPI, backlight, sound, touch                              |
| `a0-bmc-virt-eventd` | 81    | Kobject/uevent daemon                                                             |
| `d-bmc-virt-relay`   | 82    | Relay daemon — starts before bmc-openwrt and self-discovers the compositor socket |
| `b-bmc-openwrt`      | 85    | procd service for the main app                                                    |
| `c-bmc-virt-wifi`    | 90    | Re-applies WiFi config after app's wifi-detect wipe                               |

WiFi uplink credentials are templated into `/etc/bmc-virt/uplink.conf` at deploy time. The WiFi init script reads them
on every boot to re-apply the radio1 AP and radio0 STA config that bmc-openwrt's `wifi-detect` overwrites on startup.

A `bos` stub (`/usr/bin/bos`) provides the `factory_reset` subcommand: restores factory-default flags, removes the
config (`/etc/bmc/config.json` plus the kept pre-migration `/etc/bmc_config.json`, so the boot-time relocation cannot
resurrect it), and reboots.

## macOS prerequisites

- **Nix builder config**: `darwin-ensure-builder.sh` handles this automatically, but `/etc/nix/nix.custom.conf` must
  declare both `aarch64-linux` and `x86_64-linux` builders

### GPU acceleration (required for Wayland capture)

Vanilla QEMU on macOS has no virgl support, so guest GL operations like `glReadPixels` on the panel-sized framebuffer
return `GL_OUT_OF_MEMORY`. The relay never receives frames and the console shows `Connected, waiting for frames…`
indefinitely.

`flake.nix` resolves this on macOS by sourcing QEMU from the
[darwin-qemu-virgl-flake](https://github.com/kubijo/darwin-qemu-virgl-flake) input — a separate flake that builds QEMU
10.0.0 with Akihiko Odaki's macOS texture-borrowing patch + ANGLE-backed virglrenderer. No manual setup needed; nix
fetches and builds the QEMU as part of the normal `./scripts/run.sh` flow.

The display backend is `-display cocoa,gl=es`, which opens a small QEMU-owned native window where virgl gets its
Metal-backed GL context. The relay still captures frames through Wayland; the cocoa window is just QEMU's own display
output and can be ignored or minimised. (`egl-headless` appears in `-display help` but fails at runtime with *"egl: not
available on this platform"* — macOS has no native EGL.)

Run output should show:

```
QEMU: using nix binary at /nix/store/.../bin/qemu-system-aarch64
GPU: virgl via ANGLE→Metal (hardware-accelerated)
```

## Linux (aarch64) prerequisites

```bash
sudo apt install qemu-user-static binfmt-support
echo "extra-platforms = x86_64-linux" | sudo tee -a /etc/nix/nix.custom.conf
sudo systemctl restart nix-daemon
```

This enables binfmt emulation so the x86_64 OpenWrt ImageBuilder can run on aarch64-linux. `linux-ensure-binfmt.sh`
(called automatically from `run.sh`) checks these prerequisites and prints setup instructions if anything is missing.
