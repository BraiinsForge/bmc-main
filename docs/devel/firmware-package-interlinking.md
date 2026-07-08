# Firmware and Package Upgrade Interlinking

A single `CheckForUpgrade` can surface two things at once: a firmware (BOS) upgrade and an application/widget package
upgrade. This document explains how the two relate at check time and at apply time, and the one invariant that
relationship imposes — the package probe is a pre-flight of the package work the firmware upgrade itself performs, so a
broken package layer cannot be routed around by offering firmware instead.

For the package-resolution algorithm, deferred activation, and the firmware tarball itself, see
[`upgrades.md`](upgrades.md) and [`openwrt-tarball.md`](openwrt-tarball.md). This document is only about how the check's
package probe relates to the package step a firmware upgrade performs.

## One check, two offers

`check_for_upgrade` probes firmware and packages independently, then decides what to offer:

- **`select_offer` makes firmware the single startable offer.** The firmware does not carry a package set of its own.
  What changes is what the servers' index offers: it can advertise different versions, or additional packages, keyed to
  the firmware being installed. Resolving packages in the current firmware's context first would therefore be redone —
  and possibly superseded — once the new firmware lands, so the firmware offer subsumes the package step rather than
  running beside it. The requested install names are carried **verbatim** onto the firmware offer at this point — no
  index resolution happens in `select_offer`.
- **The package preview is still returned for display.** Even when firmware wins the startable slot, the response
  carries the package changeset so the UI can show what the upgrade will bring. Only the size estimate is skipped when
  firmware is present; the full plan still runs.
- **`arbitrate` sets the disruption.** A firmware offer reboots the device; a packages-only offer only restarts the app.

## The firmware upgrade carries the package upgrade

Starting the firmware offer does not resolve or install packages inside `bmc`. `bmc` records the requested install names
to a handoff file (`record_pending_install` → `PendingInstall { install: names }`), stops the widgets, and hands the
image to `sysupgrade`. It forwards names, nothing more.

The `sysupgrade` sequence runs `bmc-nix-cli upgrade [--install-from <handoff>] --next-boot <bos-version>` **before the
flash**. That resolves the full package set plus the pending installs against the configured servers — the same servers
the check's probe used — and builds the next generation with deferred activation. The flash follows; after the reboot
`nix-activator` promotes the staged generation (see [`upgrades.md`](upgrades.md#deferred-activation---next-boot)).

The two resolutions run the same code against the same servers, but not necessarily the same index content: the
firmware-time run resolves in the target firmware's context, which can advertise different versions or more packages
than the current one. The install names still resolve through `resolve_new_package(merged, name, None, User)` —
byte-for-byte the call the probe runs via `resolve_installs`, feeding the same `compute_upgrade_plan` and the same
no-downgrade guard. So the check preview is an approximation of the firmware-time result: exact for the shared package
layer, but the target index may resolve some packages differently.

## The invariant: the probe is a pre-flight, not a bypass

The firmware-time package resolution runs against the same servers and the same on-device package layer as the probe,
before the flash. A failure rooted in that shared layer — no enabled servers, servers unreachable, the on-device profile
manifest unreadable — is not something flashing can cure; it recurs when `sysupgrade` runs `bmc-nix-cli upgrade`, before
any new image is live. (A failure that depends purely on index *content* — a name or version the current index lacks —
could in principle resolve against the target firmware's index, but the check has no way to know that at probe time, and
must not gamble a startable offer on it.)

Consequently a package-probe failure is **fatal to the whole `CheckForUpgrade`, even when a firmware upgrade was
discovered**. The check verifies packages against the only context it has — the current one — and will not advertise a
startable upgrade whose package step it could not dry-run. Firmware is not a recovery vehicle for a broken package
layer; the firmware upgrade still has to drive a package resolution through that same layer and those same servers
before it flashes. Do not add logic that mints a firmware offer while suppressing a package-probe failure: at best it
advertises an upgrade whose package step then fails mid-`sysupgrade`, before flashing, with the exact error the probe
already held.

The no-downgrade guard and the ownership/category rules that protect ordinary and explicit installs apply on this path
too, because it is the same planner.

## Timing: everything package-side is pre-flash

There is no post-flash package resolution. Realisation, planning, and the profile build all happen while the outgoing
BOS is still running; only activation is deferred to the next boot via `--next-boot`. What the target firmware changes
is the index the servers offer, not *when* resolution happens — the difference between the check preview and the
firmware-time result comes from index content, never from resolving after the flash. Flashing does not make a new index
reachable that resolution then consults.

## Implementation status

This branch implements the check/offer split, the verbatim carry of install names onto the firmware offer, the handoff
writer, and the far-side `bmc-nix-cli upgrade --install-from` consumer. The `sysupgrade` step that actually invokes the
CLI, and persisting the handoff across the flash — it currently lands on tmpfs at `/tmp/bmc-nix-pending-install.json`,
which `sysupgrade` does not preserve — are cross-repo follow-ups.

The firmware run resolves against the configured servers, not a tarball-baked pinned index. The pinned-index description
in [`upgrades.md`](upgrades.md#firmware-upgrades) and [`openwrt-tarball.md`](openwrt-tarball.md) predates that change
and is stale; those documents should be reconciled separately.

## Contributor note

Keep the check probe and the firmware-time package resolution on the same code path. If they ever diverge, the check
stops predicting the firmware run and the "packages were verified before we offered" guarantee silently breaks.
