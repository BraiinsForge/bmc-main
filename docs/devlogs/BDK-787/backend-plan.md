# Boser upgrade backend

## Scope and ownership

Boser owns firmware and package execution on managed miners. Deck retains its local BMC upgrade path. The backend
exposes Rust functions and an internal watch; external API, frontend and BMC display wiring are separate. Transport and
authentication remain undecided. A localhost exemption is not suitable behind the local reverse proxy.

Use bmc-upgrade for generic package discovery, typed apply errors and cached upgrade offers. Keep Boser's existing
firmware executor: the package trait is independent, so package support does not require refactoring firmware upgrades
into the shared library. Only boser-openwrt depends on bmc-upgrade and bmc-nix; core, Unix, Buildroot and mockup use
independent types. Mockup simulates package operations with explicit opt-in.

## Check, select and start

1. Acquire the existing system-task admission lock and invalidate the previous offer.
2. Check firmware, then packages when supported. Firmware takes precedence, with package changes alongside its preview.
   Otherwise offer packages alone, or no upgrade. Genuine package-probe failures fail the whole check.
3. Cache the selected operation and install intent under a single-use offer ID.
4. Claim the offer under admission and start an owned operation with a separate execution ID. Busy rejection must not
   consume the offer; unknown, replaced or consumed IDs fail.
5. Publish execution snapshots independently of the request or watch subscriber lifetime.

Starting a package offer uses its cached index rather than fetching it again. Apply still reads the installed profile
under its lock, so a cached offer is not an immutable execution plan after local changes. Package installation updates
the installed set and adds the requested names in one operation. The catalog contains generic names, versions,
categories, descriptions and complete metadata; BMC maps it to widgets.

An absent or unhealthy Nix installation permits firmware-only operation, but not an explicit package install; the
capability still reports the unhealthy state for diagnostics. A genuine probe failure fails an interactive check, while
an automatic run with firmware available falls back to the firmware offer. Initial Nix bootstrap remains hidden in the
incoming firmware's COMMAND.

Both BMC and Boser OpenWrt resolve package probes and catalogs against the offered firmware, falling back to the running
firmware when no firmware upgrade is offered.

## Automatic preparation and execution

Share ID-free preparation in bmc-upgrade. Interactive checks cache its result under an offer ID; automatic upgrades
prepare and execute directly under one admission guard. Automatic preparation must not publish an offer or borrow
another caller's install intent. A no-op or failed preparation must not replace an interactive offer. Once execution
starts, existing offers can become stale and must be rejected.

Keep BMC's GC/free-space preflight, execution-stream monitoring and retry policy. Boser OpenWrt uses the same shared
preparation through its core-independent adapter; mockup simulates it without a bmc-upgrade dependency. Buildroot
retains its existing firmware-only fallback. Boser's automatic trigger observes its own execution outcome, while workers
publish API watch state independently of the trigger and subscriber lifetime.

Verify shared selection, cache isolation, admission across preparation/start, cached-index execution, cancellation,
terminal outcome monitoring, and watch publication without consumers. Implement and review BMC first, publish its
reviewed dependency revision with authorization, then adapt and review Boser. Keep interactive protocols unchanged.

## Execution and reporting

The existing system lock excludes competing upgrades, reset and uninstall. Accepted work survives request loss; a proven
stopped failure releases admission, while uncertain failures retain it. Typed package errors preserve that distinction
without matching error strings. Package activation manages Nix-owned services, not firmware-owned Boser.

Process-I/O errors remain uncertain because existing runners can return before proving the child has been reaped.

Boser Unix supervision drains sysupgrade stdout and stderr concurrently through an injected line decoder. OpenWrt
decodes structured Nix progress and publishes phases and byte counts through the core execution reporter. Separate pipes
do not establish global output ordering. The ready/proceed handshake remains the firmware handoff gate. Firmware handoff
means rebooting, not verified reboot success. The watch retains a current snapshot, not an event history or durable
reboot journal.

Firmware carries requested package names through /dev/shm/bmc-nix-pending-install.json. Write it only when packages were
requested and remove a stale document otherwise, so a plain firmware upgrade hands over no file. Retain the handoff
while sysupgrade may run; clean it after confirmed failure while admission is still held. Keep existing upload/download
callers and their preparation semantics compatible.

## Implementation sequence

1. Add owned firmware supervision, independent execution state and OpenWrt progress decoding.
2. Preserve typed shared package errors and expose read-only installation inspection.
3. Add the Nix-independent package service, OpenWrt adapter and independent mock implementation.
4. Share offer arbitration/cache/claim and generic package discovery; retain widget presentation in BMC.
5. Connect internal check/start/watch functions and add an opt-in console and host E2E rig for verification.

Verify arbitration, failed checks, single-use offers, cached indexes, metadata preservation, busy admission,
request/watch loss, firmware handoff failures and dependency isolation. Use repository validation and focused tests;
build firmware images only in CI. See [device verification](verification.md) and [E2E usage](e2e-usage.md).

## Follow-ups

The internal Rust entry points are check_offers, list_installable_packages, start_offer and subscribe_execution. Network
adapters can connect to these without transferring execution ownership to a request.

- BDK-796 connects BMC to Boser's API and upgrade state. The [architecture presentation](architecture.html) shows the
  intended watch-to-display flow without choosing a wire protocol.
- BDK-820 adds Boser-owned GC and the mini-miner two-hour automatic-upgrade cadence, reusing scheduling code where
  appropriate and disabling competing BMC autoupgrades. Preserve Deck behavior.
- Firmware-library sharing remains a later refactor. Display-removal MR !2522 is not integrated here.
