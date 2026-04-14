# bmc-virt Development Log

## RR time-travel debugging (#BDK-378, #BDK-383)

### rr infrastructure — working end-to-end

Built a full time-travel debugging pipeline for embedded binary debugging:

1. **Custom OpenWrt kernel** (nix-cached): OpenWrt 24.10.6 kernel 6.6.127 + OpenWrt patches +
   `CONFIG_PROC_PAGE_MONITOR=y` (rr requirement) + `CONFIG_DRM_VKMS=y` (software DRM for rr).
2. **3-derivation architecture**: `vmImageBase` (ImageBuilder, prebuilt packages) → `customKernel` (kernel only) →
   `vmImage` (x86_64: swap kernel in boot partition via debugfs; aarch64: pass-through, direct-boot via QEMU -kernel).
   No 60-min full builds.
3. **rr bundle**: rr 5.9.0 + glibc deps packaged as a nix derivation, auto-deployed to musl VM.
4. **`make rr`**: one-command workflow — builds rr-profiled binary (debug=true, strip=false), boots VM with custom
   kernel (VKMS), records under rr, pulls trace on SSH disconnect.
5. **AMD Zen**: SpecLockMap must be disabled for replay (`wrmsr -a 0xc0011020`). Hint printed after recording pull.

### RR replay UIs

- **Pernosco** — full visual debugger by the rr creators. Commercial/cloud. https://pernos.co/
- **VS Code** — connect to rr's GDB server with `rr replay -s 50505` then attach with "Native Debug" or "CodeLLDB".
- **CLion** — native rr support, attach to GDB server.

### Key learnings

- **rr + shell commands**: rr serializes all process execution via ptrace, making `ubus call` ~100x slower. Each
  fork/exec round-trips through ptrace (2 context switches per syscall). The FactoryDefault path's `wait_for_wifi_ssid`
  (15 retries × 2s + shell commands) takes >5 min under rr vs \<1s normally.
- **rr + DRM**: rr can't record GPU DMA. VKMS (pure software DRM) works but has no display output. The headless event
  loop fallback runs the full Slint pipeline without a framebuffer.
- **Stock OpenWrt kernel**: lacks `CONFIG_PROC_PAGE_MONITOR`. No prebuilt image has it.

## aarch64 native guest support (#BDK-383)

### Architecture

The VM guest now matches the host architecture: aarch64 hosts get an aarch64 OpenWrt guest (HVF/KVM accelerated), x86_64
hosts keep the x86_64 guest. The flake detects the host arch and conditionalizes everything — ImageBuilder URL, kernel
config, QEMU machine type, build profiles, binary names.

### macOS (Apple Silicon) support

On macOS, all Linux builds delegate to a custom NixOS builder VM that runs under HVF. The builder has
`boot.binfmt.emulatedSystems = [“x86_64-linux”]` so the x86_64 ImageBuilder binary can run on it. The
`darwin-ensure-builder.sh` script auto-starts the builder VM when needed.

The OpenWrt QEMU guest runs directly on macOS with HVF acceleration. Display is via VNC (guest-side server on port 5900,
macOS viewer connects to localhost).

### OpenWrt 23.05 → 24.10 upgrade

Upgraded to align GCC versions (kmod ABI mismatch caused kernel panics on aarch64). See README.md for full rationale.

### Workspace changes

- `workspace.nix`: added `aarch64-debug` and `aarch64-release` build profiles, aarch64 variants of
  bmc-virt-{dashboard,leds,vnc}
- `flake.nix` (root): added aarch64 cross-compiler to devShell, guarded Linux-only env vars behind `isLinux`
- Rust toolchain already had `aarch64-unknown-linux-musl` target — no change needed

## Native console app (#BDK-383)

Replaced VNC with a native egui console app (`bmc-virt-console`) that connects to a relay daemon (`bmc-virt-relay`)
inside the VM via a custom TCP IPC protocol. The relay captures the VKMS DRM framebuffer and SPI LED data, the console
renders them with a device frame, backlight dimming, LED glow effects, and touch input injection.

## GPIO reset button (#BDK-392)

### Uevent injection

Replaced the brute-force `killall bmc-openwrt` with proper GPIO USR_BTN simulation. The console sends `GpioButton`
press/release IPC messages to the relay, which injects netlink kobject uevents matching the format of OpenWrt's
`gpio-button-hotplug` module (`SUBSYSTEM=button`, `BUTTON=reset`, `ACTION=pressed/released`).

**Key finding:** netlink multicast from userspace doesn't reliably deliver to other userspace sockets on
`NETLINK_KOBJECT_UEVENT`. The fix: unicast to each listener PID discovered from `/proc/net/netlink`.

### IPC ping/pong keepalive

Console sends `Ping` every 500ms, relay replies with `Pong`. Combined with a 2-second read timeout on the console's
reader socket, this detects dead connections even when no frames are flowing (static scene / VM shutdown).

### Declarative VM boot state

Moved device setup and service management from the imperative deploy script into OpenWrt init.d scripts
(`a-bmc-virt-setup`, `b-bmc-openwrt`, `c-bmc-virt-wifi`, `d-bmc-virt-relay`) so the VM recovers automatically on reboot.
WiFi uplink credentials are templated into `/etc/bmc-virt/uplink.conf` and re-applied after bmc-openwrt's `wifi-detect`
wipes wireless config. A `bos` stub provides `factory_reset` for the VM environment.

All binaries and assets (bmc-openwrt, relay, frontend, sounds, fonts, LED viz) are now packed into a single overlay tar
push instead of separate scp commands.
