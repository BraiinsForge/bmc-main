---
name: bmc-deck-host-measurement
description: Use when a change needs before/after numbers from a real Deck — bmc-wasm-host memory (RssAnon, RssFile), startup timing, binary size — usually because a ticket's acceptance criteria name them. Triggers on phrases like "measure on the Deck", "RssAnon against master", "compare with master on the device", "baseline before deploying", "startup time on the Deck", or "is it slower on the device". Covers the before/after order, the full deploy that keeps host and widgets matched, the compositor restart, the validity checks, and the bundled rounds/summary scripts.
---

# Measuring bmc-wasm-host on a Deck, before and after

Lay out the whole procedure to the user **before anything is deployed**: which side is measured first, and how many
deploys and builds that costs. The device is usually already running something, so a deploy made "to see the change"
overwrites the baseline.

## What the device is running

Check before the first deploy. The host binary in the current profile tells you which tree it came from:

```sh
ssh root@<ip> 'ls -l $(readlink -f /run/current-profile/bin/bmc-wasm-host)'
```

The widgets may come from a different branch than the host. A host that lacks an import those widgets use rejects every
slot, and the screen stays on "Loading scene…".

## Deploy both sides the same way

Always deploy the full set, core plus every widget, so that the host and the widgets come from one tree:

```sh
nix run .#deck -- deploy --device <ip>
```

Never `--packages core` alone for a measurement.

The deck harness resolves `.#deck-packages.*` from the flake in the current directory. So master is deployed from a
detached worktree at `origin/master`, with the deploy run from inside it:

```sh
git worktree add --detach <path>/master-baseline origin/master
cd <path>/master-baseline && nix run .#deck -- deploy --device <ip>
```

The first deploy of a tree whose `Cargo.lock` differs from what was last built rebuilds that profile's dependency cache.
Tell the user it will take a while.

## A reading counts only when

- the host is fresh: `/etc/init.d/bmc-compositor restart`, because `bmc-openwrt` spawns `bmc-wasm-host` and the host has
  no init service of its own;
- the widgets are on screen, which the user confirms;
- `/var/log/bmc/run-bmc-wasm-host-sdk-v0.log` has no `slot rejected` line since the restart;
- the host has been idle for about a minute.

Anything measured otherwise is discarded, not reported with a caveat.

## Rounds and medians

One restart per side misleads: EGL setup alone varies by about 50 ms between restarts. Run five rounds per side with the
bundled script, then summarise:

```sh
bash .ai/skills/bmc-deck-host-measurement/rounds.sh <ip> master 5
bash .ai/skills/bmc-deck-host-measurement/rounds.sh <ip> branch 5
awk -f .ai/skills/bmc-deck-host-measurement/summarize.awk .tmp/deck-host-measurement/master-rounds.log
awk -f .ai/skills/bmc-deck-host-measurement/summarize.awk .tmp/deck-host-measurement/branch-rounds.log
```

Each round:

1. restarts the compositor;
2. waits for both widgets, aborting the run on a rejected slot or a 3-minute timeout;
3. waits 60 s idle;
4. records the host binary, `RssAnon`, `RssFile` and the startup-phase log lines.

The summary prints per-round spans and medians: EGL setup, GL → FemtoVG screen target, screen target → renderer
initialized, and start → both widgets loaded. Report the medians with their ranges, and say when the ranges overlap. The
round count, the 60 s idle and the two-widget wait suit a scene with two widgets; adjust them to the scene on screen.

## What the release host cannot tell you

- **Per-frame time.** The "wasm host render frame" log exists only with the host's `profiling` feature, which the debug
  and profiling deck profiles build. Measuring a frame means a `--profile profiling` deploy on both sides, whose release
  optimisation keeps the timings true to what ships; tell the user that cost first.
- **Per-mapping residency.** The device kernel has no `/proc/<pid>/smaps`; `RssFile` in `/proc/<pid>/status` is the
  total.
