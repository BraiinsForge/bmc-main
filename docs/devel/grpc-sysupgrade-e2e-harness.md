# End-to-End gRPC Sysupgrade Harness

`deck e2e-grpc-sysupgrade` upgrades a real Deck's firmware through the production gRPC API — the same `CheckForUpgrade`
/ `StartUpgrade` calls the web UI makes — and proves the flash landed by a boot-id change. It is the gRPC counterpart of
[`e2e-sysupgrade`](sysupgrade-e2e-harness.md): where that harness flashes over SSH and drives the firmware's `COMMAND`
directly, this one asks the device's own bmc to fetch and flash the image from a host-served firmware index, exercising
the whole discovery → download → flash → reboot chain the device runs in production. It is a development harness;
nothing here ships on the device. It flashes a single image and, unlike `e2e-sysupgrade`, is non-destructive to the nix
store: it upgrades in place and restores the device's configuration on exit.

## What It Tests

The run drives one image through the real upgrade RPCs against a fully local package rig served from the developer
machine:

- **`CheckForUpgrade`** must offer exactly the image's version. The harness serves an `index.v1.json` describing the
  image and points bmc at it via `BMC_INDEX_URL`; the offer is asserted to match the image's canonical version before
  anything is flashed.
- **`StartUpgrade`** streams the upgrade phases (`FIRMWARE_UPGRADE_PHASE_DOWNLOADING` → `…_VERIFYING` → `…_APPLYING`).
  The device downloads the image from the host index, verifies it, and flashes it.
- **The flash is proven, not assumed.** After the stream, the harness requires a changed boot id, the target firmware
  version, and fetch provenance (the index actually served `/firmware.tar`) — all re-checked through a pinned device
  identity so a resolver handing out a different Deck cannot pass the run.

Driving the real supervised path is what makes this harness valuable beyond `e2e-sysupgrade`: bmc stays procd-supervised
through the flash, and that is the exact condition under which `/sbin/sysupgrade` returns a nonzero exit on its
**success** path (procd execs stage2 without answering the final ubus call). That fault — a completed flash reported as
`Internal: Upgrade failed` — is invisible to the SSH harness and was found and fixed through this one (#BDK-611).

## Building the Firmware Image

The harness needs **one** bos-main sysupgrade tarball, newer than the version the device reports and built for the
target board family. It must be Nix-era (it carries the on-tarball `bmc-nix-cli` and `servers.json.default` payload
members); a legacy or wrong-board image is rejected in the "Validate firmware image" stage before anything runs.

There is no two-image dance and no manual version bookkeeping. `CheckForUpgrade` only offers a release strictly newer
than the installed one, so the harness **anchors** the device first: the "Ensure anchor version" stage rewrites
`/etc/bos_version` to a synthetic release below the image's, preserving the running date, commit, and build suffix. Any
newer Nix-era image works, and re-running the same image needs no prep — the anchor is re-applied on each run and
restored on the failure paths that stay on the same boot.

## Prerequisites

- A Deck reachable over the network from the developer machine. The harness serves three HTTP endpoints the device must
  reach: the firmware index (`--index-port`, default 8082) and the package feed and its index (`--packages-port` /
  `--packages-index-port`, defaults 8080 / 8081). `CheckForUpgrade` probes the package servers as well as the firmware
  index, so all three ports must be open between host and device.

- Auto-upgrade disabled on the device. The harness asserts this up front (via `GetAutoUpgrade`) and aborts with an
  instruction to disable it, so a scheduled upgrade cannot race the test.

- `grpcurl` on the developer machine (the harness ensures it is present) to speak gRPC-web to the device.

- A local nix able to build the package plan the rig serves, plus enough free RAM on the device for the flash
  (sysupgrade stages the tar in tmpfs and pivots to a ramdisk). The "Memory headroom" check runs before any mutation and
  aborts with the shortfall rather than flashing blind.

## Invocation

```sh
nix run .#deck -- e2e-grpc-sysupgrade --device DEVICE_IP --image PATH_TO_TARBALL
```

- `--index-port` (default 8082) — the host port serving the firmware index and `firmware.tar`.
- `--packages-port` / `--packages-index-port` (defaults 8080 / 8081) — the host ports serving the package feed and its
  index, which `CheckForUpgrade` also probes.
- `--password` — the device login password, if the web password is set.
- `--stream-deadline` (default 900) — seconds to wait on the `StartUpgrade` stream before giving up.

## What the Stages Do

**Preflight (read-only, no mutation):** **Ensure grpcurl**, **Device reachable**, **Validate firmware image**
(board-family checked), **Require nix-era image**, **Preflight versions** (the image is strictly newer than the device),
**Preflight device**, **Resolve / build packages** (build the plan the rig serves), **gRPC login**, and **Require
auto-upgrade disabled**. A failure here leaves the device untouched.

**Snapshot and pin:** **Snapshot device identity** (the board serial from `/proc/device-tree/serial-number`) and **Pin
device address** (resolve `--device` to a numeric address for the reboot window), then **Verify device identity**
through the pinned address. **Snapshot** the upgrade config, opkg keys, `/etc/bos_version`, and the bmc service script —
the byte snapshots used to restore the device afterwards.

**Prepare the device:** **Memory headroom**, then **Ensure anchor version** (rewrite `/etc/bos_version` below the image
release so the offer appears), **Upload firmware** and **Trust image signing keys** (accept the dev-signed image),
**Start / register the package servers** (the registration is `--exclusive`, so it disables every other server entry and
leaves the factory entry alone), and **Only the harness server resolves**, which asserts that exclusivity took rather
than assuming it — an enabled production entry decides the upgrade whenever it publishes a higher version, and an
unreachable `required` one fails the whole `CheckForUpgrade` package probe.

**Serve and point bmc at the index:** assemble the serve tree (`firmware.tar` plus the `index.v1.json` naming the
running and image versions, the image URL, its sha256, and size), start the recording index server, then **Point bmc at
the index** — inject `BMC_INDEX_URL` into the procd service environment and restart bmc **supervised**. bmc must stay
under procd through the flash; an unsupervised bmc outlives procd's sysupgrade teardown and misreads the success-path
exit (see #BDK-611).

**Drive the upgrade:** **Await bmc ready**, **gRPC login**, **CheckForUpgrade** (assert the offer is the image version),
**Snapshot boot id**, then stream **StartUpgrade** and classify the outcome. On a provisional success the harness polls
for a boot-id change, re-pins and re-verifies identity through the pinned address, and requires the flashed version to
match the image and the index to have served `/firmware.tar`.

**Safety properties:** identity is pinned before any mutation and re-verified after the reboot; an identity mismatch
performs no restoration, keeps the firmware index server alive for up to 120 s so any in-flight transfer to the intended
device can drain, and then exits without mutating anything. Ambiguous outcomes retain the snapshots and abort rather
than "restore" blindly onto an unknown device.

## Cleanup

Restoration splits on whether the device rebooted. On a **same-boot** failure the harness byte-restores the service
script, opkg keys, `/etc/bos_version`, `servers.json`, and `/etc/nix/nix.conf` from their snapshots and restarts stock
bmc. On a **changed boot** it removes the injected `BMC_INDEX_URL` token and restores `nix.conf` and `servers.json`, but
never byte-restores the pre-reboot service script, opkg keys, or `bos_version` — those snapshots belong to a firmware
generation the flash replaced. Either way it stops the package and index servers, removes the uploaded firmware tar, and
deletes the snapshot directory. Unlike `e2e-sysupgrade`, this harness restores `/etc/nix/nix.conf` from a snapshot, so
it leaves no rig lines behind on a completed run.
