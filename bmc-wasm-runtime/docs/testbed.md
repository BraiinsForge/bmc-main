# WASM widget testbed

The desktop preview: every viewport of every deck platform live at once over one pannable canvas, driving the same
`WasmWidgetRuntime` the device does.

Capture and baselines: [`regression-testing.md`](regression-testing.md).

## Running it

```bash
just wasm::testbed <widget>        # debug, dev blob cache, hot reload on the widget's source
just wasm::dev <widget>            # the same, both binaries prebuilt first
just wasm::run <widget>            # release-built widget, once
just wasm::record <widget> [<platform>:<viewport>] [name]
just wasm::profile <widget>
```

`ARGS` reaches the testbed; `--help` lists its flags.

It rebuilds the *widget* on source change, not itself — a change under `src/bin/testbed/` needs a restart.

## Targets

A `(platform, viewport)` pair, written `<platform>:<viewport>`. On disk it is `<platform>/<viewport>/` — never the
colon, which is Windows-illegal and the testbed runs on macOS.

Platforms and their viewports are `PLATFORMS` in [`src/platform_catalog.rs`](../src/platform_catalog.rs). It holds only
preview-specific facts; size, shape, DPI, slot grid and LED strip come from
`bmc_platform::HardwareProfile::for_product`, as they do on the device.

## What the window shows

Docked toolbar, Params / System / Credentials panels, status bar, and one floating window per platform over the canvas.

- **Display states** — a window stacks the states the display can be in, since they are alternatives. BMC100 has
  *Fullscreen* and *Slots*; other platforms have one. All viewports stay live in every state.
- **LED strips** — one diffuser per view, not per device: a shared strip would hide what the other views drive. The
  enclosure's margin runs on all four sides and doubles as the casing under each strip.
- **Zoom is the canvas's** — it scales windows and positions; chrome is drawn at natural size. `Fit` binary-searches the
  zoom whose packing fills the canvas. Touch coordinates divide by it, so the guest is unaffected.
- **Timings** — per-view frame cost and slip against the deadline the frame was armed for. Host FPS is the status bar's.
- **Debug** — outlines the widget's render tree and enables egui's inspector. The mock *paints* its enclosure, views and
  strips rather than allocating them, so egui cannot see them; they outline themselves under the same toggle.

## Recording

A runtime mode, not a launch flag. `RecordingMode` is one state machine — `Off` / `Choosing` / `Recording` — and each
phase carries what its exit must put back: the canvas it replaced, and mid-take the viewport's KV directory.

Record opens the choosing phase; every recordable target lands on the canvas wearing an overlay that is the button, and
one with an existing fixture asks again. Choosing pins the canvas to that platform, wipes and reseeds its KV store, and
rebuilds its views inline — the recorder's hit-test and event drain are synchronous.

Save writes `fixtures/<dataset>.jsonl.gz`, updates `[fixtures.<dataset>]` in the widget's `capture/config.toml`, then
unwinds, leaving the outcome on screen as a notice. A failed write keeps the take for a retry. Cancel discards
everything, fetch buffer included. `--record=<target>` is sugar for entering the mode at startup.

## How a view runs

Each view owns its runtime and GL context on its own thread; the main thread composites only.

- **Contexts** — a shared child context per view with a 1×1 pbuffer current on the worker, which builds renderer and
  runtime in place. Nothing `!Send` crosses a thread boundary. EGL, GLX and WGL supply the pbuffer; CGL does not.
- **Transport** — `Render::{Inline, Threaded}` inside `DeviceView`. Both drive the identical tick, which is what makes
  the fallback cheap.
- **Handoff** — the worker fences, the compositor waits server-side, and each frame is blitted into one of two present
  targets in turn. Retargeting the renderer does not work: femtovg binds the framebuffer it was constructed against
  whenever it flushes, so alternating two framebuffers blanks every second frame.
- **Teardown** — free the texture, stop painting, send `Close`. `shutdown` hands back a handle the app polls rather than
  joining, so a fetch in flight cannot stall the UI; it warns after 10 s.
- **Fallback** — `--inline-views`, also chosen per view when a shared context or pbuffer fails. The log says which.

Recording and profiling pin views inline: both question the render they just drove.

**macOS never threads.** glutin's CGL backend has no pbuffer surfaces (`pbuffers are not supported with CGL`), so every
view fails the shared-context check at startup and falls back inline, one warning apiece. Threading is therefore a Linux
path in practice, and the fence discipline built for Apple's GL 4.1 has nothing reaching it. The cost is that all of a
platform's runtimes tick on the UI thread, which a fan will tell you about.

## Constraints that decide the design

- **wasmi** (1.0.x) makes host calls straight-line synchronous and returns texture ids from inside the interpreter. No
  display-list seam, so wasm and GL move together or not at all.
- `WasmWidgetRuntime` owns no renderer. The caller owns `FemtoVgRenderer` and parks a `NonNull<dyn Renderer>` per scope;
  that pointer plus one FBO id is everything shared.
- `FemtoVgRenderer` is `!Send` (femtovg holds `Rc<glow::Context>`) and so is the runtime. Neither is pinned to the
  *main* thread — the invariant is one thread with GL current for the pair's lifetime.
- Cross-thread payloads are already `Send`: snapshots, touch events, the LED sender, `FrameTimings`, and the fetch
  observer's `Arc<Mutex<_>>`.
- The only process-wide shared object is the mDNS hub, already built for multi-runtime use.

## Alternatives already ruled out

- `egui::Scene` cannot host the canvas — its transform applies only to its own sublayer, so it cannot carry windows.
  Real windows with `.constrain(false)` plus a layer translation get native input routing and crisp text.
- Zoom as an egui layer transform resamples its own title text, because egui rasterises glyphs without knowing about the
  transform. Scaling the device geometry instead is what keeps the chrome sharp.
- `binpack2d`'s MaxRects arranges: one-shot, egui-independent, packing into a canvas of known size. The alternatives
  (`guillotiere`, `etagere`) are dynamic-atlas allocators, growing a bin instead of filling one.
- Wake reasons and duty cycle were sketched beside slip and not built — they need a per-view ring buffer nothing reads.

## Working on it

**`just`, never raw `cargo`.** The dev shell ships its own rustc, a bare `cargo` uses rustup's, and both write the same
`target/`. Interleaving them surfaces much later as `can't find crate for bmc_render_macros` while building the
`bmc-wasm-sdk` dev-dependency. Repair: `cargo clean -p bmc-wasm-sdk -p bmc-render-macros`.

**`just validate` does not run this binary's tests** — they are behind the `testbed` feature, so clippy compiles them
and nothing executes them. Finish with `just check nextest` and confirm the test name is in its output.

**GL tools and tests** need the machine's EGL or the `ci` profile in `nix/profiles.nix`. Without it a capture run fails
at `EGL device enumeration not supported`, and `mdns_coexistence` / `multi_runtime_teardown` fail locally by design.

**params-demo prints `display_info()` on screen** — the cheapest check that geometry reaches the guest, which must read
`display 1280x480 rectangular dpi=217`. For a general smoke run prefer hello-widget: it animates itself, so a stalled
view shows without touching anything.

**hello-widget's own tests never run:** `SystemTime::{now, local, utc}` are `wasm32`-gated, so it does not compile
natively and its `#[test]`s are never built. Round-layout coverage rests on the baselines. BDK-704 lifts the gate.

## Left open

Zooming past 1:1 — `Fit` only shrinks and nothing else zooms in. The view side was built and dropped: it needs a zoom
control first.
