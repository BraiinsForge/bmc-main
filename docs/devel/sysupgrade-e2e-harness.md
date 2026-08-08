# End-to-End Firmware Sysupgrade Harness

`deck e2e-sysupgrade` drives the firmware sysupgrade Nix flow against a real Deck. Where `upgrade-e2e` serves packages
and asserts a profile bump (see [End-to-End Package Upgrade Harness](upgrade-e2e-harness.md)), this harness flashes two
full firmware images and exercises the two branches the firmware's `COMMAND` takes during pre-flash image validation:
the init path and the in-place upgrade path. It is a development harness; nothing here ships on the device, and scenario
A is destructive — it wipes the device's nix store.

## What It Tests

The run has two scenarios, driven against a fully local package rig served from the developer machine:

- **Scenario A — init path.** Clears the nix store and flashes image A over the cleared store. With no store present,
  `COMMAND` takes the init branch and downloads the factory init tarball from the rig; at boot the activator promotes a
  generation from the tarball (which ships a generation but no `current` symlink).
- **Scenario B — upgrade path.** Drops a preservation marker into the backing store, then flashes image B onto image A's
  lineage. With a store already initialized, `COMMAND` takes the upgrade branch, resolving feed → index → rig cache and
  staging a `next` generation; at boot the activator consumes it, the store is upgraded **in place** (the marker
  survives), `current` advances, and a bumped package path is realised from the rig cache.

A `full` run does A then B back to back. Registration re-runs before each flash — it is idempotent and keeps the rig
authoritative regardless of what the previous flash left behind (sysupgrade preserves both `/etc/nix/nix.conf` and the
runtime `servers.json` as registered conffiles).

## Building the Two Firmware Images

The harness needs **two** bos-main sysupgrade tarballs — image A and image B — and they must carry **different**
firmware versions. Two same-version images make the upgrade plan empty (an index identical to the installed set never
invokes `nix-store`), and the run aborts at the "Validate e2e images" stage. Both images must also be Nix-era (they
carry the on-tarball `bmc-nix-cli` and `servers.json.default` payload members) and be built for the target board family;
a legacy or wrong-board image is rejected before anything destructive runs.

The firmware version is derived from the build's git state, so the way to get two differing images is to build the same
branch twice with a trivial commit in between:

1. Prepare bos-main on the branch you want to test, with its flake inputs pointing at your bmc-main feature branch (see
   the `bos-firmware-prep` flow). This fixes the code under test.
2. Trigger the `firmware-bmc100` CI job and download the resulting sysupgrade tarball. This is **image A**.
3. Make a trivial change and commit it. A bare commit bump is enough to move the derived firmware version — no code
   change is required. Push it.
4. Trigger `firmware-bmc100` again and download that tarball. This is **image B**, the newer version.

Image B must be the newer of the two: scenario B upgrades an A-initialized store to B, and the harness asserts the
device is running image A's version before it flashes B.

## Prerequisites

- A Deck reachable over the network from the developer machine: the harness serves the rig over HTTP on `--serve-port`
  (default 8083), so that port must not be firewalled between the two.

- The developer machine's route to the device is autodetected for the rig's advertised address; override it with
  `--serve-ip` when the autodetected interface is wrong.

- Enough free RAM on the device for the flash: sysupgrade stages the tar in `/tmp` (tmpfs) and pivots to a ramdisk, and
  the key-trust stage transiently extracts `rootfs.img`. The harness checks this ("Memory headroom") before each flash
  and aborts with the shortfall rather than flashing blind.

- A local nix able to build the init artifacts. The harness builds each image's index and init tarball with an
  `--impure` `nix build` of `nix/e2e-artifacts.nix` for the exact versions the tarballs carry (one consistent evaluation
  of the worktree); variant B additionally bumps `bmc-nix-cli` `0.1.0 → 0.1.1` so the upgrade plan against an
  A-initialized store is non-empty.

## Invocation

```sh
nix run .#deck -- e2e-sysupgrade --device DEVICE_IP --image-a PATH_TO_A --image-b PATH_TO_B
```

- `--scenario init|upgrade|full` (default `full`) — run only scenario A, only scenario B (which requires a store already
  initialized by a prior A or a plain init), or both in sequence.
- `--serve-ip` / `--serve-port` — the device-facing rig address (default: autodetected) and its HTTP port (default
  8083).
- `--yes` skips the two destructive confirm prompts (the store cleardown and each flash).
- `--dry-run` runs the read-only probes and logs every mutation without executing it — the device is left untouched.

## What the Stages Do

The preamble runs once: **Device reachable**, **Validate firmware image** (each image, board-family checked), **Validate
e2e images** (the two versions differ and both are Nix-era), **Build e2e artifacts**, **Build bmc-nix-cli**, **Assemble
rig** (write the serve tree and the signed cache, record every URL the device must fetch), and **Preflight rig from
device** (probe the first bytes of every rig URL from the device itself, so a routing, firewall, or host-detection fault
fails here rather than after the store is gone).

**Scenario A — init path:**

1. **Push bmc-nix-cli** / **Register rig on device** — stage the CLI and write a runtime `servers.json` whose factory
   entry is the rig (the only way to redirect init), plus a feed-linked server entry and substituter.
2. **Memory headroom** / **Pin device address** — check RAM, then resolve `--device` to a numeric address for the
   destructive window (the cleardown stops avahi with the generation's other services, so an mDNS name can stop
   resolving mid-run).
3. **Upload firmware** / **Uploaded image on pinned device** — push image A to `/tmp` and verify its on-device sha256,
   then re-verify over the pinned connection so a resolver handing out a different Deck fails before anything
   destructive.
4. **Trust image signing keys** — install image A's usign public keys so the flash's signature check accepts the
   dev-signed image (deliberately not `sysupgrade -F`, which would also wave through a failed platform check).
5. **Clear nix store** — stop the active generation's services, prove nothing still references `/nix`, unmount, and
   delete the backing store. This is the destructive premise of the init path.
6. **Flash firmware (e2e)** — flash image A unconditionally (no same-version skip: after the cleardown a skip would
   strand the device storeless), wait for the reboot, then **Verify initialized**: the device runs image A's version,
   `/nix` is backed by the store, the activator's fallback promoted a `current` generation, and its services are
   running.

**Scenario B — upgrade path:**

1. **Device on image A's firmware** / **Store initialized** / **Bumped path absent** — read-only preconditions asserted
   before any mutation, so a failed check leaves the device untouched: the device is on image A's lineage, the store is
   initialized, and the bumped store path does not yet exist (its later presence is what proves the upgrade realised it
   from the rig).
2. **Push bmc-nix-cli** / **Register rig on device** / **Drop e2e marker** / **Record generation** — re-register
   (idempotent — the rig stays authoritative whatever the reboot into A left behind), drop the preservation marker
   outside the store/profile trees, and record variant B's current generation for the advance check.
3. **Memory headroom** / **Pin device address** / **Upload firmware** / **Uploaded image on pinned device** / **Trust
   image signing keys** — as in scenario A, for image B.
4. **Flash firmware (e2e)** — flash image B, wait for the reboot, then **Verify upgraded**: the device runs image B's
   version, the marker survived (proving an in-place upgrade rather than a wipe), `current` advanced past the recorded
   generation, the active manifest lists the bumped path (realised from the rig cache), the `next.<version-B>` marker
   was consumed, and the generation's services are running.

On any exit — success or abort, once the first device mutation has happened — the harness best-effort removes the e2e
marker and the uploaded firmware tars, and restores the runtime `servers.json` to its pre-run bytes (removing it only if
it was absent before the run; the registry is captured at run start).

## Cleanup

The harness cleans up after itself device-side, with one exception it cannot undo automatically: the rig's
`extra-substituters` and `extra-trusted-public-keys` lines that registration adds to `/etc/nix/nix.conf`. That file is a
preserved conffile, so those lines survive the flashes and outlive the run, still pointing at the (now gone) ephemeral
rig. Remove them from `/etc/nix/nix.conf` on the device when you are done.

## Fault-injection suite (`deck e2e-sysupgrade-faults`)

The negative counterpart of the happy path: twenty scenarios across four groups — init-signature faults (A),
partition/partial-store damage (B), upgrade-path faults (C), and delivery variants (D). Same preamble and two-image
contract as `deck e2e-sysupgrade`; both image arguments are always required.

```
nix run .#deck -- e2e-sysupgrade-faults --device deck.local \
    --image-a fw-A.tar --image-b fw-B.tar --scenario all --yes
```

- `--scenario` takes a scenario slug (e.g. `unsigned-feed`; the BDK-601 matrix ids A1…D5 map 1:1), a group (`a`–`d`), or
  `all` (default). Every scenario asserts its read-only preconditions first, so a mis-sequenced single run aborts with
  the device untouched. Group/suite runs thread the lineage themselves (one cleardown for `all`; C5/D1/D4/D5 ride other
  scenarios' flashes instead of spending extra flash cycles).
- `full-store` (C7) is the only scenario that damages the device's free space rather than the rig: it fills the store's
  filesystem with `dd` — the device busybox has neither `fallocate` nor `truncate`, and a size derived from `df`'s
  available column would leave ext4's root reserve for nix to spend — then requires the flash to abort with the running
  system untouched. Filling the Deck's free 1.7 GiB takes about four minutes with no output. The ballast is released
  through the strict restore step, so a release failure fails the run rather than handing every later scenario a full
  store — except when the scenario is already failing, where the release degrades to a logged warning so it cannot mask
  the primary error. Because that path, and a run killed outright, can both leave the ballast behind, every flash sweeps
  it up front (`sweep_store_ballast` in `_prepare_flash`); the file is `/mnt/data/.e2e-store-ballast` if you ever need
  to remove it by hand. Recovery spends no flash cycle of its own, since the `cache-swap-retry` run that follows proves
  an upgrade still lands on the freed store.
- `--no-servers-json-preserved` downgrades D5 to observe-only for images predating the conffile registration (#BDK-358).
  By default D5 snapshots the runtime `servers.json` before the flash and asserts it comes back byte-identical.
- The rig signs its init tarballs (mandatory since BDK-376): the cache key doubles as the factory trust anchor, and the
  feed carries per-variant `signature` lines produced by `bmc-nix-cli sign-init-tarball`.
- B-group recovery contract: a failed B-scenario leaves the device reachable over ssh with services stopped and a
  damaged or absent store. The suite attempts one good-rig re-flash of image A before aborting; the manual fallback is
  another `deck e2e-sysupgrade --scenario init` run (or `bmc-nix-cli init --wipe` on the device). The serial console is
  never required.
- B2's ext4 corruption recipe is proven host-side by `bmc-tui/tests/test_ext4_recipe.py` on a loopback image; the device
  run only confirms the fixture. A divergence is a finding, not an accepted outcome. OpenWRT's e2fsprogs ships no
  `debugfs`, so the harness cross-builds a static one and pushes it to a tmpfs `/tmp` path (built lazily, only when a B2
  run needs it, and swept with the other pushed artifacts) rather than requiring `debugfs` on the device.
