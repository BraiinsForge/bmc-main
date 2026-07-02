# BMC Upgrades

The upgrade path is a Nix-backed package manager: applications and their runtime dependencies are published to binary
caches, discovered through lightweight JSON indexes, and installed into custom BMC profile generations (see
[`profiles.md`](profiles.md)). BOS (the firmware image) sits underneath as a separate concern — it is not a Nix
dependency — and its lifecycle is coordinated with the Nix side rather than expressed through it.

This document covers the pieces a contributor needs to work on the upgrade flow itself: what indexes exist, how versions
are chosen, how a firmware upgrade differs from an application-only upgrade, and how downgrade and rollback are handled.

> **Implementation status:** this document describes the implemented `bmc-main` behavior. Remaining cross-repo work:
> pack and invoke the firmware Nix payload from the OpenWrt tarball `COMMAND`, publish factory tarball `.sig` files, and
> flip the shipped `FactoryServerEntry.require_signature` setting once signatures are available.

## Sources of Truth

Two independent inputs drive package selection at runtime:

- **Remote package indexes.** Each configured server publishes `nix-package-index.v1.json` listing available packages
  (name, version, `store_path`, optional `upgrade_strategy` / `install_strategy` hints). Servers are declared in
  `/etc/nix-upgrade/servers.json` with a `priority` and `known_public_key`. Indexes may also transitively reference
  other indexes (`indexes[]`) for federated discovery. Store paths in indexes are realised on-device via
  `nix-store --realise` from the substituters configured on the device; cache metadata in the index (`caches[]`,
  per-package `cache`) is informational only and ignored by resolution.
- **The installed manifest.** Every profile generation carries its own `manifest` — the same shape as an index, plus
  `installed_by` (`system` or `user`), `installed_from` (server id), and `pinned` (semver constraint). The manifest is
  the record of what is currently installed; upgrade planning diffs it against the merged remote view.

There is a third input that only appears during firmware upgrades: a **pinned index** shipped inside the firmware
tarball itself. See [Firmware Upgrades](#firmware-upgrades).

## Resolution Algorithm

`bmc-nix` merges all enabled indexes into a single view keyed by package name, then resolves each package the manifest
cares about. The rules, in order:

1. **Scope to the source server.** If the package is already installed and the server named in `installed_from` lists
   any entry for it, resolution considers only that server's entries. Other servers are consulted only when the origin
   server has no entry for the package at all — an origin offering only unusable (e.g. lower) versions leaves the
   package stale rather than migrating it to another server.
2. **Discard downgrades.** Drop any entry whose version is strictly lower than the currently installed version. This is
   a hard rule — it does not depend on pinning or server priority. Same-version entries are kept because their
   `store_path` may have changed (e.g. a rebuild against a new toolchain), and picking up the new path is a legitimate
   upgrade.
3. **Honour the pin.** If the manifest entry has a `pinned` constraint (`1.2.3`, `^1.2`, `~1.2.3`, `1.2.x`, `>=1.2, <2`,
   etc.), filter candidates to those matching the constraint. A bare full version means "exactly that version". `null`
   means unpinned.
4. **Pick the latest remaining version.**
5. **Break ties by server priority.** Lower `priority` wins. Priorities in `servers.json` must be unique — an ambiguous
   match is a configuration error.
6. **Fail loud on remaining ambiguity.** Two entries with the same name, same version, and same priority that disagree
   on the `store_path` is a publishing bug on the server side. `bmc-nix` refuses to guess; byte-identical entries (same
   `store_path`) are accepted as mirrors.

If the filter chain leaves an installed package with no candidate version, the installed version is kept as-is for both
`system` and `user` packages and reported as *stale*. If a package is present in the manifest but absent from every
consulted index, the `installed_by` field decides the outcome: `system` is a hard failure, while `user` is stale and
kept.

File-level conflicts inside the merged symlink tree are a separate concern — resolved by server priority (with a
warning), see [`profiles.md`](profiles.md).

## Upgrade Flow

An application-layer upgrade — the common case, no firmware change — runs entirely on-device:

1. Fetch and merge indexes from every enabled server in `servers.json`.
2. Realise all target store paths via `nix-store --realise`, which pulls missing NARs and dependencies from the
   configured substituters. Missing store paths abort the upgrade.
3. Build a new profile generation from the resolved package set (`bmc-nix` profile builder — symlink tree, hooks,
   manifest; see [`profiles.md`](profiles.md)).
4. Run the generation's activation entrypoint, which atomically swaps `current` at the write boundary.

The previous generation stays on disk and remains the rollback target.

## Firmware Upgrades

Firmware (BOS) upgrades are orchestrated by `bmc-upgrade` and, unlike a normal package upgrade, must not consult remote
indexes at all. Every firmware tarball ships with:

- a copy of `bmc-nix-cli`, and
- one **pinned** `nix-package-index.v1.json` baked into the image.

That pinned index is the sole and authoritative source of package versions for the Nix step of the firmware upgrade.
Remote indexes and `servers.json` entries are ignored for this run. This guarantees the application set activated
alongside the new BOS matches exactly what the firmware was built and tested against, independent of what any server
currently advertises.

One rule from the general flow still applies: the no-downgrade filter still runs against the currently installed
versions. A firmware upgrade will not roll an installed application backwards even if the pinned index would otherwise
suggest a lower version.

The CLI contract for the tarball path is
`bmc-nix-cli upgrade --only-indexes --index file://<tarball>/nix-package-index.v1.json --next-boot`. `--only-indexes`
makes the consulted index set exactly the explicit `--index` references: it does not read `servers.json`, and it does
not follow federated `indexes[]` references inside those indexes.

**Activation is always deferred.** A firmware upgrade never activates the new profile in-place — the running BOS is on
its way out, and its running services must not be reconfigured to match a generation built for the incoming BOS. The
tarball command must invoke `bmc-nix-cli upgrade --next-boot`, which produces a `next` symlink to a built generation
(see [Deferred Activation](#deferred-activation---next-boot)). After the reboot into the new BOS, `nix-activator`
consumes that symlink and runs the target generation's activation against the previous `current`. Once the device is
back to normal operation, subsequent upgrades resume the ordinary application-layer flow using remote indexes.

There is no code path in the firmware-upgrade flow that activates the profile immediately — treating `--next-boot` as
optional here would risk activating a generation whose services expect kernel/userland facilities the current (about to
be replaced) BOS does not provide.

## Deferred Activation (`--next-boot`)

`bmc-nix-cli upgrade --next-boot` is the same resolution and profile-build path as a normal upgrade, but it stops before
activation. It still builds the next numbered generation directory (`<N>-link`) up front. Instead of swapping `current`,
it atomically writes `next` as a symlink inside the profile directory (`/nix/var/nix/gcroots/profiles/bmc/next`)
pointing at that freshly built `<N>-link`.

Every profile apply run removes a stale `next` symlink immediately after taking the profile lock. This means any later
run invalidates a pending deferred activation, including a no-op run that does not build a replacement generation. A
second `--next-boot` run before reboot therefore replaces the pending target with the newly built generation.

At boot, `nix-activator` checks for `next`. If it is present, the service:

1. Backs up `current` as `previous`.
2. Runs the target generation's activation entrypoint; the generation already exists, so no boot-time promotion or
   renumbering happens.
3. Removes `next` after a successful activation.
4. Restores `previous` and re-activates it if the target activation fails.

If `next` is missing on boot, the service is a no-op — this is the common case on ordinary boots.

The design intent is:

- The upgrade run does not have to defer any of its heavy work to boot; realisation, symlink-tree build, hooks, and
  manifest generation all happen up-front while the CLI is running normally. Only activation is delayed.
- Failure modes are limited. If the reboot never happens or the device is powered off before `nix-activator` runs, the
  `next` symlink simply sits until the next boot or until a later profile apply run invalidates it; nothing about the
  current generation has been touched.
- If the boot-time activation fails, the previous generation is restored and re-activated — the same behaviour as any
  other failed activation.

`--next-boot` is the mechanism the firmware-upgrade path uses (see [Firmware Upgrades](#firmware-upgrades)), but it is
not exclusive to it: any caller that needs to synchronise activation with a reboot can request it.

## BOS Downgrade

On platforms that allow the user to downgrade BOS (currently BMM101), the same firmware-tarball mechanism is reused with
a different resolution outcome. The older firmware's pinned index will typically advertise lower application versions
than what is currently installed.

Nothing special is needed for this case — the no-downgrade filter drops those lower-version entries and the installed
versions are kept as-is. In practice:

- Installed applications keep their current, newer versions across a BOS downgrade.
- The pinned index acts as a floor / compatibility hint only for packages that would otherwise be missing.
- No Nix-level rollback of application packages happens as a side effect. If the user also wants to downgrade an
  application, that is a separate action: profile rollback, or an explicit install pinned to a specific version.

## Rollback

Every generation is a complete profile view; rollback is an atomic swap of `current` back to a previous generation
directory plus a re-run of that generation's activation entrypoint. There is no attempt to reverse arbitrary side
effects — the previous activation scripts are trusted to bring the live system back to their generation's expected
state.

Only rollbacks to generations still present on disk are possible. There is no cross-firmware rollback: the Nix profile
ring and the BOS partition ring are independent, and rolling back BOS does not implicitly roll back the profile (or vice
versa).

## Garbage Collection

The device must always have enough free space for the next upgrade, including the worst case where every derivation
changes (glibc or compiler bumps). `bmc-nix::gc` reclaims space in two stages:

- **Generation cleanup.** Old generation directories are removed according to `/etc/nix-upgrade/gc.json`
  (`keep_generations`, `keep_days`, `protected_generations`, `min_free_space`). The current and the latest generations
  are always kept, as is the generation pointed to by `next`; `protected_generations` is empty by default.
- **Nix store GC.** `nix-collect-garbage` removes store paths no longer referenced by any surviving generation.

GC runs via the `bmc-nix-cli gc` subcommand, intended for a periodic timer or for when disk space runs low — it is not
part of the upgrade flow. A pre-flight free-space check that triggers GC opportunistically before an upgrade is planned
but not implemented yet. Progress is reported so a long GC does not look like a hang.

## Initialization and Factory Reset

The Nix store on new devices is populated in one of three ways:

- **Factory flash.** New devices are shipped with `/nix/store` and `/nix/var/nix` already populated and the initial
  profile activated (or activated on first boot).
- **First-boot upgrade from a pre-Nix firmware.** The first Nix-capable firmware is marked as a required version (users
  cannot skip it). Its image `COMMAND` invokes `bmc-nix-cli init` from the tarball, which prepares `/dev/mmcblk0p4` as
  an ext4 data partition mounted at `/mnt/data`, fetches the initialization tarball for the incoming firmware's version
  (passed by the `COMMAND` via `--bos-version-file`; the running `/etc/bos_version` predates Nix and has no tarball),
  stages it into `/mnt/data/nix.tmp`, copies root overlays to `/`, and atomically promotes `nix.tmp/nix` to `nix` (see
  [`openwrt-tarball.md`](openwrt-tarball.md#initialization)); the profile activates on next boot. If `/mnt/data/nix`
  already exists, `init` exits successfully without changing it unless `--wipe` is passed.
- **Fallback initializer.** A small statically-linked binary is kept forever on the device to recover from a wiped
  store. It offers minimal Wi-Fi configuration, then downloads the tarball listed in `factory` from `servers.json` for
  the current `/etc/bos_version`. Because NTP has not synced yet, the client disables TLS certificate validation and can
  rely on the tarball's Ed25519 signature (verified against `known_public_key`) as the primary integrity guarantee.
  Verification is controlled by `factory.require_signature`: when true, the initializer fetches `<download_url>.sig` and
  aborts on a missing, malformed, or rejected signature; when false (the serde default), it skips verification and logs
  a warning (see [`../stories/nix-store-initializer.md`](../stories/nix-store-initializer.md)).

Factory reset drops a marker file that instructs the initializer to wipe `/nix/store` and its state on the next boot.
Doing it via the initializer avoids fighting running processes that hold open files in the store.

## `installed_by` and Removal Policy

The manifest's `installed_by` field controls how a package is treated during upgrades and removals:

- `system` — installed as part of the core set. Upgraded automatically. The user cannot uninstall.
- `user` — explicitly installed by the user. Kept across upgrades until the user removes it.

This matters for the upgrade planner: `system` packages missing from every consulted index are treated as a hard failure
(something is wrong on the server side); `user` packages missing from every consulted index become stale and hold at
their current version. A package that exists in the indexes but has no candidate version after resolution filters is
stale-and-kept for both `system` and `user`.

## Contributor Checklist

- Never let a resolution path bypass the no-downgrade filter. It is the single load-bearing rule that protects users
  from server-side mistakes.
- Firmware-upgrade code paths must not consult remote indexes. Route through the pinned index shipped in the tarball.
- Failure modes prefer keep-current over guess: stale packages, ambiguous priorities, and unavailable store paths all
  abort cleanly rather than silently substituting.
- Store-path realisation, hook execution, and activation are separate stages. Don't fold side effects into the
  resolution layer.
