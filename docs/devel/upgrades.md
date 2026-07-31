# BMC Upgrades

The upgrade path is a Nix-backed package manager: applications and their runtime dependencies are published to binary
caches, discovered through lightweight JSON indexes, and installed into custom BMC profile generations (see
[`profiles.md`](profiles.md)). BOS (the firmware image) sits underneath as a separate concern — it is not a Nix
dependency — and its lifecycle is coordinated with the Nix side rather than expressed through it.

This document covers the pieces a contributor needs to work on the upgrade flow itself: what indexes exist, how versions
are chosen, how a firmware upgrade differs from an application-only upgrade, and how downgrade and rollback are handled.

> **Implementation status:** this document describes the implemented `bmc-main` behavior. The OpenWrt tarball `COMMAND`
> packs and invokes the firmware Nix payload (see [`openwrt-tarball.md`](openwrt-tarball.md)). Init verifies the factory
> tarball's Ed25519 signature against the factory entry's `known_public_key` by default. Remaining cross-repo work:
> bmc-packages does not yet sign the published feed entries, and the shipped `servers.json.default` carries a
> placeholder key.

## Sources of Truth

Two independent inputs drive package selection at runtime:

- **Remote package indexes.** Package indexes (`nix-package-index.v1.json`) list available packages (name, version,
  `store_path`, optional `upgrade_strategy` / `install_strategy` hints). Servers are declared in
  `/etc/nix-upgrade/servers.json` with a `priority` and `known_public_key`, and each entry links its content through
  exactly one of two source URLs — both exact document URLs, never base URLs:

  - `index_url` — the server's package index document, fetched directly.
  - `feed_url` — the server's **package feed** (`nix-package-feed.v1.json`), a per-firmware release catalog whose
    `entries` map a `bos_version` to that firmware's init tarball (`download_url`, `profile_path`) and package index
    (`index_url`). Resolution fetches the feed, selects the entry for the target firmware version, and follows its
    `index_url`. A feed-linked server participating in resolution without a firmware scope, or a selected entry without
    an `index_url`, is a hard error. Fetch failures follow the server's `required` policy: required servers abort, while
    optional servers warn and are skipped when another source succeeds. Resolution still fails when every enabled source
    fails.

  Indexes may also transitively reference other indexes (`indexes[]`) for federated discovery. Store paths in indexes
  are realised on-device via `nix-store --realise` from the substituters configured on the device; cache metadata in the
  index (`caches[]`, per-package `cache`) is informational only and ignored by resolution.

- **The installed manifest.** Every profile generation carries its own `manifest` — the same shape as an index, plus
  `installed_by` (`system` or `user`), `installed_from` (server id), and `pinned` (semver constraint). The manifest is
  the record of what is currently installed; upgrade planning diffs it against the merged remote view.

The firmware scope for feed-entry selection comes from `bmc-nix-cli upgrade --firmware <BOS_VERSION>`; without the flag,
`/etc/bos_version` is read — but only when an enabled feed-linked server actually participates, so index-only registries
and `--only-indexes` runs never touch it.

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
4. Run the generation's activation entrypoint, which atomically swaps `current` as its final activation step.

The previous generation stays on disk and remains the rollback target.

## Firmware Upgrades

Firmware (BOS) upgrades are orchestrated by `bmc-upgrade`. They merge all enabled package servers just like a normal
package upgrade, but scope each feed-linked server to the incoming firmware version. Every firmware tarball ships with:

- a copy of `bmc-nix-cli`, and
- a `servers.json.default` registry whose server entries link package feeds.

The Nix step resolves against the server registry with the shipped file as the fallback: a valid runtime
`/etc/nix-upgrade/servers.json` — explicit persistent device state, e.g. a `register-server` registration — wins
wholesale; the staged default is used only when the runtime file is absent (a malformed runtime config is quarantined to
`.bcp` first). Each feed-linked server's feed is fetched, the entry for the incoming firmware version is selected, and
that entry's `index_url` joins the merged view alongside enabled direct index servers and federated child indexes.

The Forge release server is required, so an unavailable feed, a missing target entry, or a missing `index_url` aborts
the upgrade keep-current. Optional third-party server failures warn and are skipped when another source succeeds.

One rule from the general flow still applies: the no-downgrade filter still runs against the currently installed
versions. A firmware upgrade will not roll an installed application backwards even if the target firmware's index would
otherwise suggest a lower version.

The CLI contract for the tarball path is

```sh
bmc-nix-cli upgrade \
    --default-servers-config <staged>/servers.json.default \
    --firmware <bos-version> \
    --next-boot
```

where `<bos-version>` is the incoming firmware's version read from the tarball's own version file — not the running
`/etc/bos_version`, which still names the outgoing BOS. `--firmware` scopes feed-entry selection to the incoming
firmware; `--next-boot` requires it and derives the marker version from the same value, because silently scoping to the
outgoing `/etc/bos_version` during a sysupgrade is exactly the bug the explicit flag prevents. BOS runs the tarball
`COMMAND` from image validation, which happens twice per sysupgrade run; the `COMMAND` stages once and skips the second
pass via a `/tmp` marker (see
[`openwrt-tarball.md`](openwrt-tarball.md#command-responsibilities-and-the-double-validation)).

**Activation is always deferred.** A firmware upgrade never activates the new profile in-place — the running BOS is on
its way out, and its running services must not be reconfigured to match a generation built for the incoming BOS. The
tarball command must invoke `bmc-nix-cli upgrade --firmware <bos-version> --next-boot`, which produces a
`next.<bos-version>` symlink to a built generation (see [Deferred Activation](#deferred-activation---next-boot)). After
the reboot into the new BOS, `nix-activator` consumes that symlink and runs the target generation's activation against
the previous `current`. Because the marker carries the incoming version in its name, an activator on any other firmware
— in particular the old BOS after a sysupgrade that failed before flashing — never finds it and removes it as stale
instead of activating packages resolved for a firmware it is not running. Once the device is back to normal operation,
subsequent upgrades resume the ordinary application-layer flow using remote indexes.

There is no code path in the firmware-upgrade flow that activates the profile immediately — treating `--next-boot` as
optional here would risk activating a generation whose services expect kernel/userland facilities the current (about to
be replaced) BOS does not provide.

## Deferred Activation (`--next-boot`)

`bmc-nix-cli upgrade --firmware <bos-version> --next-boot` is the same resolution and profile-build path as a normal
upgrade, but it stops before activation. It still builds the next numbered generation directory (`<N>-link`) up front.
Instead of swapping `current`, it atomically writes `next.<bos-version>` as a symlink inside the profile directory
(`/nix/var/nix/gcroots/profiles/bmc/next.<bos-version>`) pointing at that freshly built `<N>-link`. The `--firmware`
value is the BOS version whose boot may consume the staged generation — the version the resolved index belongs to.
Encoding it in the marker name is what gates activation: an activator never checks a version, it simply cannot see a
marker staged for a different firmware.

Every profile apply run removes stale deferred-activation markers (all `next.*`, and a bare legacy `next`) immediately
after taking the profile lock. This means any later run invalidates a pending deferred activation, including a no-op run
that does not build a replacement generation. A second `--next-boot` run before reboot therefore replaces the pending
target with the newly built generation.

At boot, the firmware's `nix-activator` init script consumes the staged marker. The BOS image bundles both the script
and `bmc-nix-cli`; the script is a thin wrapper that restores the persistent-store bind mount (`bmc-nix-cli mount`,
ending the boot step early when the store is unavailable) and then runs `bmc-nix-cli activate --generation next`.
Keeping the script thin is deliberate: the marker grammar and the activation contract live in exactly one place — the
CLI built into the same image — so the boot script can never skew against the staging side.

`activate --generation next` implements the boot contract (in `bmc-nix::activation`). It reads the running version from
`--bos-version-file` (default `/etc/bos_version`) and looks for `next.<version>`. When the marker for the running
version is present and resolves to a generation directory, it runs that generation's activation entrypoint with
`current` still naming the previous generation — the entrypoint's final `write-boundary` step is the only thing that
moves `current`, so a crash or failure before it leaves `current` on the previous generation. The entrypoint derives
`PROFILE_OLD_GENERATION` from `current` itself, so diff-driven scripts see the real old generation without it being
passed in. The marker is consumed only on success; on failure the activator restores `current` to the target it
snapshotted before the entrypoint ran (removing it when none existed), then falls through to re-activating `current` to
reconcile the live system back onto the previous generation. Markers staged for other versions are removed as stale.
When `/etc/bos_version` is missing or empty, staleness is undecidable: all markers are left alone and `current` is
activated. Without a marker for the running version the activator falls back to activating `current`, which is a no-op —
the common case on ordinary boots; on the very first boot after initialization no `current` exists yet and the fallback
activates the latest generation instead, which is how the initial profile comes live.

Not every supported firmware bundles the activator. For firmware without one, the core package carries a shell port of
the same boot contract (`nix/pkgs/core/files/nix-activator`), and its `060` activation entry (`firmware-init-services`)
provisions it as an overlay copy of `/etc/init.d/nix-activator` plus an `S91` rc.d link; on firmware with a bundled
activator it leaves the init.d path alone and drops the overlay link, so exactly one enabled link remains — with two,
boot ran the activator twice and stacked `/nix` bind mounts. Neither the overlay copy nor the link is a sysupgrade
conffile: flashing any firmware sheds the bridge. A bundling image boots its own activator to consume the staged marker;
flashing another bridge-needing image leaves the profile dormant until the next deploy or init re-runs activation. The
shell variant and the provisioning entry are transitional: remove them once no supported firmware lacks the bundled
activator.

One portability trap for that shell port (and for any script that runs on the device, tarball `COMMAND` included): the
device BusyBox is built without `CONFIG_FEATURE_TR_CLASSES`, so `tr -d '[:space:]'` deletes the literal characters
`space:[]` instead of whitespace — this once mangled the version read from `/etc/bos_version` and made the activator
sweep its own matching marker as stale. Use explicit character sets (`tr -d ' \t\r\n'`). nixpkgs' BusyBox enables the
feature, so the nix-run script tests cannot catch this class of bug; only the device shows it.

Activating any generation other than `current` follows this same revert rule, not just the staged marker: the whole
sequence runs under the profile lock, and a failed entrypoint triggers automatic re-activation of the old generation. A
reverted activation still reports an error — `RevertedAfterFailure`, carrying the original failure — so callers
(including the boot service) see it as a failed run even though the device is left on a working generation. If the
revert re-activation itself fails, the error is `RevertFailed`, carrying both the original and the revert failure.

`bmc-nix` never writes the `current` symlink; a generation's own activation scripts do, as does the boot-time activator
when it restores the previous `current` after a failed staged activation. The core package's final activation step
(`998`, the `bmc-activation-write-boundary` binary) moves `current` atomically (a temporary symlink, then a rename) and
durably (a filesystem sync before the flip, a directory fsync after it), so `current` advances only after every other
step has succeeded; `095-link-current` derives `/run/current-profile` from it. Reverting to the old generation works by
re-running its activation entrypoint, which moves `current` back through the same mechanism. The one exception is the
boot-time activator: when a staged entrypoint fails after advancing `current`, the activator restores the snapshotted
target through the same tmp-symlink-and-rename pattern, so its fallback resolves to the previous generation instead of
the failed one.

The design intent is:

- The upgrade run does not have to defer any of its heavy work to boot; realisation, symlink-tree build, hooks, and
  manifest generation all happen up-front while the CLI is running normally. Only activation is delayed.
- Failure modes are limited. If the reboot never happens or the device is powered off before `nix-activator` runs, the
  marker simply sits until a boot of its firmware version consumes it, an activator of another version sweeps it as
  stale, or a later profile apply run invalidates it; nothing about the current generation has been touched.
- If the boot-time activation fails, the previous generation is restored and re-activated — the same behaviour as any
  other failed activation.

`--next-boot` is the mechanism the firmware-upgrade path uses (see [Firmware Upgrades](#firmware-upgrades)), but it is
not exclusive to it: any caller that needs to synchronise activation with a reboot can request it.

## BOS Downgrade

On platforms that allow the user to downgrade BOS (currently BMM101), the same firmware-tarball mechanism is reused with
a different resolution outcome. The older firmware's feed-resolved index will typically advertise lower application
versions than what is currently installed.

Nothing special is needed for this case — the no-downgrade filter drops those lower-version entries and the installed
versions are kept as-is. In practice:

- Installed applications keep their current, newer versions across a BOS downgrade.
- The older firmware's index acts as a floor / compatibility hint only for packages that would otherwise be missing.
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
  (`keep_generations`, `keep_days`, `protected_generations`). By default, the two highest numbered generations are kept.
  The current and latest generations, every generation referenced by a deferred-activation marker, explicitly protected
  generations, generations covered by `keep_days`, and transient generations protected by the caller can retain more
  than that default.
- **Nix store GC.** `nix-collect-garbage` removes store paths no longer referenced by any surviving generation.

`bmc` collects Nix garbage every two hours. The schedule is a cron job on the shared scheduler, and its offset inside
the two-hour period is drawn randomly at each startup, so devices booted together are unlikely to collect together. The
first collection lands between 30 minutes and two hours after startup, keeping it clear of the boot window; a clock
correction or a DST fall-back can defer it by one further occurrence.

Each run cleans up profile generations, then runs `nix-collect-garbage` only when that cleanup actually removed
something, so an idle device does no store scan. When a run removes generations but the store sweep then fails, the next
run sweeps unconditionally — those generations are already gone, so no later cleanup would count them again.

Automatic upgrades collect unconditionally after claiming an available upgrade and before firmware or package work
begins, so a sequence of automatic upgrades cannot accumulate store paths. The collection is best-effort — what decides
the attempt is the free-space check after it: a package upgrade aborts before download when the estimated unpacked size
plus 10% headroom exceeds the free space on the store filesystem. Without an estimate (the dry-run timed out) or a
free-space reading, the attempt proceeds and the realization itself fails if space runs out. Firmware images download to
tmpfs and skip the check. Manual UI-driven upgrades run the same free-space check but never the collection.
`bmc-nix-cli gc` also forces collection and reports progress so a long collection does not look like a hang.

To stop periodic collection while debugging on a device, set `periodic` in `/etc/nix-upgrade/gc.json`:

```json
{ "periodic": "disabled" }
```

The next occurrence honors it — no restart needed — and the file survives a sysupgrade. The toggle covers only the
periodic path: collection before an automatic upgrade and `bmc-nix-cli gc` both still run.

## Initialization and Factory Reset

The Nix store on new devices is populated in one of two ways:

- **Factory provisioning.** SD card images carry `/nix/store` and `/nix/var/nix` already initialized inside the image.
  eMMC devices will ship the initial store read-only on a dedicated partition, with the working store initialized from
  that offline copy. The initial profile is activated (or activated on first boot).
- **First-boot upgrade from a pre-Nix firmware.** The first Nix-capable firmware is marked as a required version (users
  cannot skip it). Its image `COMMAND` invokes `bmc-nix-cli init` from the tarball, which prepares `/dev/mmcblk0p4` as
  an ext4 data partition mounted at `/mnt/data`, fetches the factory server's package feed and from it the
  initialization tarball for the incoming firmware's version (passed by the `COMMAND` via `--firmware`; the running
  `/etc/bos_version` predates Nix and has no tarball), stages it into `/mnt/data/nix.tmp`, and atomically promotes
  `nix.tmp/nix` to `nix`, discarding entries outside `nix/` — live rootfs files come from activation on first boot (see
  [`openwrt-tarball.md`](openwrt-tarball.md#initialization)); the profile activates on next boot. `init` exits
  successfully without changing an existing store only when it is fully initialized: the store is nonempty and its
  database and BMC profile are present. An incomplete `/mnt/data/nix` is not overwritten implicitly; recovery must pass
  `--wipe`.

Factory reset reinitializes the store from the offline initial store, so restoring factory state needs no network
access.

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
- Firmware-upgrade code paths scope every feed-linked server to the target firmware with explicit `--firmware`. Keep the
  Forge release server required; optional third-party servers may degrade when another source succeeds, and federation
  continues normally.
- Failure modes prefer keep-current over guess: stale packages, ambiguous priorities, and unavailable store paths all
  abort cleanly rather than silently substituting.
- Store-path realisation, hook execution, and activation are separate stages. Don't fold side effects into the
  resolution layer.
- On-device shell (the tarball `COMMAND`, activation scripts, the shell activator) must not rely on optional BusyBox
  features. Known trap: the device BusyBox lacks `CONFIG_FEATURE_TR_CLASSES`, so `tr` treats classes like `[:space:]` as
  literal characters — use explicit character sets.
