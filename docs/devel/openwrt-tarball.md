# OpenWrt Firmware Tarball

The OpenWrt firmware tarball is the sysupgrade artifact produced by BOS builds — the image `bmc-upgrade` writes to the
alternate partition and reboots into. This document covers the pieces of that tarball that the Nix upgrade path cares
about; the OpenWrt image itself (kernel, rootfs, boot glue) is a BOS concern.

Two things ship inside every tarball that concern `bmc-nix`:

- **`bmc-nix-cli`** — the CLI wrapper around the `bmc-nix` library used during firmware installation.
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
the sole and authoritative source of package versions. Remote indexes and `/etc/nix-upgrade/servers.json` entries are
ignored for that run.

See [`upgrades.md`](upgrades.md#firmware-upgrades) for how this index plugs into the overall upgrade flow, and
[`../devlogs/BDK-212/nix/nix-concepts.md`](../devlogs/BDK-212/nix/nix-concepts.md) for the ambient concepts.

## What `bmc-nix-cli` Does From The Tarball

During a firmware upgrade, `bmc-upgrade` invokes `bmc-nix-cli` from the tarball with the tarball's firmware index as
input. The CLI:

1. Parses the pinned `nix-package-index.v1.json`.
2. Diffs it against the current profile manifest.
3. Applies the standard resolution rules — including the no-downgrade filter, so lower-version entries in the pinned
   index are discarded and any application already installed at a higher version stays at its current version. See
   [`upgrades.md`](upgrades.md#resolution-algorithm).
4. Realises the resolved store paths (the caches to fetch from are declared inside the pinned index's `caches[]`).
5. Builds a new profile generation.
6. Leaves the new generation inactive and writes the pending-activation flag; activation happens on the next boot after
   the BOS partition swap.

`bmc-nix-cli` inside the tarball is intentionally the same binary used for the offline build-time profile assembly
(`build-profile`, factory-tarball construction). Reusing one CLI keeps the resolution and profile-build code paths
identical across build-host, factory-tarball, and firmware-tarball contexts.

## Consequences for the Firmware Build

Every firmware build must produce, in addition to the OpenWrt image itself:

- The pinned `nix-package-index.v1.json` covering all applications intended to ship with that firmware release.
- A `bmc-nix-cli` binary matching the target architecture (armv7-glibc for `bmc-openwrt`).
- Enough closure information that the on-device `nix-store --realise` step can pull the required store paths from the
  caches referenced inside the pinned index. (In practice the pinned index carries the same `caches[]` block that a
  remote index would.)

The pinned index must be self-consistent: every `store_path` it references must be reachable from a cache in its
`caches[]`, and every checker package the release relies on must be listed. There is no fallback to remote indexes to
paper over a missing entry — the run will abort.

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
