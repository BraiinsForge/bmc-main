# libudev-zero Implementation Plan

Status: **Complete**. All stages green on both HW (Goodix panel) and VM (QEMU virtio tablet). This file remains as the
record of what was done and why, including paths investigated and discarded.

## Assumptions (initial)

- `HEAD` is reverted first, so the working baseline is the path-based libinput approach from `HEAD^`, not the raw-evdev
  fallback.
- The objective is a working touch stack on the current appliance image via libinput, backed by libudev-zero.
- The preferred fix lives in packaging/runtime dependencies, not in compositor-side input parsing.

## Architectural notes

- **VM mirrors HW, no VM-awareness in core software.** The compositor, widgets, and relay target the real Deck. The VM's
  job is to replicate that environment closely enough that the app runs unchanged. Every change must hold on both
  targets; any fix that would only make sense on the VM belongs in the VM image, not in `bmc-openwrt` / `bmc-platform` /
  widgets.
- **Capability-based input discovery, not name matching.** The compositor and the VM relay share
  `bmc_platform::linux_input::discover_touch_node`, which matches
  `(ABS_X + ABS_Y) ∨ (ABS_MT_POSITION_X + ABS_MT_POSITION_Y)` with `BTN_TOUCH` and dedupes by canonical parent input
  device. Vendor/name strings are never inspected. Works identically for Goodix (mixed legacy + MT axes) and QEMU's
  virtio tablet.
- **libudev-zero is additive, not a replacement for OpenWrt's `mdev`.** OpenWrt's busybox `mdev` still creates
  `/dev/input/event*` at hotplug time. libudev-zero is a library in the Nix closure that libinput links against; it
  synthesises `ID_INPUT_*` tags from sysfs on demand when libinput queries. No OS-level device manager is swapped.

## Stage 1: Restore the libinput baseline

**Goal**: Re-establish the path-based libinput compositor path after reverting the raw-evdev fallback.

**Success criteria**:

- `bmc-openwrt` constructs `Libinput::new_from_path(RootLibinputInterface)` and registers `LibinputInputBackend` in the
  compositor event loop.
- The raw-evdev detour (`touch_input.rs`, `evdev` dep, direct `Generic<OwnedFd>` touch registration) is gone.
- The compositor uses capability-based discovery via `bmc_platform::linux_input::discover_touch_node` rather than any
  hardcoded `/dev/input/event0`.

**Status**: Complete. Baseline was restored upstream (`b5112988`). `DEFAULT_INPUT_NODE` was retired in favour of shared
discovery in `bmc-platform::linux_input`, consumed identically by the compositor and the VM relay.

## Stage 2: Swap the appliance builds to libudev-zero

**Goal**: Make every appliance-image package set (HW + VM) provide `libudev.so.1` from libudev-zero instead of
systemd-minimal-libs, and relink `libinput` against the same libudev provider so the library and the runtime can't
disagree.

**Success criteria**:

- `applianceOverlay` in `workspace.nix` composes `libinput.override { udev = libudev-zero }` and publishes a
  `compositorUdev` marker attribute on each appliance package set (`armv7Pkgs`, `x86Pkgs`, `aarch64Pkgs`).
- `compositorRuntimeDeps` / `guiTargetDeps` resolve `libudev.so.1` from libudev-zero; `systemd-minimal-libs` does not
  end up on the libinput runtime edge.
- On a real boot, libinput registers the discovered touchscreen as `ID_INPUT_TOUCHSCREEN` without any
  `"udev device never initialized"` or `"not tagged as supported input device"` messages.

**libudev-zero pin**: nixpkgs ships libudev-zero 1.0.3, whose `set_properties_from_evdev` checks `EV_REL` before
`EV_ABS`. Any device carrying both (e.g. QEMU's `virtio-tablet-pci`, with `REL_WHEEL + ABS_X/ABS_Y + BTN_TOUCH`) falls
through the `EV_REL` branch and never reaches the `TOUCHSCREEN` tag, so libinput refuses the device as
`not tagged as supported input device`. Upstream fix
[`bbeb7ad5`](https://github.com/illiliti/libudev-zero/commit/bbeb7ad5) (*"Fixes incorrect detection of touchpads
(#66)"*) swaps the branch order so `EV_ABS` wins. No release after 1.0.3 includes the fix, so `applianceOverlay` pins
libudev-zero's `src` to the fix commit directly.

**Status**: Complete. HW (cargo-deployed via `scripts/nix-cargo-deploy.sh`) logs
`libinput registered /dev/input/event0 (name='Goodix Capacitive TouchScreen', …)`, gesture arbitration and scene-commit
fire on swipe. VM logs `libinput registered /dev/input/event1 (name='QEMU Virtio Tablet', …)` with the same predicate +
pinned libudev-zero, touch events reach the compositor, scenes swipe.

## Stage 3: Prove libinput delivers touch end-to-end

**Goal**: Confirm tap, drag, scene swipe, and cancel semantics through the full stack on both targets with the code
exactly as it will ship.

**Tests** (performed 2026-04-23):

- VM: `just validate` green, `nix run .#run` in `bmc-virt/` starts the guest, `scripts/get-logs.sh` pulls `bmc.log`,
  grep confirms the `libinput registered` line and `DragEnd` / `Scene transition committed` entries after swipe.
- HW: `nix develop .#armv7-glibc-release --command scripts/nix-cargo-deploy.sh compositor 192.168.1.183` relinks the
  Deck's compositor binary + copies the updated closure (new libudev-zero and relinked libinput).
  `start-compositor bmc-openwrt` on the device brings the Wayland compositor up; Goodix touches drive the scene
  transition logs as above.
- On either target, `libinput debug-events` can be used as a secondary sanity check when the libinput CLI is in the
  closure.

**Status**: Complete on both targets.

## Paths investigated and discarded

Recorded so the same dead ends aren't re-walked on adjacent tickets.

- **`mdevd` + coldplug as a uevent rebroadcaster.** Early diagnosis assumed libudev-zero needed a running uevent
  broadcaster to populate state, because its README names `mdevd -O 4` + `contrib/helper.c` as the recommended
  integrations. Implemented and verified to do nothing useful on either target: libudev-zero reads sysfs on demand
  during `udev_device_new_from_syspath`, and the touch devices on both HW and VM are present at boot, so there is no
  hotplug event to rebroadcast. The real failure was libudev-zero 1.0.3's branch-order bug (see Stage 2). Live-verified
  on the VM after the bbeb7ad5 pin: killed mdevd, restored `/proc/sys/kernel/hotplug = /sbin/hotplug` (broken missing
  target), restarted the compositor — libinput still registered the tablet as `ID_INPUT_TOUCHSCREEN` and delivered
  events. All mdevd infrastructure was removed before commit.
- **QEMU `virtio-multitouch-pci` to get an "unambiguous touchscreen" on the VM.** Switched for one iteration while
  diagnosing what we thought was a pointer/touch classification failure. Turned out to just expose a pure-protocol-B
  device (MT axes only, no legacy `ABS_X/ABS_Y`) which libudev-zero 1.0.3 *also* refused to tag — same bbeb7ad5 bug.
  Reverted to `virtio-tablet-pci` once the libudev-zero pin was in place. The tradeoff (virtio-tablet's REL/mouse-button
  noise vs virtio-multitouch's pure MT) is moot: both work with the libudev-zero fix, virtio-tablet is closer to what
  the Deck's Goodix panel looks like (mixed legacy + MT axes), so it stays.
- **Compositor-side raw evdev handling.** Already rejected in the revert that kicks off this branch. The path forward
  was always to make the existing path-based libinput + libudev integration actually work, not reimplement libinput.
- **uinput-backed host evdev passthrough via `virtio-input-host-pci`.** Considered when we thought the only way to get a
  "Goodix-shaped" profile on the VM was to synthesise one from the host. Architecturally clean but dwarfed by the much
  smaller libudev-zero pin. Discarded.

## Observability added

- Compositor installs a log-priority hook on its `libinput` context at `DEBUG` so libinput's own diagnostic lines
  (device classification, tag checks, quirks) reach `bmc.log` via the default stderr handler. Makes
  `"not tagged as supported input device"`-class rejections visible by default — that single message was the
  load-bearing diagnostic in this ticket. ~12 lines of FFI in `bmc-openwrt/src/compositor/egl_compositor.rs`.
