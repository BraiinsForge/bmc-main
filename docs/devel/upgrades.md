# BMC Upgrades

The upgrade path is a Nix-backed package manager: applications and their runtime dependencies are published to binary
caches, discovered through lightweight JSON indexes, and installed into custom BMC profile generations (see
[`profiles.md`](profiles.md)). BOS (the firmware image) sits underneath as a separate concern — it is not a Nix
dependency — and its lifecycle is coordinated with the Nix side rather than expressed through it.

This document covers the pieces a contributor needs to work on the upgrade flow itself: what indexes exist, how versions
are chosen, how a firmware upgrade differs from an application-only upgrade, and how downgrade and rollback are handled.

## Sources of Truth

Two independent inputs drive package selection at runtime:

- **Remote package indexes.** Each configured server publishes `nix-package-index.v1.json` listing available packages
  (name, version, `store_path`, `installed_from` server, cache reference, optional `upgrade_strategy` /
  `install_strategy` hints). Servers are declared in `/etc/nix-upgrade/servers.json` with a `priority` and
  `known_public_key`. Indexes may also transitively reference other indexes (`indexes[]`) for federated discovery. Store
  paths in indexes are realised on-device via `nix-store --realise` from the caches declared in each index's `caches[]`.
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

If the filter chain leaves an installed package with no candidates, the installed version is kept as-is. If a package is
present in the manifest but absent from every index, it is reported as *stale* (also kept as-is).

File-level conflicts inside the merged symlink tree are a separate concern — resolved by server priority (with a
warning), see [`profiles.md`](profiles.md).

## Upgrade Flow

An application-layer upgrade — the common case, no firmware change — runs entirely on-device:

1. Fetch and merge indexes from every enabled server in `servers.json`.
2. Run compatibility checker packages against the resolved target set. Checkers are shipped as regular packages and
   validate things Nix cannot (wayland protocol versions, kernel driver presence, minimum BOS version, etc.). Checks
   always run against the *target* version, not the currently active one. A negative checker verdict either escalates to
   a full firmware upgrade or blocks the operation.
3. Realise all target store paths via `nix-store --realise`, which pulls missing NARs and dependencies from the caches
   declared in the index. Missing store paths abort the upgrade.
4. Build a new profile generation from the resolved package set (`bmc-nix` profile builder — symlink tree, hooks,
   manifest; see [`profiles.md`](profiles.md)).
5. Run the generation's activation entrypoint, which atomically swaps `current` at the write boundary.

The previous generation stays on disk and remains the rollback target.

## Firmware Upgrades

Firmware (BOS) upgrades are orchestrated by `bmc-upgrade` and, unlike a normal package upgrade, do not consult remote
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

Sequencing: the new Nix profile is prepared before the firmware image is applied but not activated. `bmc-upgrade`
invokes `bmc-nix-cli upgrade --next-boot`, which produces a "next" pointer instead of a live generation (see
[Deferred Activation](#deferred-activation---next-boot)). After the reboot into the new BOS, an OpenWrt boot service
promotes that pointer into a real generation and runs its activation. Once the device is back to normal operation,
subsequent upgrades resume the ordinary application-layer flow using remote indexes.

## Deferred Activation (`--next-boot`)

`bmc-nix-cli upgrade --next-boot` is the same resolution and profile-build path as a normal upgrade, but it stops before
activation and before promoting the built generation into the profile ring. Instead, it writes a **`next` file** — a
small marker inside the profile directory (`/nix/var/nix/gcroots/profiles/bmc/next`) that points at the staged
generation contents and captures the intent to activate them on the next boot.

At boot, a dedicated OpenWrt init service (`bmc-nix-next` or equivalent) checks for `next`. If it is present, the
service:

1. Promotes the staged contents into the next numbered generation directory (`<N>-link`), taking the same atomic rename
   step used by a normal profile build.
2. Removes the `next` marker.
3. Runs the new generation's activation entrypoint against the previous `current`, atomically swapping `current` at the
   write boundary.
4. Leaves the system in the same state a normal upgrade would leave it in — previous generation still on disk as a
   rollback target, new generation active.

If `next` is missing on boot, the service is a no-op — this is the common case on ordinary boots.

The design intent is:

- The upgrade run does not have to defer any of its heavy work to boot; realisation, symlink-tree build, hooks, and
  manifest generation all happen up-front while the CLI is running normally. Only activation is delayed.
- Failure modes are limited. If the reboot never happens or the device is powered off before the init service runs, the
  `next` marker simply sits until the next boot; nothing about the current generation has been touched.
- If the boot-time promotion or activation fails, the current generation remains active — the same behaviour as any
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
  (`keep_generations`, `keep_days`, `protected_generations`, `min_free_space`). Generation 1 (factory) is protected by
  default.
- **Nix store GC.** `nix-collect-garbage` removes store paths no longer referenced by any surviving generation.

GC runs via the `bmc-nix-cli gc` subcommand, intended for a periodic timer or for when disk space runs low — it is not
part of the upgrade flow. A pre-flight free-space check that triggers GC opportunistically before an upgrade is planned
but not implemented yet. Progress is reported so a long GC does not look like a hang.

## Initialization and Factory Reset

The Nix store on new devices is populated in one of three ways:

- **Factory flash.** New devices are shipped with `/nix/store` and `/nix/var/nix` already populated and the initial
  profile activated (or activated on first boot).
- **First-boot upgrade from a pre-Nix firmware.** The first Nix-capable firmware is marked as a required version (users
  cannot skip it). Its image `COMMAND` downloads the initial store tarball and extracts it to the root partition; the
  profile activates on next boot.
- **Fallback initializer.** A small statically-linked binary is kept forever on the device to recover from a wiped
  store. It offers minimal Wi-Fi configuration, then downloads the tarball listed in `factory` from `servers.json` for
  the current `/etc/bos-version`. Because NTP has not synced yet, the client disables TLS certificate validation and
  relies on the tarball's Ed25519 signature (verified against `known_public_key`) as the primary integrity guarantee.

Factory reset drops a marker file that instructs the initializer to wipe `/nix/store` and its state on the next boot.
Doing it via the initializer avoids fighting running processes that hold open files in the store.

## `installed_by` and Removal Policy

The manifest's `installed_by` field controls how a package is treated during upgrades and removals:

- `system` — installed as part of the core set. Upgraded automatically. The user cannot uninstall.
- `user` — explicitly installed by the user. Kept across upgrades until the user removes it.

This matters for the upgrade planner: `system` packages missing from a new index are treated as a hard failure
(something is wrong on the server side); `user` packages missing from any index become stale and hold at their current
version.

## Contributor Checklist

- Never let a resolution path bypass the no-downgrade filter. It is the single load-bearing rule that protects users
  from server-side mistakes.
- Firmware-upgrade code paths must not consult remote indexes. Route through the pinned index shipped in the tarball.
- Failure modes prefer keep-current over guess: stale packages, ambiguous priorities, and unavailable store paths all
  abort cleanly rather than silently substituting.
- Store-path realisation, hook execution, and activation are separate stages. Don't fold side effects into the
  resolution layer.
