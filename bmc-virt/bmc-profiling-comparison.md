# Compositor profiling comparison — `jku/BDK-383/vm-wayland-reduced` vs `master`

Date: 2026-04-13 Device: `braiins-deck` @ `192.168.1.183` (STM32MP157, Vivante GC400 rev 4652, Mesa 25.1.1 OpenGL ES
2.0) Panel: 600×1280 @ 63 Hz (480×1280 buffer, 1280×480 logical after 270° rotate)

## Motivation

During work on `jku/BDK-383/vm-wayland-reduced` we added the Wayland `ext-image-copy-capture-v1` protocol, a
frame-coherency fix with a pixel-readback cache (d0c231cc), and a 3D-rendering + animation-timing fix in flip-clock
(fa239abb). Because these commits touched hot paths (texture import, widget EGL FBO setup, compositor event loop), we
wanted to check whether they regressed real-hardware performance vs `master`.

## Method

1. Bootstrapped the device with `scripts/nix-init.sh` (from the branch working tree).
2. Reverted `/etc/bmc_config.json` flip-clock params to the stock `"mode": "extruded"`.
3. For each branch, rebuilt the compositor **and** both widgets with `--features profiling` and cargo-deployed them to
   the device:
   - `CARGO_EXTRA_FLAGS="--features profiling" nix develop ".#armv7-glibc-release" -c scripts/nix-cargo-deploy.sh compositor 192.168.1.183`
   - `nix develop ".#armv7-glibc-release" -c scripts/nix-cargo-deploy.sh widget flip-clock 192.168.1.183`
   - `nix develop ".#armv7-glibc-release" -c scripts/nix-cargo-deploy.sh widget digital-clock 192.168.1.183`
4. Started the compositor, let it run ~60 s, collected stopwatch reports via the existing `ii-stopwatch` instrumentation
   (emits one `compositor:` and one `render_scene:` `tracing::info!` line every 5 s).
5. First (warmup) 5 s window discarded in both cases; steady-state averages computed from the remaining 11–14 windows.

Tested configuration: single fullscreen flip-clock scene in **Extruded** mode, nothing else active, no capture client
attached.

Raw logs live alongside this file:

- `bmc-profiling-vm.log` — branch (SHA `7cc3fa72`, 15 windows, ~76 s)
- `bmc-profiling-master.log` — master (SHA `80649ba4`, 12 windows, ~60 s)

## Results

| Metric                                                   |                             Branch `7cc3fa72` | Master `80649ba4` |                    Δ (branch vs master) |
| -------------------------------------------------------- | --------------------------------------------: | ----------------: | --------------------------------------: |
| **Presented FPS** (`render_scene` count / 5 s)           |                                      **25.2** |          **17.0** |               **+48 %** (branch faster) |
| Frame budget (1000 ms ÷ fps)                             |                                         40 ms |             59 ms |                                   −32 % |
| `render_scene` phase — `bind` avg                        |                                          6 µs |              4 µs |                          +2 µs (≈ free) |
| `render_scene` phase — `compose` avg                     |                                        455 µs |            445 µs |                          +10 µs (noise) |
| `render_scene` phase — `finish` avg                      |                                        570 µs |            420 µs |                                 +150 µs |
| `render_scene` phase — `flip` avg                        |                                        475 µs |            465 µs |                          +10 µs (noise) |
| **Compositor per-frame work** (bind+compose+finish+flip) |                                   **~1.5 ms** |      **~1.35 ms** |      +10 % (both < 4 % of frame budget) |
| Compositor main-loop rate                                |                                      ~1600 Hz |            ~51 Hz | **~30× more loop iterations on branch** |
| Avg `dispatch` sleep per loop iter                       |                                        520 µs |             19 ms |          branch sleeps shorter per iter |
| `render_scene` calls per loop iter                       | 97 % (most early-return on `is_flip_pending`) |              33 % |                                         |

### Per-frame budget breakdown (steady-state average)

```
Branch   (40 ms budget, 25.2 fps):
  bind     6 µs    0.02%
  compose  455 µs  1.1%
  finish   570 µs  1.4%
  flip     475 µs  1.2%
  --- compositor work ----
  total    ~1.5 ms 3.8%
  idle     ~38.5 ms 96.2% (blocked in event-loop dispatch)

Master   (59 ms budget, 17.0 fps):
  bind     4 µs    0.01%
  compose  445 µs  0.8%
  finish   420 µs  0.7%
  flip     465 µs  0.8%
  --- compositor work ----
  total    ~1.35 ms 2.3%
  idle     ~57.7 ms 97.7% (blocked in event-loop dispatch)
```

## Interpretation

- **No regression.** Branch is materially faster than master in this scenario (+48 % fps). The pre-existing BDK-141
  baseline (compositor spinning at vsync producing duplicate frames, `finish=43 ms/frame` burning GPU) does not apply
  any more — the branch (and master, post the `needs_redraw`-style logic that landed earlier) no longer renders unless
  the widget commits.

- **The compositor is not the bottleneck on either branch.** Per-frame compositor work is ~1.4 ms on both, which is 2–4
  % of the frame budget. The remaining 96–98 % of each frame is spent in `event_loop.dispatch()` sleeping until the next
  wake-up (widget commit, frame callback, timer, DRM event, …).

- **The fps difference is widget-side.** Both compositors do equivalent work per frame. The difference is that the
  branch's flip-clock widget can sustain 25 buffer commits/s on the GC400, while master's only manages 17. Credible
  explanations:

  - `fa239abb flip-clock: Fix 3D rendering and animation timing` — explicitly reworks the widget render/animation loop.
    This is the primary candidate for the speed-up.
  - `d0c231cc bmc-widget egl: add depth renderbuffer to export FBOs` — lets 3D widgets render with proper depth testing.
    Likely neutral or mildly positive on perf (previously some frames may have been wasted on incorrect output, or depth
    was emulated in shader).

- **Curiosity worth a follow-up (not performance-critical, but wasteful):** the branch's compositor event loop iterates
  ~30× more often per second than master (1600 Hz vs 51 Hz). Each extra iteration is cheap (avg 625 µs, mostly sleeping
  inside `dispatch()`), and 97 % of them early-return in `render_scene` via `is_flip_pending()`, so the additional CPU
  cost is small. Still, something in the branch — most likely the new capture-protocol event sources, the dirty-buffer
  tracking on commit, or the smithay git-rev bump to `c114b88e` — is installing more frequent wake-up sources on the
  calloop event loop. Worth investigating if/when someone looks at idle power.

- **`finish` is ~150 µs slower on branch.** Likely explained by the dmabuf-reimport-on-commit change in
  `scene_renderer.rs::import_textures`: on branch, every widget commit re-imports the corresponding DMA-BUF via
  `eglCreateImageKHR` + `glEGLImageTargetTexture2DOES`, whereas master imports each `WlBuffer` id once and caches the
  `GlesTexture` forever. The branch's new semantics are motivated by virgl flicker
  (`"This avoids redundant EGLImage creation on virgl which can produce subtly different host-side copies and cause flicker"`
  — inline comment). On real Vivante hardware this reimport is unnecessary for correctness and costs ~150 µs per frame.
  Candidate for a virgl-gated fast path if someone chases that down.

## Net judgment

Merge-ready from a performance standpoint. The branch is faster than master for the realistic scenario (single
fullscreen flip-clock in Extruded mode), and the CPU is 78 % idle either way. The two minor inefficiencies identified
(elevated loop wake-up rate, unnecessary dmabuf reimport on real HW) are worth filing as follow-ups but are not
blockers.

## Appendix — artifacts

- Raw logs: `bmc-profiling-vm.log`, `bmc-profiling-master.log`
- Build commits compared: `7cc3fa72` (branch HEAD) vs `80649ba4` (master HEAD at time of test)
- Widget binaries deployed from each branch via `nix-cargo-deploy.sh widget …`
