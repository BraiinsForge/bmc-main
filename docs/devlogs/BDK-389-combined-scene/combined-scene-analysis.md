# Combined-scene frame pacing

**Ticket**: BDK-389 · **Branch**: `jku/BDK-389/combined-scene`

## Problem

A single flip-clock widget placed in a combined-scene medium slot submitted buffers ~2.6× faster than the same widget
placed in a fullscreen scene. Per-widget CPU scaled with the submission rate, violating the BDK-389 acceptance
criterion: *"CPU load of each widget instance stays the same when in combined scene configuration as in the case of full
widget size."*

Baseline measurement on the Deck (Braiins Deck, ARMv7, Vivante GC400, DSI 600×1280 @ 63 Hz):

| Scenario                |  Buffer-attach rate | Compositor CPU | Widget CPU | Total |
| ----------------------- | ------------------: | -------------: | ---------: | ----: |
| fullscreen, 1 widget    |               52 Hz |           2.6% |       6.9% |  9.5% |
| combined, 3 flip-clocks | 134 Hz (per widget) |           7.7% |   6.3% avg | 14.0% |

## Diagnosis

Two independent bugs compounded:

1. **No compositor-side rate limit on frame callbacks.** `wl_surface.frame` callbacks fired on every page flip. On cheap
   surfaces (medium slot, ~150k px) the render fits in one vblank so the compositor flipped at the display's 63 Hz,
   doubling the callback rate vs the render-cost-limited fullscreen scene (~30 Hz). Nothing in the compositor capped
   this.
2. **The widget wasn't actually waiting on frame callbacks.** The flip-clock render loop armed `needs_render` on any
   poll wake-up. `wl_buffer.release` events (emitted whenever the compositor released the previous widget buffer)
   therefore kicked a fresh render, so the loop was render- cost-bound, not callback-bound. Under combined's cheaper
   render, it ran faster.

Each bug individually would be hidden by the other — fixing just the compositor had no effect because the widget ignored
the pacing; fixing just the widget left it paced at the display rate (double what we wanted).

## Solution

Architecture lives in two landed commits:

- [`widgets: flip-clock: Drive render loop with a phase state machine`](../../../widgets/flip-clock/src/wayland.rs)
  replaces `needs_render` / `take_render_requested` / `mark_needs_render` / animating-derived-timeout booleans with an
  explicit `LoopPhase { RenderPending, WaitingForCallback, WaitingForIdleTimeout }`. Each variant maps to exactly one
  `poll_dispatch` timeout (`0` / `-1` / `ms to next second`). Phase transitions come from three inputs only: frame
  callback arrived, idle timeout expired, timezone update. Non-callback events (e.g. `wl_buffer.release`) no longer
  touch the phase, closing the feedback loop.
- [`bmc-openwrt: Pace per-widget frame callbacks via a single tick`](../../../bmc-openwrt/src/compositor/egl_compositor.rs)
  adds a per-widget 32 ms rate cap on `CompositorState::send_frame_callbacks_for_presented_widgets`, and consolidates
  three previously-competing call sites (post-render firing, HW-only retry timer, headless-only timer) into one
  always-on 16 ms calloop tick.

A new `PollOutcome { Events, Timeout }` on the `WidgetSurface::poll_dispatch` trait carries out the
event-vs-timeout distinction the widget needs. Before, the `Result<bool>` return (true on normal, false on EAGAIN)
couldn't tell "the idle timeout expired" from "an unrelated event arrived" — that ambiguity was the entire widget bug.

## Results

![Rate and CPU across pacing stages](./rate-and-cpu.png)

At HEAD:

| Scenario                      | Buffer-attach rate | Compositor CPU | Widget CPU | Total |
| ----------------------------- | -----------------: | -------------: | ---------: | ----: |
| fullscreen, 1 flip-clock      |              12 Hz |           2.4% |       2.0% |  4.3% |
| combined, 1 medium flip-clock |              12 Hz |           2.4% |       2.1% |  4.5% |

- Submission rate is identical between scene kinds (ratio 1.07×, within measurement noise).
- Both scenarios land at ~12 Hz — the flip-clock's actual content rate (10 animation frames during each 350 ms digit
  flip + 1 idle frame per second). The widget now renders only when it has content to show, not at whatever rate the
  compositor happens to flip at.
- Absolute CPU drops ~55% in fullscreen and ~68% in combined versus baseline, because the previous runaway submission
  rate was pure waste.
- Compositor CPU is flat between scenes (~2.4%), where previously it scaled with the number of renders.

## Takeaways

- **Policy belongs at the single chokepoint**, not replicated in every client. We tried a widget-side cap first and it
  was bypassed by the busy-loop bug; moving the cap to the compositor (one place, one policy) immediately stopped being
  relitigable per-widget.
- **Clients must respect `wl_surface.frame`**, and the render loop needs to be able to distinguish a real idle-timeout
  wake from an arbitrary event wake. `poll(2)` already tells you (return value `0` = timeout) — we were just discarding
  that signal at the trait boundary.
- **State machines survive reviewers, boolean piles don't.** The widget loop was four booleans interacting, and the
  busy-loop feedback bug hid in the interaction for weeks. Making the phase explicit made the bug visible and the fix
  structural.
- **Don't let the retry mechanism depend on the activity it's pacing.** The intermediate "fire callbacks on each DRM
  vblank" attempt deadlocked because vblanks only occur after page flips, and page flips only occur if the widget
  submits, and the widget wouldn't submit without a callback. A self-ticking timer has no such dependency cycle.

## Open follow-ups (out of scope for BDK-389)

These items surfaced during the investigation but aren't addressed here:

- `DeckWidgetSurfaceClient::take_size_changed()` still returns `false` unconditionally. Widgets start at the size passed
  via `DECK_WIDTH`/`DECK_HEIGHT` env vars, so this only matters if the slot changes size after startup, which doesn't
  happen today.
- `render_scene()` composites every visible widget on every render even when only one committed new content. The damage
  infrastructure is plumbed (`OutputDamage::Widgets(set)`) but the renderer doesn't consume it yet.
- Widgets occasionally show the first rendered frame without animation state initialised; cosmetic, not a correctness
  issue.

## How the data was collected

Reproduction for anyone rerunning the comparison:

1. Deploy from `jku/BDK-389/combined-scene` with profiling:
   ```
   CARGO_EXTRA_FLAGS="--features profiling" \
     nix develop ".#armv7-glibc-release" -c scripts/nix-cargo-deploy.sh compositor <device-ip>
   nix develop ".#armv7-glibc-release" -c scripts/nix-cargo-deploy.sh widget flip-clock <device-ip>
   ```
2. Write a fullscreen or combined config to `/etc/bmc_config.json` (example configs in
   `bmc-virt/data/bmc_config_combined_flip_clocks.json`).
3. Launch the compositor with `RUST_LOG=debug,bmc_openwrt::compositor=debug` and redirect to `/tmp/bmc-openwrt.log`.
   Widget traces land in `/var/log/bmc/flip-clock-widget.log` on the device.
4. Submission rate: count `Buffer attached for widget` lines per 30 s.
5. CPU: `top -b -n 20 -d 1 | grep -E 'bmc-openwrt|bmc-widget-flip'`, then average the 7th column (CPU%).

The plot is generated by a Python script kept with the devlog repository clone during the investigation; it's not
checked in because there's nothing specific to rerun against now that the landed state is the only interesting data
point. Re-running the two commands above and plotting `buffer-attach rate` + stacked compositor/widget CPU would
reproduce the right-hand bars.
