# OpenWrt Firmware Tarball

The OpenWrt firmware tarball is the sysupgrade artifact produced by BOS builds — the image `bmc-upgrade` writes to the
alternate partition and reboots into. This document covers the pieces of that tarball that the Nix upgrade path cares
about; the OpenWrt image itself (kernel, rootfs, boot glue) is a BOS concern.

> **Implementation status:** this describes the implemented `bmc-main` firmware payload and CLI behavior. Remaining
> cross-repo work: pack and invoke the payload from the OpenWrt tarball `COMMAND`, publish factory tarball `.sig` files,
> and flip the shipped `FactoryServerEntry.require_signature` setting once signatures are available.

Two things ship inside every tarball that concern `bmc-nix`:

- **`bmc-nix-cli`** — the CLI wrapper around the `bmc-nix` library used during firmware installation and, when the store
  does not exist yet, initialization.
- **The firmware index** — a `nix-package-index.v1.json` pinned to the exact package versions the firmware was built and
  tested against.

These are not optional extras. Every released firmware tarball must contain both, so a device can complete a firmware
upgrade without any network access beyond fetching the tarball and its store paths.

## Why It Ships With Its Own Index

An ordinary application-layer upgrade merges every enabled server's `nix-package-index.v1.json` and resolves against the
live view. A firmware upgrade must not do that. What is compatible with the new BOS is a build-time property of the
firmware — the same set of applications that CI validated against the image. A remote server can gain, lose, or reorder
versions at any time; consulting it during a firmware upgrade would let a device end up with an application set the
firmware release was never tested with.

The firmware index closes that gap. It is a normal `nix-package-index.v1.json` — same schema, same conflict-resolution
rules — but frozen at firmware build time and shipped inside the tarball. During the firmware-upgrade Nix step, it is
the sole and authoritative source of package versions. `bmc-nix-cli upgrade --only-indexes` makes the consulted index
set exactly the explicit `--index` references: remote indexes, `/etc/nix-upgrade/servers.json`, and federated
`indexes[]` recursion are ignored for that run.

See [`upgrades.md`](upgrades.md#firmware-upgrades) for how this index plugs into the overall upgrade flow, and
[`../devlogs/BDK-212/nix/nix-concepts.md`](../devlogs/BDK-212/nix/nix-concepts.md) for the ambient concepts.

## What `bmc-nix-cli` Does From The Tarball

During a firmware upgrade, the tarball `COMMAND` must invoke `bmc-nix-cli` from the tarball with the tarball's firmware
index as input:

```sh
bmc-nix-cli upgrade --only-indexes --index file://<tarball>/nix-package-index.v1.json --next-boot <bos-version>
```

`<bos-version>` is the incoming firmware's version, read from the tarball's own version file — the running
`/etc/bos_version` still names the outgoing BOS.

The CLI:

1. Parses the pinned `nix-package-index.v1.json`.
2. Diffs it against the current profile manifest.
3. Applies the standard resolution rules — including the no-downgrade filter, so lower-version entries in the pinned
   index are discarded and any application already installed at a higher version stays at its current version. See
   [`upgrades.md`](upgrades.md#resolution-algorithm).
4. Ignores `servers.json` and does not follow `indexes[]` references inside the pinned index because `--only-indexes`
   was selected.
5. Realises the resolved store paths from the substituters configured on the device.
6. Builds a new profile generation.
7. Leaves the new generation staged as a `next.<bos-version>` symlink to the built `<N>-link`, named for the incoming
   firmware version. Firmware upgrades **always** take this deferred-activation path — the profile is never activated
   in-place against the outgoing BOS. The incoming firmware's `nix-activator` consumes the marker after the BOS
   partition swap; any other firmware's activator cannot see it and sweeps it as stale, so a failed sysupgrade never
   activates the staged packages. See [Deferred Activation](upgrades.md#deferred-activation---next-boot).

`bmc-nix-cli` inside the tarball is intentionally the same binary used for the offline build-time profile assembly
(`build-profile`, factory-tarball construction). Reusing one CLI keeps the resolution and profile-build code paths
identical across build-host, factory-tarball, and firmware-tarball contexts.

## Consequences for the Firmware Build

Every firmware build must produce, in addition to the OpenWrt image itself:

- The pinned `nix-package-index.v1.json` covering all applications intended to ship with that firmware release.
- A static armv7-musl `bmc-nix-cli` binary for the device.
- A substituter setup on the device (`nix.conf`) from which the on-device `nix-store --realise` step can pull every
  store path the pinned index references.

The pinned index must be self-consistent: every `store_path` it references must be reachable from the configured
substituters. There is no fallback to remote indexes to paper over a missing entry — the run will abort.

## Initialization

The firmware tarball is also the vehicle by which a device that has never had Nix gains a `/nix/store` for the first
time. This is used both by the very first firmware release that introduces Nix (users cannot skip that version) and by
the fallback recovery path after a factory reset that wipes the store.

Two pieces are involved:

- `bmc-nix-cli init` — an initialization subcommand of the same on-tarball CLI. It performs the on-device steps
  described below.
- The firmware image `COMMAND` — the sysupgrade hook that BOS runs before applying the new image. On a Nix-era running
  system — identified by the presence of `/etc/init.d/nix-activator`, a rootfs marker independent of this boot's mount
  and activation state — the `COMMAND` first runs `bmc-nix-cli init` without `--wipe`: it mounts the data partition when
  necessary, no-ops when the store already exists (empty stdout), and otherwise initializes it for the incoming firmware
  version (printing the new profile path), in which case staging is already complete. When the store pre-existed, the
  `COMMAND` restores the `/nix` bind mount when missing, extends `PATH` with `/run/current-profile/bin` (otherwise added
  only by login shells; realisation spawns `nix-store` on the outgoing system), and runs the pinned-index upgrade. On a
  pre-Nix system it runs `bmc-nix-cli init --wipe`, replacing any store left behind by an earlier aborted upgrade with
  one matching the firmware being flashed. In both branches the `COMMAND` passes the incoming firmware's version via
  `--bos-version-file` — the running `/etc/bos_version` may predate Nix and have no factory tarball.

The `init` command is intentionally distinct from the firmware-upgrade flow (`build-profile` and friends). Init operates
before there is any profile to diff against; it only has to populate the store and lay down the initial profile shipped
in the tarball.

### On-device Steps

`bmc-nix-cli init` performs the following, in order:

1. **Prepare the data partition.** The default device is `/dev/mmcblk0p4` and the default mount point is `/mnt/data`.
   The CLI verifies the block device, refuses to touch a partition that backs an active mount, uses `blkid` to detect an
   existing filesystem, and runs `mkfs.ext4` when the partition is blank. It then checks the filesystem with
   `e2fsck -p`, escalating to `e2fsck -y` when preen cannot repair it and reformatting with `mkfs.ext4` when errors
   remain even after that, before creating the mount point and mounting it as ext4. If the partition is already mounted
   at `/mnt/data`, this step is a no-op.
2. **Short-circuit when the store is already promoted.** If `<data-dir>/nix` exists, `init` exits 0 without changing it
   unless `--wipe` is passed. `--wipe` refuses to run while `/nix` is an active mount — the running system would be
   using the very store it deletes.
3. **Select and download the initialization tarball.** Selection is by the BOS version read from `/etc/bos_version`,
   matched against the factory index (the `factory` entry from `/etc/nix-upgrade/servers.json`; see
   [`upgrades.md`](upgrades.md#initialization-and-factory-reset) for how first-boot certificate validation and tarball
   signatures interact). If no tarball matches the current BOS version, the initializer escalates to a full BOS upgrade
   — the latest BOS version always has to have a tarball on the factory server, otherwise this whole path breaks.
4. **Apply the factory tarball signature policy.** If the factory entry has `require_signature: true`, the CLI fetches
   `<download_url>.sig`, verifies the tarball against `known_public_key`, and aborts on a missing, malformed, or
   rejected signature. If `require_signature` is absent or false, verification is skipped and a warning is logged.
5. **Unpack the tarball into `/mnt/data/nix.tmp` and atomically promote `nix.tmp/nix` to `nix`.** The tarball extracts
   into a staging directory inside `/mnt/data`. Only the extracted `nix.tmp/nix` subtree is renamed to `<data-dir>/nix`;
   entries outside `nix/` are ignored and removed with the staging directory — live rootfs files such as
   `/etc/nix/nix.conf` come from activation on first boot. This gives the boot-time services a single check — "does
   `/mnt/data/nix` exist?" — that cannot observe a half-extracted store. A crash or power loss mid-extract leaves
   `nix.tmp` behind, which the next `init` run wipes and re-extracts.

Once `nix` is in place, the initial profile shipped inside the tarball is available and can be activated on next boot.
Activation itself is not part of `init` — the boot-time service handles it, the same way it would after a firmware
upgrade.

### Relation to the Fallback Initializer

The static fallback initializer covered in [`upgrades.md`](upgrades.md#initialization-and-factory-reset) — the small
program kept forever for recovering from a wiped store on a device that no longer has any Nix-produced code available —
performs a similar sequence, but from outside the tarball. `bmc-nix-cli init` is the in-tarball path for the common
case, where the tarball itself carries the code that will do the initialization; the static initializer is the
last-resort path for when the tarball cannot be applied because nothing on the device can execute it yet.

Both paths must converge on the same on-disk layout: `/mnt/data/nix` containing the store,
`/nix/var/nix/gcroots/profiles/` carrying the initial profile, with `nix.tmp` used only as a staging directory.

## BOS Downgrade

On platforms that allow downgrading BOS (currently BMM101), the older firmware's tarball still contains its own pinned
firmware index. The mechanism is unchanged: `bmc-nix-cli` runs against that older index, the no-downgrade filter drops
any lower-version entries, and the currently installed applications stay at their current versions. No separate
downgrade code path is needed — this falls out of the resolution algorithm. See
[`upgrades.md`](upgrades.md#bos-downgrade).

## Contributor Checklist

- Never let the firmware-upgrade code path fall back to remote indexes. The pinned firmware index inside the tarball is
  the only source.
- Every firmware tarball must include both `bmc-nix-cli` and a self-consistent firmware index. Omitting either is a
  release blocker.
- Treat the firmware index as a normal `nix-package-index.v1.json`. Do not invent a separate schema for it; sharing the
  schema is what lets one CLI serve both flows.
- The no-downgrade rule applies here too. Do not add a firmware-upgrade-only bypass to "force" older versions in — that
  is what manual profile rollback is for.
