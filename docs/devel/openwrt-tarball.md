# OpenWrt Firmware Tarball

The OpenWrt firmware tarball is the sysupgrade artifact produced by BOS builds — the image `bmc-upgrade` writes to the
alternate partition and reboots into. This document covers the pieces of that tarball that the Nix upgrade path cares
about; the OpenWrt image itself (kernel, rootfs, boot glue) is a BOS concern.

> **Implementation status:** this describes the implemented `bmc-main` firmware payload and CLI behavior, and the
> BOS-side `COMMAND` that packs and invokes it. Remaining cross-repo work: content signature verification does not exist
> yet — the CLI has no signature policy, verifies nothing against `known_public_key`, and the shipped
> `servers.json.default` carries a placeholder key.

Two things ship inside every tarball that concern `bmc-nix`:

- **`bmc-nix-cli`** — the CLI wrapper around the `bmc-nix` library used during firmware installation and, when the store
  does not exist yet, initialization.
- **`servers.json.default`** — the shipped server registry, whose server entries link package feeds
  (`nix-package-feed.v1.json`).

These are not optional extras. Every released firmware tarball must contain both, so the firmware-upgrade Nix step can
resolve the incoming firmware's package index without depending on whatever registry state the outgoing system carries.

## Why the Index Is Resolved Through a Feed

An ordinary application-layer upgrade merges every enabled server's index and resolves against the live view. A firmware
upgrade uses the same registry merge, but scopes every feed-linked server to the incoming firmware version. What is
compatible with the new BOS is a build-time property of the firmware — the same set of applications that CI validated
against the image. A feed's *current* entry can gain, lose, or reorder versions at any time; resolving it without the
incoming firmware scope could select an application set the firmware release was never tested with.

The package feed closes that gap server-side. Each feed entry maps a `bos_version` to that firmware's own package index
(`index_url`) — an ordinary `nix-package-index.v1.json`, same schema, same conflict-resolution rules, published
per-release and left alone afterwards. The firmware-upgrade Nix step passes `--firmware <incoming version>`, so each
feed-linked server resolves to the index published for that firmware. Enabled direct index servers and federated child
indexes still join the merged view. The Forge release server is required, so its failure aborts keep-current; optional
third-party server failures warn and are skipped when another source succeeds.

See [`upgrades.md`](upgrades.md#firmware-upgrades) for how this plugs into the overall upgrade flow, and
[`../devlogs/BDK-212/nix/nix-concepts.md`](../devlogs/BDK-212/nix/nix-concepts.md) for the ambient concepts.

## What `bmc-nix-cli` Does From The Tarball

During a firmware upgrade, the tarball `COMMAND` must invoke `bmc-nix-cli` from the tarball with the staged registry as
the fallback:

```sh
bmc-nix-cli upgrade \
    --default-servers-config <staged>/servers.json.default \
    --firmware <bos-version> \
    --next-boot
```

`<bos-version>` is the incoming firmware's version, read from the tarball's own version file — the running
`/etc/bos_version` still names the outgoing BOS. `--next-boot` derives its marker version from the required explicit
`--firmware`, so feed selection and deferred activation cannot target different firmware versions.

The CLI:

1. Loads the server registry: a valid runtime `/etc/nix-upgrade/servers.json` wins wholesale; the staged
   `servers.json.default` is used only when the runtime file is absent (a malformed runtime file is quarantined to
   `.bcp` first).
2. Resolves each feed-linked server: fetches its `nix-package-feed.v1.json`, selects the entry for `--firmware`, and
   fetches the index at that entry's `index_url`, then merges enabled direct indexes and federated children.
3. Diffs the merged view against the current profile manifest and applies the standard resolution rules — including the
   no-downgrade filter, so lower-version entries are discarded and any application already installed at a higher version
   stays at its current version. See [`upgrades.md`](upgrades.md#resolution-algorithm).
4. Realises the resolved store paths from the substituters configured on the device.
5. Builds a new profile generation.
6. Leaves the new generation staged as a `next.<bos-version>` symlink to the built `<N>-link`, named for the incoming
   firmware version. Firmware upgrades **always** take this deferred-activation path — the profile is never activated
   in-place against the outgoing BOS. The incoming firmware's `nix-activator` consumes the marker after the BOS
   partition swap; any other firmware's activator cannot see it and sweeps it as stale, so a failed sysupgrade never
   activates the staged packages. See [Deferred Activation](upgrades.md#deferred-activation---next-boot).

`bmc-nix-cli` inside the tarball is intentionally the same binary used for the offline build-time profile assembly
(`build-profile`, factory-tarball construction). Reusing one CLI keeps the resolution and profile-build code paths
identical across build-host, factory-tarball, and firmware-tarball contexts.

## `COMMAND` Responsibilities and the Double Validation

The tarball's `COMMAND` is the BOS-side driver of everything above. It runs on the outgoing system as part of
sysupgrade's image validation: it stages the payload (`bmc-nix-cli` and `servers.json.default`) into a temporary
directory, selects the init or upgrade branch (see [Initialization](#initialization)), and invokes the tarball CLI with
the incoming firmware's version. Staging completes before the flash — a failure aborts the sysupgrade keep-current — and
the boot into the new firmware only ever consumes what staging left behind (the promoted store, or the
`next.<bos-version>` marker).

The branch decision uses `bmc-nix-cli is-initialized`, whose exit status is a contract with the `COMMAND`:

- `0` — the store is fully initialized and bind-mounted at `/nix`;
- `1` — the store is absent or incomplete;
- `3` — the store is fully initialized but is not bind-mounted at `/nix`;
- `2` — the command encountered a runtime failure.

Exits `1` and `3` select the wipe-and-init path. A fully initialized but unmounted store is an inconsistent recovery
state, such as one left by a failed first-Nix sysupgrade, so the incoming firmware reinitializes it instead of trusting
its profile. Exit `2` aborts the sysupgrade so a runtime failure can never be mistaken for a recoverable store state.
`init --wipe` still refuses to run while anything is mounted at `/nix`.

BOS validates a sysupgrade image twice per run: `/sbin/sysupgrade` calls `platform_check_image` through
`/usr/libexec/validate_firmware_image`, and procd's `upgraded` re-validates the image before flashing. Without a guard,
each pass would stage the profile and build its own generation. The `COMMAND` therefore records the target firmware
version in `/tmp/bos-nix-profile-prepared` after a successful staging pass; the second pass finds its own target version
in the marker, skips staging, and consumes the marker. Consuming it (rather than keeping it until reboot) means a
leftover marker from an interrupted run can only shift staging to a later run's second pass, never suppress it for a
whole run; the version content means a marker left by a run targeting a different firmware never matches. `/tmp` is
tmpfs, so a reboot clears the marker regardless. The marker is an optimization, not a correctness gate — a missed skip
only builds a redundant generation, and a second `--next-boot` run simply replaces the pending marker with the newer
generation.

## Consequences for the Firmware Build

Every firmware release must produce, in addition to the OpenWrt image itself:

- A published `nix-package-index.v1.json` covering all applications intended to ship with that firmware release, and a
  package feed entry for the release's `bos_version` pointing at it via `index_url`.
- A static armv7-musl `bmc-nix-cli` binary for the device (shipped in the tarball).
- A `servers.json.default` registry in the tarball whose feed-linked entries name the feeds serving those releases.
- A substituter setup on the device (`nix.conf`) from which the on-device `nix-store --realise` step can pull every
  store path the release's index references.
- A `bmc-nix-cli` bundled in the firmware rootfs itself, plus a `nix-activator` boot service that is a thin wrapper over
  it (`bmc-nix-cli mount`, then `bmc-nix-cli activate --generation next`), so boot-time marker consumption always speaks
  the grammar the image was built with. See [Deferred Activation](upgrades.md#deferred-activation---next-boot).

The required release index must be self-consistent: every `store_path` it references must be reachable from the
configured substituters. Optional indexes may supplement it, but cannot make a failed required server succeed.

## Initialization

The firmware tarball is also the vehicle by which a device that has never had Nix gains a `/nix/store` for the first
time. This is used by the very first firmware release that introduces Nix (users cannot skip that version).

Two pieces are involved:

- `bmc-nix-cli init` — an initialization subcommand of the same on-tarball CLI. It performs the on-device steps
  described below.
- The firmware image `COMMAND` — the sysupgrade hook that BOS runs before applying the new image. On a Nix-era running
  system — identified by the presence of `/etc/init.d/nix-activator`, a rootfs marker independent of this boot's mount
  and activation state — the `COMMAND` first runs `bmc-nix-cli init` without `--wipe`: it mounts the data partition when
  necessary, no-ops when the store is fully initialized (empty stdout), and otherwise initializes it for the incoming
  firmware version (printing the new profile path), in which case staging is already complete. When the store
  pre-existed, the `COMMAND` restores the `/nix` bind mount when missing, extends `PATH` with `/run/current-profile/bin`
  (otherwise added only by login shells; realisation spawns `nix-store` on the outgoing system), and runs the
  feed-resolved upgrade. On a pre-Nix system it runs `bmc-nix-cli init --wipe`, replacing any store left behind by an
  earlier aborted upgrade with one matching the firmware being flashed. In both branches the `COMMAND` passes the
  incoming firmware's version via `--firmware` — the running `/etc/bos_version` may predate Nix and have no factory
  tarball.

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
2. **Short-circuit when the store is initialized.** `init` requires a nonempty `<data-dir>/nix/store`, the
   `<data-dir>/nix/var/nix/db/db.sqlite` database, and the `<data-dir>/nix/var/nix/gcroots/profiles/bmc` profile. When
   all three are present, it exits 0 without changing the store unless `--wipe` is passed. An incomplete store is not
   overwritten implicitly; recovery must explicitly pass `--wipe`. `--wipe` refuses to run while `/nix` is an active
   mount — the running system would be using the very store it deletes.
3. **Select and download the initialization tarball.** Selection is by `--firmware`, falling back to the version in
   `/etc/bos_version` when the flag is omitted. The firmware `COMMAND` passes the incoming version explicitly because
   the running file still names the outgoing firmware. The version is matched against the factory server's package feed
   (`nix-package-feed.v1.json`, fetched from the `factory` entry of `/etc/nix-upgrade/servers.json`). If no feed entry
   matches the requested BOS version, `init` fails — the factory server has to keep an entry for every Nix-capable BOS
   version, otherwise this path breaks.
4. **No content verification.** Signature verification is not implemented: the CLI consults neither the factory entry's
   `known_public_key` nor any signature policy for the feed or the tarball. Because NTP has not synced on first boot,
   the download client also disables TLS certificate validation, so the downloads are trusted by URL alone. (NAR
   substitutions on the package-upgrade path are unaffected — nix verifies those against the `trusted-public-keys` that
   `register-server` writes to `nix.conf`.)
5. **Unpack the tarball into `/mnt/data/nix.tmp` and atomically promote `nix.tmp/nix` to `nix`.** The tarball extracts
   into a staging directory inside `/mnt/data`. Only the extracted `nix.tmp/nix` subtree is renamed to `<data-dir>/nix`;
   entries outside `nix/` are ignored and removed with the staging directory — live rootfs files such as
   `/etc/nix/nix.conf` come from activation on first boot. This gives the boot-time services a single check — "does
   `/mnt/data/nix` exist?" — that cannot observe a half-extracted store. A crash or power loss mid-extract leaves
   `nix.tmp` behind, which the next `init` run wipes and re-extracts.

Once `nix` is in place, the initial profile shipped inside the tarball is available and can be activated on next boot.
Activation itself is not part of `init` — the boot-time service handles it, the same way it would after a firmware
upgrade.

## BOS Downgrade

On platforms that allow downgrading BOS (currently BMM101), the feed keeps its entry for the older `bos_version`, so the
older firmware's `--firmware` scope resolves to its own release index. The mechanism is unchanged: `bmc-nix-cli` runs
against that older index, the no-downgrade filter drops any lower-version entries, and the currently installed
applications stay at their current versions. No separate downgrade code path is needed — this falls out of the
resolution algorithm. See [`upgrades.md`](upgrades.md#bos-downgrade).

## Contributor Checklist

- The firmware-upgrade code path scopes every feed-linked server to the `--firmware` target. Keep the Forge release
  server required; optional third-party servers may degrade when another source succeeds, and federation continues
  normally.
- Every firmware tarball must include both `bmc-nix-cli` and `servers.json.default`, and every release must publish a
  feed entry whose `index_url` names a self-consistent release index. Omitting any of these is a release blocker.
- Treat the feed-resolved index as a normal `nix-package-index.v1.json`. Do not invent a separate schema for it; sharing
  the schema is what lets one CLI serve both flows.
- The no-downgrade rule applies here too. Do not add a firmware-upgrade-only bypass to "force" older versions in — that
  is what manual profile rollback is for.
- Staging runs from image validation, which BOS performs twice per sysupgrade. Keep the `/tmp/bos-nix-profile-prepared`
  guard version-keyed and consumed on use, and never make correctness depend on it.
