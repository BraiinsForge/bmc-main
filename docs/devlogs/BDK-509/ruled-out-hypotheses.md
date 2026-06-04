# BDK-501 ruled-out hypotheses

Updated: 2026-06-04.

Repro baseline:

- current device target: SSH config host `home-deck.rpi.wg` over VPN
- previous LAN target: `10.0.0.129`
- serial fallback was `/dev/ttyUSB0` at 115200 baud, but the device is not currently connected over serial
- config: `docs/devlogs/BDK-501/bmc_config_mining_info_wiggle_repro.json`
- command shape: start compositor with `BMC_DIAG_SWIPE_REPLAY_FIFO=/run/bmc-swipe-replay` and
  `bmc-openwrt --hardware-profile=BFM100`
- wiggle sequence:
  - `wiggle 10 8 6`
  - `wiggle 10 8 8`
  - `wiggle 10 6 10`
- verification target: 10 restarted BFM100 wiggle runs without new `etnaviv` MMU fault or GPU recovery lines

Important constraint:

- do not work around the issue by hiding or disabling swipe-neighbor scene rendering. The device must be able to render
  scenes being swiped to.

## Ruled Out

### Fully offscreen neighbor culling as a complete fix

Status: ruled out as a solution.

Reason:

- hiding/disabling inactive or neighbor scenes could avoid the fault by avoiding the rendering pressure that triggers
  it, but it violates the requirement that swipe target scenes remain renderable.
- this must not be used as a BDK-501 fix.

### Negative/offscreen destination geometry as the sole root cause

Status: ruled out as sufficient.

Tested change:

- clipped rendered widget damage to the visible output area while keeping the full destination rectangle for the widget,
  so partially visible swipe neighbors still render.

Result:

- local `bmc-openwrt` tests passed.
- deployed core + `widget-mining-info` to profile `186-link`.
- 10-run verifier failed on run 2 with the same style of MMU fault:
  - `MMU fault status 0x00000002`
  - `MMU 0 fault addr 0xffffe9c0`
  - `recover hung GPU!`

Conclusion:

- offscreen/negative geometry may still be involved in the trigger surface, but clipping alone does not fix BDK-501.

### Immediate host render-target destruction on lifecycle demotion as the complete cause

Status: ruled out as sufficient.

Tested change:

- changed `bmc-wasm-host` lifecycle handling to retain a render target after demotion to `Prepared`/`Dormant` while any
  presented `wl_buffer` slot is still unreleased.
- reused retained render targets when the scene re-entered before releases arrived.
- added tests for retained target reuse and pending-release destruction deferral.

Result:

- `cargo test -p bmc-wasm-host` passed.
- deployed core + `widget-mining-info` to profile `187-link`.
- verifier failed on run 1.
- logs showed `render_allocs=3 render_releases=0`, so the tested run had no render-target release/destruction churn.

Conclusion:

- freeing host render targets during rapid `Entering`/`Prepared` churn is not the complete cause of the MMU fault.

### Host-side GL producer completion alone

Status: ruled out as sufficient.

Tested change:

- replaced host-side nonblocking `glFlush` before DMA-BUF export/Wayland commit with `glFinish`.

Result:

- `cargo test -p bmc-wasm-host` passed.
- deployed core + `widget-mining-info` to profile `188-link`.
- verifier passed run 1 but failed on run 2:
  - new fault at `MMU 0 fault addr 0xffffe9c0`
  - `recover hung GPU!`

Conclusion:

- missing completion of the host producer GL queue is not sufficient by itself to explain BDK-501.

### Compositor-side GL completion alone

Status: ruled out as sufficient.

Tested change:

- changed compositor `EglContext::finish_rendering` from `glFlush` to `glFinish`.
- this was tested together with the host-side `glFinish` diagnostic build.

Result:

- `cargo test -p bmc-openwrt` passed with required `LD_LIBRARY_PATH`.
- deployed core + `widget-mining-info` to profile `189-link`.
- verifier passed run 1 but failed on run 2.
- run 2 faulted mid-wiggle, before completing all wiggle steps.

Conclusion:

- simple producer/compositor GL queue completion does not eliminate the MMU fault.
- explicit native fence synchronization may still be worth investigating, but plain `glFinish` did not solve it.

### Host-side `wl_buffer.destroy` as the immediate trigger in the collected failures

Status: ruled out as the immediate trigger for the collected failures.

Evidence:

- in the host-retain and `glFinish` failure logs, `Dropped destroyed buffer ...` messages occurred during verifier
  shutdown, after the new MMU fault had already been detected.
- in the `glFinish` failure, new DMA-BUF imports and invalidated-buffer releases occurred around the fault window, while
  destroyed-buffer logs appeared later during shutdown.
- in the cropped-destination failure on profile `191-link`, destroyed-buffer logs again appeared during verifier
  shutdown at `2026-06-03T21:51:24`, after the verifier had already detected the new fault from the completed run.
- in the cleanup-before-release failure on profile `192-link`, destroyed-buffer logs appeared during verifier shutdown
  at `2026-06-03T21:59:06`, after the fault had already been counted.
- in the coredump baseline failure on profile `212-link`, the new kernel fault landed around `2026-06-04T07:26:55.29Z`;
  host `Dropped destroyed buffer` count was 0 in the fault window, and the compositor `Dropped destroyed buffer` log
  appeared later at `2026-06-04T07:26:57.513Z`.

Conclusion:

- buffer destroys remain worth auditing for correctness, but they were not the immediate trigger in the collected
  failure runs.

Open destroy-order concern:

- `EglRenderTargetFactory::destroy` currently calls `target.buffers.destroy_all(egl)` before
  `surface.destroy_minted_wl_buffer(...)`.
- that order frees EGL/GBM export buffers before sending `wl_buffer.destroy`; it is suspicious for shutdown or teardown
  races even though it did not line up with the MMU fault window in the current logs.

### Compositor DMA-BUF import as a necessary trigger

Status: ruled out as necessary.

Evidence:

- earlier host phase captures showed the new MMU fault can occur before the entering-neighbor frame is
  exported/committed and before the compositor imports that frame.
- the user also reproduced a failure with no compositor import of the relevant host buffers before the fault.
- in that scenario the compositor still rendered an empty scene, so it could still submit GPU work and force scheduler
  or MMU-context activity without importing the host DMA-BUF.
- kernel source inspection supports this distinction:
  - etnaviv uses per-file scheduling/MMU contexts for separate GPU clients.
  - etnaviv command-buffer queueing handles `switch_mmu_context` and emits MMU flush/stall commands.
  - an empty compositor render is still a compositor GPU job through a different DRM file/context.

Conclusion:

- importing the host buffer is not a necessary condition for BDK-501.
- the remaining cross-process suspect is GPU job interleaving/MMU-context switching between host and compositor, not the
  import operation specifically.

### DMA-BUF acquire-fence synchronization as a complete fix

Status: ruled out as sufficient.

Tested change:

- implemented the old Linux explicit synchronization protocol using acquire fence FDs.
- exported native EGL fences from the widget/host producer path.
- sent those fences with the surface commit.
- waited for the acquire fences in the compositor before dirty DMA-BUF import.
- disabled the previous cross-process `flock` workaround with `BMC_GPU_RENDER_LOCK_PATH=""`.

Result:

- deployed core and `widget-mining-info` to generation `226-link`.
- reproduced with `--hardware-profile=BFM100` and the Mining Info wiggle config.
- logs showed both producer-side native fence export and compositor-side acquire-fence waits.
- completed 7 runs before the device entered a pingable/no-SSH stuck state.
- all 7 completed runs produced new MMU fault / GPU recovery lines.

Conclusion:

- waiting on the producer's DMA-BUF acquire fence is not enough to prevent BDK-501.
- this only orders compositor use of a particular committed buffer after the producer has finished rendering that
  buffer.
- it does not serialize unrelated GPU jobs, cross-process GPU submission, or etnaviv MMU-context switches between
  `bmc-wasm-host` and `bmc-openwrt`.
- this is consistent with earlier evidence that the fault can occur before the relevant host buffer is imported by the
  compositor.

### Wayland `ObjectId` cache-key collision across clients

Status: ruled out.

Reason:

- logs showed repeated display strings like `ObjectId(wl_buffer@11)` for different widget clients, which looked like a
  possible texture-cache key collision.
- Smithay `wayland-backend` server `ObjectId` implements equality/hash with the backend's inner object identity, not
  just the protocol number. Its docs explicitly state that object IDs compare equal only for the same protocol object
  from the same client.

Conclusion:

- using `ObjectId` as the texture-cache key is not a cross-client collision bug, despite the abbreviated display output.

## Still Active

### Correct output-space damage coordinates for `render_texture_from_to`

Status: ruled out as sufficient.

Finding:

- Smithay `Frame::render_texture_from_to` expects damage in physical output coordinates.
- the first clipping experiment accidentally changed the render call to pass destination-local damage.

Tested change:

- correct the compositor to pass visible output-space damage while still keeping the full destination rectangle, so
  swipe-neighbor rendering remains enabled.

Result:

- focused `cargo test -p bmc-openwrt scene_renderer` passed.
- deployed core + `widget-mining-info` to profile `190-link`.
- verifier failed on run 1 with new MMU fault lines.
- run summary: `wiggle_steps=48 wiggle_end=2 render_allocs=3 render_releases=0`.

Conclusion:

- passing output-space damage is the correct API usage and should be kept, but it does not fix BDK-501 by itself.

### Negative/offscreen `render_texture_from_to` destination coordinates as the complete cause

Status: ruled out as sufficient.

Tested change:

- cropped every widget render to the visible output rectangle before calling Smithay `render_texture_from_to`.
- cropped the source rectangle through the inverse scanout transform so visible swipe-neighbor pixels still render
  instead of stretching or hiding the neighbor scene.
- passed the cropped visible destination as the GLES destination, avoiding negative/offscreen destination coordinates.
- kept neighbor rendering enabled.

Result:

- focused `cargo test -p bmc-openwrt scene_renderer` passed.
- deployed core + `widget-mining-info` to profile `191-link`.
- verifier failed on run 2 with new MMU fault lines:
  - `MMU fault status 0x00000002`
  - `MMU 0 fault addr 0xfffff2c0`
  - `recover hung GPU!`
- run summary: `wiggle_steps=60 wiggle_end=3 render_allocs=3 render_releases=0`.
- failure bundle: `/tmp/claude-1001/BDK-501-verify-fail-cropped-dst-memory-20260603T2151/`.

Conclusion:

- negative/offscreen destination coordinates are not the complete cause of BDK-501.
- the crop may still be useful as API hygiene, but it does not eliminate the MMU fault.

### Sampled system memory or CMA exhaustion as the immediate explanation

Status: ruled out for the sampled profile `191-link` and `192-link` failures.

Evidence:

- the verifier sampled `/proc/meminfo` before startup, after startup, and between wiggle commands.
- in failing run 2:
  - before startup: `MemAvailable=168216 kB`, `CmaFree=63088 kB`
  - after startup: `MemAvailable=116464 kB`, `CmaFree=41756 kB`
  - after wiggle 1: `MemAvailable=111836 kB`, `CmaFree=40888 kB`
  - after wiggle 2: `MemAvailable=103984 kB`, `CmaFree=72168 kB`
  - after wiggle 3: `MemAvailable=102968 kB`, `CmaFree=72176 kB`
- post-failure sample after verifier shutdown had `MemAvailable=168608 kB`, `CmaFree=92588 kB`.
- in failing run 1 on profile `192-link`:
  - before startup: `MemAvailable=168060 kB`, `CmaFree=48596 kB`
  - after startup: `MemAvailable=116340 kB`, `CmaFree=18564 kB`
  - after wiggle 1: `MemAvailable=114428 kB`, `CmaFree=18428 kB`
  - after wiggle 2: `MemAvailable=111308 kB`, `CmaFree=17180 kB`
  - after wiggle 3: `MemAvailable=109312 kB`, `CmaFree=32656 kB`
- the default cross-process GPU render lock passed 10 restarted BFM100 wiggle runs while sampling much lower CMA in at
  least one non-failing run:
  - run 1 after startup: `MemAvailable=117876 kB`, `CmaFree=5916 kB`
  - run 1 before wiggle 1: `MemAvailable=117804 kB`, `CmaFree=5892 kB`
  - final fault counter stayed unchanged at `110`

Conclusion:

- the reproduced fault is not explained by simple sustained system-memory exhaustion or sustained CMA exhaustion in this
  run.
- this does not fully exclude short-lived allocation pressure or CMA fragmentation below the sampling interval.

### Immediate compositor release after texture invalidation as the complete cause

Status: ruled out as sufficient.

Tested change:

- after invalidating cached widget textures, explicitly called Smithay/GLES `cleanup_texture_cache()`.
- called compositor `finish_rendering()` before sending `wl_buffer.release()` for the invalidated buffers.
- kept neighbor rendering enabled.

Result:

- focused `cargo test -p bmc-openwrt scene_renderer` passed.
- deployed core + `widget-mining-info` to profile `192-link`.
- verifier failed on run 1 with new MMU fault lines:
  - `MMU fault status 0x00000002`
  - `MMU 0 fault addr 0xffffe9c0`
  - `MMU 0 fault addr 0xfffff2c0`
  - `recover hung GPU!`
- run summary: `wiggle_steps=43 wiggle_end=1 render_allocs=3 render_releases=0`.
- failure bundle: `/tmp/claude-1001/BDK-501-verify-fail-cleanup-before-release-20260603T2159/`.

Conclusion:

- simply draining compositor texture cleanup before buffer release does not eliminate the MMU fault.
- the broader buffer-lifecycle path remains suspicious because the run still had DMA-BUF imports and buffer churn near
  the fault window, but this narrower cleanup-before-release ordering is not sufficient.

### Host slot present/release churn immediately before the fault

Status: ruled out for the instrumented profile `193-link` failure.

Evidence:

- added host-side debug logs for:
  - `wl_buffer present commit` with `slot_idx` and Wayland buffer id
  - `wl_buffer release slots drained`
- deployed core + `widget-mining-info` to profile `193-link`.
- verifier failed on run 1:
  - `wiggle_steps=32 wiggle_end=1 render_allocs=3 render_releases=0`
  - new MMU fault addresses included `0xffffe9c0` and `0x000003c0`
- relevant host log sequence:
  - active widget committed `wl_buffer@11` slot 0 at `22:04:24.678`
  - active widget committed `wl_buffer@13` slot 1 at `22:04:24.866`
  - active widget drained release for slot 0 at `22:04:24.879`
  - active widget committed `wl_buffer@11` slot 0 at `22:04:34.955`
  - active widget drained release for slot 1 at `22:04:34.964`
  - entering neighbor committed its first buffers at `22:04:50.260` and `22:04:50.322`
- kernel fault uptime maps to roughly the `22:04:49-22:04:50` wall-clock window for this boot, before the entering
  neighbor's first host buffer commits.
- failure bundle: `/tmp/claude-1001/BDK-501-verify-fail-slot-instrumentation-20260603T2204/`.

Conclusion:

- the captured failure is not explained by the host immediately reusing or committing a released slot just before the
  MMU fault.
- the compositor can fault while wiggling existing/imported textures before new entering-neighbor host commits arrive.
- host runtime remains worth auditing, but this specific slot present/release timing was not the immediate trigger in
  the captured run.

### Clearing the compositor flip gate only on DRM page-flip events

Status: ruled out as sufficient.

Hypothesis:

- `DrmOutput::on_vblank` previously cleared `flip_pending`, and the DRM event handler called it for both `Vblank` and
  `PageFlip`.
- if ordinary vblank events arrived before the page flip completed, the compositor could render into the previous front
  buffer while it was still being scanned out.

Tested change:

- split DRM output handling so `on_vblank` only updates the last vblank timestamp.
- added `on_page_flip` and clear `flip_pending` only for `DrmEvent::PageFlip`.
- kept neighbor rendering enabled.

Result:

- focused `cargo test -p bmc-openwrt scene_renderer` passed.
- deployed core + `widget-mining-info` to profile `194-link`.
- verifier failed on run 1:
  - `faults_before=50`
  - `faults_after=53`
  - `wiggle_steps=48 wiggle_end=2 render_allocs=3 render_releases=0`
  - new MMU fault addresses included the recurring `0xffffe9c0`.
- sampled memory/CMA did not show sustained exhaustion in the failing run:
  - before startup: `MemAvailable=168156 kB`, `CmaFree=52336 kB`
  - after startup: `MemAvailable=116544 kB`, `CmaFree=25112 kB`
  - before wiggle 1: `MemAvailable=116432 kB`, `CmaFree=25112 kB`
  - after wiggle 1: `MemAvailable=114492 kB`, `CmaFree=24912 kB`
  - after wiggle 2: `MemAvailable=107380 kB`, `CmaFree=31684 kB`
  - after wiggle 3: `MemAvailable=102188 kB`, `CmaFree=61296 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-pageflip-gate-20260604T0000/`.

Buffer destroy observation from this run:

- no host-side `wl_buffer destroy` lines were present before the fault window in `bmc-wasm-host.log`.
- compositor-side destroyed-buffer drops appeared during verifier shutdown at `22:13:30`, after the wiggles and after
  the verifier had already detected new kernel fault lines.
- compositor-side `wl_buffer.release` still occurred during wiggle processing after texture invalidation, so buffer
  release/lifetime remains worth auditing, but literal `wl_buffer.destroy` timing was not the immediate trigger in this
  captured run.

Conclusion:

- prematurely clearing `flip_pending` on vblank is not the complete cause of BDK-501.
- this event split may still be correct compositor hygiene, but it does not eliminate the MMU fault.

### Releasing invalidated widget buffers while a page flip is pending

Status: ruled out as sufficient.

Hypothesis:

- the compositor was invalidating widget textures and sending `wl_buffer.release` before checking whether a previous
  page flip was still pending.
- even after `glFinish`, this could have returned DMA-BUF-backed widget buffers to the host while the display pipeline
  still had work outstanding.

Tested change:

- kept invalidated buffers and pending releases queued while `renderer.output().is_flip_pending()` was true.
- only ran texture cleanup and sent queued `wl_buffer.release` after the page flip completed.
- kept neighbor rendering enabled.

Result:

- focused `cargo test -p bmc-openwrt scene_renderer` passed.
- deployed core + `widget-mining-info` to profile `195-link`.
- verifier passed run 1 and failed on run 2:
  - `faults_before=53`
  - `faults_after=59`
  - `wiggle_steps=43 wiggle_end=1 render_allocs=3 render_releases=0`
  - new MMU fault addresses included `0xffffe9c0` and `0xfffff2c0`.
- sampled memory/CMA in the failing run:
  - before startup: `MemAvailable=167992 kB`, `CmaFree=77700 kB`
  - after startup: `MemAvailable=116612 kB`, `CmaFree=44600 kB`
  - before wiggle 1: `MemAvailable=116512 kB`, `CmaFree=44588 kB`
  - after wiggle 1: `MemAvailable=114756 kB`, `CmaFree=44456 kB`
  - after wiggle 2: `MemAvailable=111456 kB`, `CmaFree=43156 kB`
  - after wiggle 3: `MemAvailable=109416 kB`, `CmaFree=43132 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-defer-release-until-pageflip-20260604T0020/`.

Buffer destroy observation from this run:

- no host-side `wl_buffer destroy` lines were present before the fault window.
- compositor-side destroyed-buffer drops appeared during verifier shutdown at `22:21:25`, after the verifier had already
  detected new kernel fault lines.
- `render_releases=0` means the host did not destroy its render target before the failure.

Conclusion:

- deferring invalidated-buffer release until after page-flip completion is not enough to eliminate the MMU fault.
- literal `wl_buffer.destroy` timing is again not the immediate trigger in this captured run.

### Exact outer-edge sampling in rotated `render_texture_from_to`

Status: ruled out as sufficient.

Hypothesis:

- BFM100 uses `DisplayTransform::Deg90`, and the recurring fault addresses are close to buffer edges.
- the GC400 path might be faulting when the rotated texture draw samples exactly on imported DMA-BUF edges during
  neighbor swipes.

Tested change:

- applied a 0.5 px inset to source rectangles for rotated texture draws only.
- left `Transform::Normal` source rectangles unchanged.
- kept neighbor rendering enabled.

Result:

- focused `cargo test -p bmc-openwrt scene_renderer` passed with 18 tests.
- deployed core + `widget-mining-info` to profile `196-link`.
- verifier failed on run 1:
  - `faults_before=59`
  - `faults_after=62`
  - `wiggle_steps=48 wiggle_end=2 render_allocs=3 render_releases=0`
- sampled memory/CMA in the failing run:
  - before startup: `MemAvailable=167636 kB`, `CmaFree=47784 kB`
  - after startup: `MemAvailable=116612 kB`, `CmaFree=20324 kB`
  - before wiggle 1: `MemAvailable=116532 kB`, `CmaFree=20320 kB`
  - after wiggle 1: `MemAvailable=114736 kB`, `CmaFree=20244 kB`
  - after wiggle 2: `MemAvailable=107480 kB`, `CmaFree=26720 kB`
  - after wiggle 3: `MemAvailable=104292 kB`, `CmaFree=71332 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-rotated-source-inset-20260604T0035/`.

Buffer destroy observation from this run:

- host log again showed `wl_buffer present commit` and `wl_buffer release slots drained`, but no host-side
  `wl_buffer.destroy` before the failure.
- compositor destroyed-buffer drops appeared during verifier shutdown at `22:28:25`, after the verifier had already
  detected new MMU fault lines.
- `render_releases=0` again means host render-target destruction was not part of the pre-fault window.

Conclusion:

- simple exact-edge source sampling is not the complete cause of BDK-501.
- the broader rotated-texture path may still be relevant, but a small source inset does not eliminate the fault.

### Oversized host staging texture / partial UV scaling

Status: ruled out as sufficient.

Hypothesis:

- `bmc-wasm-host` normally renders 480x480 BFM100 widget content through a 1280x480 shared staging texture.
- the host staging blit samples only the active subregion with `u_uv_scale=(480/1280, 480/480)`.
- the GC400 path might fault during the staging-to-export blit or following `glFinish` because it samples a partial
  subregion of an oversized texture.

Tested change:

- added a diagnostic `BMC_WASM_HOST_STAGING_SIZE` override.
- ran the verifier with `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- kept neighbor rendering enabled.
- deployed core + configured `widget-mining-info` to profile `198-link`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_before=71`
  - `faults_after_startup=71`
  - `faults_after_wiggle_1=73`
  - new dmesg fault: `[ 6443.113406] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA:
  - before startup: `MemAvailable=168184 kB`, `CmaFree=48168 kB`
  - after startup: `MemAvailable=119340 kB`, `CmaFree=19664 kB`
  - before wiggle 1: `MemAvailable=119252 kB`, `CmaFree=19628 kB`
  - after wiggle 1: `MemAvailable=117360 kB`, `CmaFree=19484 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-host-staging-480-20260604T0118/`.

Timing and buffer-destroy observation:

- the fault occurred after startup and during the first wiggle.
- first entering-neighbor host frame for peer `16241`:
  - lifecycle to `Entering`: `22:44:25.524`
  - render begin: `22:44:25.528`
  - runtime render complete: `22:44:25.579`
  - host staging blit complete: `22:44:25.631`
  - host `glFinish` complete: `22:44:26.663`
  - export/commit: `22:44:26.664`
- no host-side `wl_buffer.destroy` was logged before the fault window.
- compositor destroyed-buffer drops appeared during verifier shutdown at `22:44:26.791`, after the verifier had already
  detected new kernel fault lines.

Conclusion:

- the oversized 1280x480 staging texture and partial UV scaling are not sufficient to explain BDK-501.
- the reproduced fault still overlaps the host wait in `glFinish` after `SharedRenderScratch::blit_to`.
- exact-size staging reduced the blit/finish path shape but did not eliminate the MMU fault.

### `SharedRenderScratch::blit_to` as required trigger

Status: ruled out as required trigger.

Hypothesis:

- the MMU fault might require the host staging-to-export FBO blit in `SharedRenderScratch::blit_to`.
- earlier evidence showed faults while the host was waiting in the post-blit `glFinish`, but that could still mean the
  blit command itself was the bad GPU operation.

Tested change:

- added diagnostic `BMC_WASM_HOST_FINISH_BEFORE_BLIT=1`.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- inserted a host `glFinish` immediately after FemtoVG/runtime rendering and before `SharedRenderScratch::blit_to`.
- kept neighbor rendering enabled.
- deployed core + configured `widget-mining-info` to profile `199-link`.

Result:

- verifier passed run 1 and failed on run 2 after the first wiggle command:
  - `faults_before=74`
  - `faults_after_startup=74`
  - `faults_after_wiggle_1=76`
  - new dmesg fault: `[ 6878.217664] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA in the failing run:
  - before startup: `MemAvailable=167756 kB`, `CmaFree=67348 kB`
  - after startup: `MemAvailable=118904 kB`, `CmaFree=42004 kB`
  - before wiggle 1: `MemAvailable=118848 kB`, `CmaFree=41968 kB`
  - after wiggle 1: `MemAvailable=119000 kB`, `CmaFree=41824 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-pre-blit-finish-20260604T0134/`.

Timing and phase evidence:

- first entering-neighbor host frame for peer `17305` in the failing run:
  - lifecycle to `Entering`: `22:51:40.632`
  - render begin: `22:51:40.634`
  - runtime render complete: `22:51:40.685`
  - host pre-blit `glFinish` begin: `22:51:40.736`
  - host pre-blit `glFinish` complete: `22:51:41.758`
  - host staging blit complete: `22:51:41.759`
  - post-blit `glFinish` complete: `22:51:41.775`
  - export/commit: `22:51:41.776`
- mapping verifier uptime to wall time places the new kernel fault during the pre-blit `glFinish`, before the staging
  blit and before export/commit of the entering buffer.
- compositor DMA-BUF import for the entering buffer happened at `22:51:41.783`, after the fault window.

Buffer destroy observation:

- no host-side `wl_buffer.destroy` was logged before the pre-fault window.
- compositor destroyed-buffer drops appeared during verifier shutdown at `22:51:41.914`, after the verifier detected the
  new fault.
- `render_releases=0` for run 1; run 2 failed before the final run summary, but the host logs show no pre-fault destroy.

Conclusion:

- `SharedRenderScratch::blit_to` is not required to trigger at least this reproduced MMU fault.
- compositor import/render and buffer destruction are again after the fault window.
- the remaining suspect window is runtime/FemtoVG GPU rendering into the host staging FBO, completed by the pre-blit
  `glFinish`.

### Runtime work before FemtoVG flush as sufficient trigger

Status: ruled out as sufficient.

Hypothesis:

- the MMU fault might be caused by runtime/WASM execution or any direct GL work before `FemtoVgRenderer::flush()`.
- the previous pre-blit-finish run only proved the fault happened before `SharedRenderScratch::blit_to`; it did not
  split runtime rendering from FemtoVG command submission.

Tested change:

- added diagnostic `BMC_WASM_HOST_FINISH_BEFORE_BLIT=finish-around-femtovg-flush`.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- inserted one `glFinish` immediately after `runtime.render(...)` and before `FemtoVgRenderer::flush()`.
- kept the existing pre-blit `glFinish` immediately after `FemtoVgRenderer::flush()` and before
  `SharedRenderScratch::blit_to`.
- kept neighbor rendering enabled.
- deployed core + configured `widget-mining-info` to profile `200-link`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_before=77`
  - `faults_after_startup=77`
  - `faults_after_wiggle_1=79`
  - new dmesg fault: `[ 7236.308831] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA:
  - before startup: `MemAvailable=167852 kB`, `CmaFree=52476 kB`
  - after startup: `MemAvailable=118876 kB`, `CmaFree=25324 kB`
  - before wiggle 1: `MemAvailable=118844 kB`, `CmaFree=25252 kB`
  - after wiggle 1: `MemAvailable=118968 kB`, `CmaFree=25040 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-femtovg-flush-split-20260604T0149/`.

Timing and phase evidence:

- first entering-neighbor host frame for peer `18074`:
  - lifecycle to `Entering`: `22:57:38.726`
  - render begin: `22:57:38.729`
  - runtime render complete: `22:57:38.778`
  - pre-FemtoVG-flush `glFinish` begin: `22:57:38.779`
  - pre-FemtoVG-flush `glFinish` complete: `22:57:38.780`
  - pre-blit `glFinish` begin: `22:57:38.832`
  - pre-blit `glFinish` complete: `22:57:39.858`
  - staging blit complete: `22:57:39.858`
  - export/commit: `22:57:39.881`
- mapping verifier uptime to wall time places the new kernel fault during the post-FemtoVG-flush/pre-blit `glFinish`.
- compositor DMA-BUF import for the entering buffer happened at `22:57:39.882`, after the fault window.

Conclusion:

- runtime/WASM execution and any pre-FemtoVG-flush GL work are not sufficient to trigger this reproduced MMU fault.
- the faulting GPU work is submitted by `FemtoVgRenderer::flush()` for the entering-neighbor frame.
- `SharedRenderScratch::blit_to`, export/commit, compositor import/render, and buffer destroy remain after the fault
  window in this capture.

### Simple text rendering as required trigger

Status: ruled out as required trigger.

Hypothesis:

- the skip-text pass could have been caused by suppressing any `Canvas::fill_text` call, including simple labels.
- if simple text were required, keeping paragraph text rendered while skipping simple text should prevent the fault.

Tested change:

- ran the verifier with `BMC_RENDER_TEXT_DIAGNOSTIC=skip-simple-text`.
- kept paragraph text rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- kept `BMC_WASM_HOST_FINISH_BEFORE_BLIT=finish-around-femtovg-flush`.
- redeployed core + configured `widget-mining-info`; profile remained `202-link`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_before=80`
  - `faults_after_startup=80`
  - `faults_after_wiggle_1=82`
  - new dmesg fault addresses included `0xffffe9c0` and `0x000003c0`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=118832 kB`
  - `CmaFree=35016 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-skip-simple-text-20260604T0300/`.

Timing and buffer-destroy observation:

- first entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then entered the post-FemtoVG-flush/pre-blit `glFinish` and did not log completion, staging blit, or
  export/commit.
- no host-side `wl_buffer.destroy` or compositor destroyed-buffer drop was logged before the fault window.
- the first destroyed-buffer drop appeared during shutdown after the verifier had already detected the new fault.

Conclusion:

- simple text rendering is not required for the reproduced BDK-501 MMU fault.
- paragraph text rendering remains sufficient to reproduce the fault in the host/FemtoVG flush window.
- this result narrows the active suspect to rich paragraph rendering rather than all text rendering.

### Unnecessary paragraph decoration width measurement as the complete cause

Status: ruled out as sufficient.

Hypothesis:

- `ParagraphLayoutCache::draw` measured each rendered segment width with `Canvas::measure_text()` even when the span had
  no underline or strikethrough.
- Mining Info value/unit paragraphs do not use decorations, so those measurements were unnecessary and interleaved with
  paragraph `fill_text` calls in the faulting FemtoVG flush window.

Tested change:

- changed paragraph rendering to call `segment_width()` and draw decorations only when `style.underline` or
  `style.strikethrough` is true.
- kept normal paragraph text visible.
- kept neighbor rendering enabled.
- deployed core + configured `widget-mining-info` to profile `203-link`.

Result:

- verifier passed runs 1 through 7.
- verifier failed on run 8 after the first wiggle command:
  - `faults_before=86`
  - `faults_after_startup=86`
  - `faults_after_wiggle_1=88`
  - new dmesg fault: `[ 9815.629998] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=119040 kB`
  - `CmaFree=42812 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-no-deco-measure-20260604T0325/`.

Timing and buffer-destroy observation:

- first entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then spent about one second in the post-FemtoVG-flush/pre-blit `glFinish`.
- the new kernel fault landed during that wait.
- destroyed-buffer drops were logged only after the verifier detected the fault and sent shutdown.

Conclusion:

- unnecessary paragraph segment width measurement is not the complete cause of BDK-501.
- it may affect timing or pressure, but it does not pass the 10-run verifier.
- the remaining target is paragraph `fill_text` submission or the glyph-cache work caused by that submission.

### Missing paragraph fallback font as the complete cause

Status: ruled out as sufficient.

Hypothesis:

- simple text paths set FemtoVG fonts to `[selected_font, font_fallback]`, but paragraph segment rendering used only the
  selected Braiins font.
- Mining Info paragraph units include non-ASCII text such as `°C`, so missing fallback could have made paragraph
  glyph-cache behavior differ from simple text rendering.

Tested change:

- changed paragraph segment `fill_text()` and `measure_text()` paints to include `font_fallback`.
- kept normal paragraph text visible.
- kept neighbor rendering enabled.
- tested on top of the no-decoration-measure change.
- deployed core + configured `widget-mining-info` to profile `204-link`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_before=89`
  - `faults_after_startup=89`
  - `faults_after_wiggle_1=91`
  - new dmesg fault: `[10125.000095] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=119292 kB`
  - `CmaFree=29952 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-fallback-font-20260604T0345/`.

Timing and buffer-destroy observation:

- the first entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then spent about one second in the post-FemtoVG-flush/pre-blit `glFinish`.
- the new kernel fault landed during that wait.
- destroyed-buffer drops were logged only after fault detection and shutdown.

Conclusion:

- missing fallback font in paragraph rendering is not the complete cause of BDK-501.
- the remaining target is the paragraph `fill_text` workload itself, including how many text segments are submitted and
  how FemtoVG uploads/renders their glyphs.

### Multiple styled paragraph spans per line as the complete cause

Status: ruled out as sufficient.

Hypothesis:

- Mining Info paragraph text has separate value/unit spans, so multiple styled paragraph `fill_text()` calls per line
  might have been the trigger.
- merging a paragraph line into one visible text draw would avoid the per-span draw sequence while still rendering the
  scenes that are swiped to.

Tested change:

- changed paragraph rendering to draw each laid-out paragraph line as one visible `fill_text()` call using the base
  style.
- kept paragraph text visible and kept neighbor rendering enabled.
- tested on top of the no-decoration-measure and paragraph fallback-font candidates.
- deployed core + configured `widget-mining-info` to profile `205-link`.

Result:

- verifier failed on run 2 after the first wiggle command:
  - `faults_after_startup=92`
  - `faults_after_wiggle_1=94`
  - new dmesg fault: `[10485.256098] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=118816 kB`
  - `CmaFree=30812 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-merge-paragraph-spans-20260604T0400/`.

Timing and buffer-destroy observation:

- the first entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then spent about one second in the post-FemtoVG-flush/pre-blit `glFinish`.
- the new kernel fault landed during that wait.
- destroyed-buffer drops were logged only after fault detection and shutdown.

Conclusion:

- paragraph span segmentation and multiple styled text draws per paragraph line are not the complete cause of BDK-501.
- the active target is the paragraph line `fill_text` workload itself, including glyph atlas uploads or draw submission
  generated by `FemtoVgRenderer::flush()`.

### Paragraph glyph offsets and alphabetic baseline as the complete cause

Status: ruled out as sufficient.

Hypothesis:

- paragraph rendering might have faulted because it submitted text at cosmic-text glyph x offsets and alphabetic
  baseline coordinates, unlike the simpler canvas text path.
- drawing each paragraph line with a simple top-baseline `fill_text()` call would avoid that coordinate/baseline path
  while keeping visible paragraph text and neighbor rendering.

Tested change:

- changed paragraph rendering to draw each laid-out paragraph line as one visible top-baseline `fill_text()` call.
- bypassed per-span drawing, cosmic glyph x offsets, and alphabetic-baseline positioning.
- kept paragraph text visible and kept neighbor rendering enabled.
- deployed core + configured `widget-mining-info` to profile `206-link`.

Result:

- verifier failed on run 2 after the first wiggle command:
  - `faults_after_startup=95`
  - `faults_after_wiggle_1=97`
  - new dmesg fault: `[10947.285106] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe9c0`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=118768 kB`
  - `CmaFree=35932 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-simple-lines-20260604T0415/`.

Timing and buffer-destroy observation:

- the first entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then spent about one second in the post-FemtoVG-flush/pre-blit `glFinish`.
- the new kernel fault landed during that wait.
- destroyed-buffer drops were logged only after fault detection and shutdown.

Conclusion:

- paragraph glyph x offsets and alphabetic-baseline positioning are not the complete cause of BDK-501.
- visible paragraph text rendered through FemtoVG `fill_text()` remains the narrowed trigger.

### Plain RAM/CMA exhaustion as the immediate trigger

Status: ruled out as sufficient.

Evidence:

- multiple verifier runs sampled memory and CMA immediately around the fault.
- failures consistently occurred with substantial available memory:
  - skip-simple-text failure: `MemAvailable=118832 kB`, `CmaFree=35016 kB`
  - no-decoration-measure failure: `MemAvailable=119040 kB`, `CmaFree=42812 kB`
  - paragraph fallback-font failure: `MemAvailable=119292 kB`, `CmaFree=29952 kB`
  - merge-paragraph-spans failure: `MemAvailable=118816 kB`, `CmaFree=30812 kB`
  - paragraph-simple-lines failure: `MemAvailable=118768 kB`, `CmaFree=35932 kB`
- the production-default GPU render lock also passed the 10-run BFM100 verifier with `CmaFree` dropping to `5892 kB` and
  `MemAvailable` dropping to `103740 kB` without a new GPU fault.

Conclusion:

- simple total RAM exhaustion or simple total CMA exhaustion is not the immediate BDK-501 trigger.
- CMA fragmentation or driver allocation pressure is still not ruled out; dmesg history includes `alloc_contig_range`
  busy messages, but the fault itself reproduces with non-trivial `CmaFree` available.

### Any tiny visible paragraph fill_text payload as sufficient trigger

Status: ruled out as sufficient.

Hypothesis:

- any visible paragraph `fill_text()` submission during entering-neighbor rendering might be enough to fault, even with
  tiny text payloads.

Tested change:

- changed paragraph rendering to replace each laid-out paragraph line with the visible placeholder text `"X"`.
- kept paragraph draw calls, layout, and neighbor scene rendering enabled.
- deployed core + configured `widget-mining-info` to profile `207-link`.

Result:

- verifier exited successfully after 10 restarted BFM100 wiggle runs.
- `rtk` truncated redirected output after run 3, but the SSH process exit code was `0`.
- captured visible verifier output showed no fault-counter increase:
  - run 1: `faults_after_startup=98`, `faults_after=98`
  - run 2: `faults_after_startup=98`, `faults_after=98`
  - run 3 visible portion: `faults_after_startup=98`, `faults_after_wiggle_1=98`
- post-run dmesg sanity check showed the dmesg tail ending at the previous paragraph-simple-lines fault; no new
  `MMU fault status` appeared after this verifier run.
- bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-placeholder-lines-20260604T0425/`.

Conclusion:

- a tiny visible placeholder paragraph payload is not sufficient to reproduce BDK-501.
- the real Mining Info paragraph text payload or the glyph atlas/upload work it causes remains implicated.

### Distinct real Mining Info glyph variety as required trigger

Status: ruled out as required.

Hypothesis:

- the real Mining Info text might fault because it uses distinct numeric, punctuation, unit, slash, percent, and degree
  glyphs that churn or upload more glyph atlas entries than a placeholder.

Tested change:

- changed paragraph rendering to preserve each line's length and whitespace but replace every non-space character with
  `X`.
- kept paragraph draw calls, layout, and neighbor scene rendering enabled.
- deployed core + configured `widget-mining-info` to profile `208-link`.

Result:

- verifier failed on run 2 after the first wiggle command:
  - `faults_after_startup=98`
  - `faults_after_wiggle_1=100`
  - new dmesg fault: `[12095.626333] etnaviv-gpu 59000000.gpu: MMU fault status 0x00000002`
  - new fault address: `0xffffe480`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=116588 kB`
  - `CmaFree=47708 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-xmask-lines-20260604T0438/`.

Timing and buffer-destroy observation:

- the faulting entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then entered the post-FemtoVG-flush/pre-blit `glFinish`; no completion, staging blit, export, or commit was
  logged for that frame.
- destroyed-buffer drops were logged only after fault detection and shutdown.

Conclusion:

- distinct real Mining Info glyph variety is not required for BDK-501.
- preserving paragraph line length/spacing with repeated `X` glyphs is sufficient to reproduce, while the one-character
  placeholder passed 10 restarted runs.

### Flushing FemtoVG after each paragraph line as sufficient mitigation

Status: ruled out as sufficient.

Hypothesis:

- the fault might be caused by letting too much paragraph text work accumulate before a FemtoVG flush, and flushing
  after each paragraph line might keep the GPU command stream small enough to avoid the MMU fault.

Tested change:

- kept normal paragraph rendering and inserted `canvas.flush()` after every paragraph line.
- kept paragraph text visible and kept neighbor scene rendering enabled.
- deployed core + configured `widget-mining-info` to profile `209-link`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_after_startup=101`
  - `faults_after_wiggle_1=103`
  - new dmesg fault addresses: `0xffffddc0`, then `0xffffc380`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=119528 kB`
  - `CmaFree=29068 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-flush-lines-20260604T0450/`.

Timing and buffer-destroy observation:

- the faulting entering-neighbor frame completed runtime render and then entered the first host pre-FemtoVG-flush
  `glFinish`.
- no pre-FemtoVG-flush completion, pre-blit, staging blit, export, or commit was logged for that frame.
- destroyed-buffer drops were logged only after fault detection and shutdown.

Conclusion:

- flushing after every paragraph line is not a sufficient mitigation.
- this keeps the active suspect inside host runtime/FemtoVG text GPU work; the failure still precedes staging blit,
  buffer export, Wayland commit, and compositor-side buffer destruction.

### Capping xmask paragraph lines to 8 characters as sufficient mitigation

Status: ruled out as sufficient.

Hypothesis:

- the fault might require the full paragraph line length, and capping visible xmask text to 8 characters per paragraph
  line might reduce glyph/quad volume enough to avoid the MMU fault while still rendering visible neighboring scenes.

Tested change:

- changed paragraph xmask rendering to preserve only up to the first 8 characters per laid-out paragraph line.
- kept paragraph text visible, preserved whitespace within that limit, and kept neighbor scene rendering enabled.
- deployed core + configured `widget-mining-info` to profile `210-link`.

Result:

- verifier passed runs 1 and 2, then failed on run 3 after the first wiggle command:
  - `faults_after_startup=107`
  - `faults_after_wiggle_1=109`
  - new fault address: `0xffffde40`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=113584 kB`
  - `CmaFree=61344 kB`
- failure bundle: `/tmp/claude-1001/BDK-501-verify-paragraph-xmask-limit8-20260604T0033/`.

Timing and buffer-destroy observation:

- the faulting entering-neighbor frame completed runtime render and the pre-FemtoVG-flush `glFinish`.
- the host then entered the post-FemtoVG-flush/pre-blit `glFinish`; no completion, staging blit, export, or commit was
  logged for that frame.
- destroyed-buffer drops were logged only after fault detection and shutdown.

Conclusion:

- capping paragraph xmask text to 8 characters per line is not a sufficient mitigation.
- the one-character placeholder still matters as a lower-volume contrast; the fault threshold is above that placeholder
  case and at or below this 8-character xmask case under the current BFM100 wiggle repro.

### Pinning all GPU submitters to CPU 0 instead of locking

Status: ruled out as sufficient.

Hypothesis:

- the MMU fault might be caused by userspace GPU submitters running on different CPU cores, and pinning `bmc-openwrt`,
  `bmc-wasm-host`, and the widget thin processes to a single core might provide the same protection as the cross-process
  GPU render lock.

Tested change:

- copied ARM `util-linux` `taskset` to the device.
- ran the BFM100 wiggle verifier with `start-compositor` launched through `taskset -c 0`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the inter-process render lock.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- did not set `BMC_WASM_HOST_FINISH_BEFORE_BLIT`.

Affinity evidence:

- before wiggle in the failing run:
  - `bmc-openwrt pid=9687 Cpus_allowed_list: 0`
  - `bmc-wasm-host pid=9710 Cpus_allowed_list: 0`
  - all three `bmc-wasm-thin` processes had `Cpus_allowed_list: 0`

Result:

- verifier passed run 1 and then failed on run 2 after the first wiggle command:
  - `faults_after_startup=110`
  - `faults_after_wiggle_1=112`
  - new fault addresses: `0xffffe9c0`, then `0x000003c0`
- sampled memory/CMA at the failure sample:
  - `MemAvailable=115844 kB`
  - `CmaFree=43680 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-cpu0-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure compositor log counts:
  - `Dropped destroyed buffer`: 1
  - `Invalidated cached texture`: 3
  - `Destroyed buffer`: 0

Conclusion:

- CPU-core pinning alone is not a sufficient mitigation.
- the cross-process lock is not merely preventing different userspace processes from issuing GPU work on different CPU
  cores.
- the necessary property appears to be explicit serialization of GL command emission/submission across the separate DRM
  clients.

### Mesa `ETNA_MESA_DEBUG=cflush_all` without app lock

Status: ruled out as sufficient.

Hypothesis:

- forcing etnaviv to flush all relevant GPU caches before state updates might be enough to avoid the MMU fault without
  the app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set `ETNA_MESA_DEBUG=cflush_all`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- did not set `BMC_WASM_HOST_FINISH_BEFORE_BLIT`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_after_startup=116`
  - `faults_after_wiggle_1=118`
  - new fault address: `0xfffff080`
- final post-run device fault counter was `120`, including recover lines.
- sampled memory/CMA at the failure sample:
  - `MemAvailable=107984 kB`
  - `CmaFree=50228 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-cflush-all-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure compositor log counts:
  - `Dropped destroyed buffer`: 2
  - `Invalidated cached texture`: 4
  - `Destroyed buffer`: 0

Conclusion:

- `cflush_all` alone is not a sufficient mitigation.
- cache flushing before state updates, even with Mesa's pre-HALTI5 stall-before-texture-cache-flush path, does not
  replace either the app-level cross-process render lock or the combined `flush_all,cflush_all` diagnostic.

### Mesa `ETNA_MESA_DEBUG=draw_stall` without app lock

Status: ruled out as sufficient.

Hypothesis:

- `ETNA_MESA_DEBUG=flush_all` might only help because it waits/stalls GPU pipeline progress after each draw. If so,
  `ETNA_MESA_DEBUG=draw_stall`, which emits an FE/PE stall after each draw without flushing/submitting after each
  primitive, might also prevent the MMU fault.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set `ETNA_MESA_DEBUG=draw_stall`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.
- did not set `BMC_WASM_HOST_FINISH_BEFORE_BLIT`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_after_startup=120`
  - `faults_after_wiggle_1=122`
  - new fault address: `0xffffe7c0`
- final post-run device fault counter was `123`, including the recover line.
- sampled memory/CMA at the failure sample:
  - `MemAvailable=110608 kB`
  - `CmaFree=47524 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-draw-stall-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure compositor log counts:
  - `Dropped destroyed buffer`: 2
  - `Invalidated cached texture`: 5
  - `Destroyed buffer`: 0

Conclusion:

- `draw_stall` alone is not a sufficient mitigation.
- the successful `flush_all` diagnostic is not explained merely by inserting FE/PE stalls after draw commands.
- the important `flush_all` behavior is more likely the Gallium flush/submission boundary after every rendered
  primitive, command-buffer segmentation, or explicit sync/fence progress caused by those flushes.

### Compositor-side-only Mesa `ETNA_MESA_DEBUG=flush_all`

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might only need to affect the compositor process. If so, setting
  `flush_all` globally for `bmc-openwrt` while removing it from spawned widget/host processes should still avoid the MMU
  fault.

Tested change:

- deployed profile `215-link` with a temporary spawner hook that treats `BMC_DIAG_WIDGET_ETNA_MESA_DEBUG=__unset` as
  `cmd.env_remove("ETNA_MESA_DEBUG")` for widget processes.
- set global `ETNA_MESA_DEBUG=flush_all`.
- set `BMC_DIAG_WIDGET_ETNA_MESA_DEBUG=__unset` so `bmc-wasm-thin` and inherited `bmc-wasm-host` did not receive
  `ETNA_MESA_DEBUG`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle command:
  - `faults_after_startup=123`
  - `faults_after_wiggle_1=125`
  - new fault address: `0xffffe9c0`
- final post-run device fault counter was `126`, including the recover line.
- sampled memory/CMA at the failure sample:
  - `MemAvailable=114956 kB`
  - `CmaFree=25684 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-compositor-etna-flush-only-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure compositor log counts:
  - `Dropped destroyed buffer`: 2
  - `Invalidated cached texture`: 4
  - `Destroyed buffer`: 0

Conclusion:

- compositor-side-only `flush_all` is not sufficient.
- the matching widget-side-only pass and compositor-side-only failure identify the host/widget Mesa command stream as
  the side where per-primitive Gallium flush/submission segmentation prevents the MMU fault.

### Mesa `ETNA_MESA_DEBUG=no_ts` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding a tile-status interaction in etnaviv. If so,
  disabling tile status with `ETNA_MESA_DEBUG=no_ts` might prevent the MMU fault without the app-level cross-process
  render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=no_ts`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier passed run 1 and failed on run 2 after the first wiggle:
  - `faults_after_startup=126`
  - `faults_after_wiggle_1=128`
  - new fault address: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `129`
- sampled memory/CMA at the failing sample:
  - `MemAvailable=115536 kB`
  - `CmaFree=47092 kB`
- minimum sampled values across the diagnostic:
  - `MemAvailable=97836 kB`
  - `CmaFree=43320 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-no-ts-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- disabling tile status is not sufficient to replace either the app-level cross-process render lock or Mesa `flush_all`.
- the successful `flush_all` diagnostic is less likely to be explained by a simple tile-status-compression hazard alone.

### Runtime `etnaviv.hw_job_limit=1`

Status: not tested at runtime.

Hypothesis:

- limiting etnaviv to one queued hardware job might serialize enough kernel-side scheduler work to avoid the no-lock
  multi-process MMU fault.

Observation:

- current runtime value was `hw_job_limit=4`.
- `/sys/module/etnaviv/parameters/hw_job_limit` had permissions `-r--r--r--`.

Conclusion:

- this diagnostic cannot be run by writing the sysfs parameter on the live device.
- it remains an open boot-parameter or kernel-build diagnostic rather than a ruled-out mitigation.

### Mesa `ETNA_MESA_DEBUG=no_supertile` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding a tiled/supertiled layout interaction in
  etnaviv. If so, disabling supertiling with `ETNA_MESA_DEBUG=no_supertile` might prevent the MMU fault without the
  app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=no_supertile`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_after_startup=129`
  - `faults_after_wiggle_1=131`
  - new fault address: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `132`
- sampled memory/CMA at the failing sample:
  - `MemAvailable=109356 kB`
  - `CmaFree=43880 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-no-supertile-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- disabling supertiling is not sufficient to replace either the app-level cross-process render lock or Mesa `flush_all`.
- the successful `flush_all` diagnostic is less likely to be explained by a simple tiled-layout hazard alone.

### Mesa `ETNA_MESA_DEBUG=no_singlebuffer` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding etnaviv single-buffer resource reuse or
  synchronization behavior. If so, disabling that feature with `ETNA_MESA_DEBUG=no_singlebuffer` might prevent the MMU
  fault without the app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=no_singlebuffer`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_after_startup=132`
  - `faults_after_wiggle_1=134`
  - new fault address: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `135`
- sampled memory/CMA at the failing sample:
  - `MemAvailable=109548 kB`
  - `CmaFree=41264 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-no-singlebuffer-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- disabling etnaviv's single-buffer feature is not sufficient to replace either the app-level cross-process render lock
  or Mesa `flush_all`.
- the successful `flush_all` diagnostic is less likely to be explained by single-buffer resource reuse alone.

### Mesa `ETNA_MESA_DEBUG=linear_pe` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding a PE tiling/layout path. If so, forcing the
  linear PE path with `ETNA_MESA_DEBUG=linear_pe` might prevent the MMU fault without the app-level cross-process render
  lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=linear_pe`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_after_startup=135`
  - `faults_after_wiggle_1=137`
  - new fault address: `0xffffc000`
  - verifier-pattern fault counter after the run: `138`
- sampled memory/CMA at the failing sample:
  - `MemAvailable=114988 kB`
  - `CmaFree=40772 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-linear-pe-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- forcing the linear PE path is not sufficient to replace either the app-level cross-process render lock or Mesa
  `flush_all`.
- the successful `flush_all` diagnostic is less likely to be explained by a simple PE layout mode alone.

### Mesa `ETNA_MESA_DEBUG=zero` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be masking stale or uninitialized resource contents. If
  so, zeroing all resources after allocation with `ETNA_MESA_DEBUG=zero` might prevent the MMU fault without the
  app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=zero`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_after_startup=138`
  - `faults_after_wiggle_1=140`
  - first new fault address observed by the verifier: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `144`
- post-run dmesg tail also showed a second fault address: `0x00000fc0`.
- sampled memory/CMA at the failing sample:
  - `MemAvailable=107852 kB`
  - `CmaFree=38264 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-zero-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- zeroing all resources after allocation is not sufficient to replace either the app-level cross-process render lock or
  Mesa `flush_all`.
- stale or uninitialized resource contents are less likely to explain the successful `flush_all` diagnostic.

### Mesa `ETNA_MESA_DEBUG=no_texdesc` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the host/FemtoVG workload is text/glyph-heavy, so the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be
  avoiding a texture descriptor path issue. If so, disabling texture descriptors with `ETNA_MESA_DEBUG=no_texdesc` might
  prevent the MMU fault without the app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=no_texdesc`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier passed run 1 and failed on run 2 after the first wiggle:
  - `faults_after_startup=144`
  - `faults_after_wiggle_1=146`
  - first new fault address observed by the verifier: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `150`
- post-run dmesg tail also showed a second fault address: `0x000003c0`.
- sampled memory/CMA at the failing sample:
  - `MemAvailable=115888 kB`
  - `CmaFree=55868 kB`
- minimum sampled values across the diagnostic:
  - `MemAvailable=96880 kB`
  - `CmaFree=35488 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-no-texdesc-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- disabling texture descriptors is not sufficient to replace either the app-level cross-process render lock or Mesa
  `flush_all`.
- a simple etnaviv texture-descriptor-path issue is less likely to explain the successful `flush_all` diagnostic.

### `GALLIUM_THREAD=0` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding Gallium threaded-context batching, reordering,
  or offloading rather than changing etnaviv submission/progress. If so, disabling Gallium threaded context with
  `GALLIUM_THREAD=0` might prevent the MMU fault without the app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- left `ETNA_MESA_DEBUG` unset.
- set `GALLIUM_THREAD=0`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier passed run 1 and failed on run 2 after the first wiggle:
  - `faults_after_startup=150`
  - `faults_after_wiggle_1=152`
  - first new fault address observed by the verifier: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `156`
- post-run dmesg tail also showed a second fault address: `0x00000fc0`.
- sampled memory/CMA at the failing sample:
  - `MemAvailable=115308 kB`
  - `CmaFree=52636 kB`
- minimum sampled values across the diagnostic:
  - `MemAvailable=102744 kB`
  - `CmaFree=52516 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-gallium-thread0-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- disabling Gallium threaded context is not sufficient to replace either the app-level cross-process render lock or Mesa
  `flush_all`.
- the successful `flush_all` diagnostic is less likely to be explained by threaded-context command reordering/offloading
  alone.

### Combined etnaviv feature-disable flags without app lock

Status: ruled out as sufficient.

Hypothesis:

- the individual feature-disable flags might fail alone but pass in combination if the MMU fault requires interaction
  between multiple etnaviv resource/layout/texture paths.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set `ETNA_MESA_DEBUG=no_ts,no_supertile,no_singlebuffer,linear_pe,zero,no_texdesc`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_after_startup=156`
  - `faults_after_wiggle_1=158`
  - first new fault address observed by the verifier: `0xffffe9c0`
  - verifier-pattern fault counter after the run: `162`
- post-run dmesg tail also showed a second fault address: `0x00000000`.
- sampled memory/CMA at the failing sample:
  - `MemAvailable=112040 kB`
  - `CmaFree=50416 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-combined-feature-disable-verify-device-20260604.log`.

Buffer-destroy observation:

- post-failure log counts:
  - `Dropped destroyed buffer`: 0
  - `Invalidated cached texture`: 2
  - `Destroyed buffer`: 0

Conclusion:

- combining `no_ts`, `no_supertile`, `no_singlebuffer`, `linear_pe`, `zero`, and `no_texdesc` is not sufficient to
  replace either the app-level cross-process render lock or Mesa `flush_all`.
- the successful `flush_all` diagnostic is less likely to be explained by an interaction between these etnaviv
  resource/layout/texture feature paths.

### System memory or CMA exhaustion as the immediate trigger

Status: ruled out as sufficient for the collected failures.

Evidence:

- repeated verifier runs sampled memory and CMA before startup, after startup, and after wiggle steps.
- recent failing runs had healthy memory/CMA at the failure sample:
  - `GALLIUM_THREAD=0`: `MemAvailable=115308 kB`, `CmaFree=52636 kB`
  - combined etnaviv feature-disable flags: `MemAvailable=112040 kB`, `CmaFree=50416 kB`
  - coredump baseline: `MemAvailable=115064 kB`, `CmaFree=52048 kB`
- the coredump baseline run captured `/sys/class/devcoredump/devcd2/data` at `3474664` bytes, and the parsed active BO
  IOVA ranges did not contain the first fault address `0xffffe9c0`.
- the parsed MMUv2 page table showed exception PTEs for both captured fault pages:
  - `0xffffe9c0`: PTE `0x00000002`; valid pages in that master entry ended at `0xffffc000`.
  - `0x000003c0`: PTE `0x00000002`; low valid pages started at `0x00001000`.

Conclusion:

- low system memory or low free CMA is not a sufficient explanation for the reproduced MMU faults.
- the captured failure is better explained as GPU work referencing unmapped virtual addresses than as allocation
  exhaustion.
- memory pressure can still change timing or allocation layout, but the observed failures do not require exhausted
  memory/CMA.

### Mesa `ETNA_MESA_DEBUG=nocache` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful host-side `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding shader-cache reuse or cached
  compiler/resource state, not the Gallium flush/submission boundary itself. If so, disabling the shader cache with
  `ETNA_MESA_DEBUG=nocache` might prevent the MMU fault without the app-level cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=nocache`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_after_startup=168`
  - `faults_after_wiggle_1=170`
  - first new fault address observed by the verifier: `0xffffe9c0`
  - second new fault address after recovery: `0x000003c0`
  - verifier-pattern fault counter after the run: `174`
- sampled memory/CMA at the failing sample:
  - `MemAvailable=109876 kB`
  - `CmaFree=46480 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-nocache-verify-device-20260604.log`.

Buffer-destroy observation:

- host `Dropped destroyed buffer`: 0 in the fault window.
- compositor `Dropped destroyed buffer ObjectId(wl_buffer@11)` appeared at `2026-06-04T07:37:49.523Z`, after the
  verifier had detected the new fault.

Conclusion:

- disabling the etnaviv shader cache is not sufficient to replace either the app-level cross-process render lock or Mesa
  `flush_all`.
- shader-cache reuse or cached compiler artifacts are less likely to explain the successful `flush_all` diagnostic.

### Mesa `ETNA_MESA_DEBUG=no_autodisable` as a distinct env-only diagnostic

Status: ruled out as useful to run separately.

Reason:

- Mesa etnaviv exposes `ETNA_MESA_DEBUG=no_autodisable`, but the local Mesa tree already sets `ETNA_DBG_NO_AUTODISABLE`
  unconditionally in `etnaviv_screen.c`.
- setting the env flag would not change the tested etnaviv feature set for this repro.

Conclusion:

- `no_autodisable` is not a useful separate env-only diagnostic for BDK-501 on the current Mesa build.
- this does not prove autodisable is unrelated on other Mesa versions; it only means the current build already disables
  it.

### Mesa `ETNA_MESA_DEBUG=no_early_z` without app lock

Status: ruled out as sufficient.

Hypothesis:

- the successful host-side `ETNA_MESA_DEBUG=flush_all` diagnostic might be avoiding an early-Z/depth-state interaction.
  If so, disabling early Z with `ETNA_MESA_DEBUG=no_early_z` might prevent the MMU fault without the app-level
  cross-process render lock.

Tested change:

- used profile `212-link`.
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set global `ETNA_MESA_DEBUG=no_early_z`.
- kept normal neighbor rendering enabled.
- kept `BMC_WASM_HOST_STAGING_SIZE=480x480`.

Result:

- verifier passed run 1 and failed on run 2:
  - run 1: `faults_after=174`
  - run 2: `faults_after_startup=174`
  - run 2: `faults_after_wiggle_1=176`
  - dmesg tail showed the same fault-address pattern:
    - `0xffffe9c0`
    - `0x000003c0`
  - verifier-pattern fault counter after the run: `180`
- sampled memory/CMA at the failing sample:
  - `MemAvailable=115636 kB`
  - `CmaFree=50872 kB`
- full log: `/tmp/claude-1001/bdk501-no-lock-etna-no-early-z-verify-device-20260604.log`.

Buffer-destroy observation:

- host `Dropped destroyed buffer`: 0 in the fault window.
- compositor `Dropped destroyed buffer ObjectId(wl_buffer@11)` appeared at `2026-06-04T07:42:54.993Z`, after the
  verifier had detected the new fault.

Conclusion:

- disabling early Z is not sufficient to replace either the app-level cross-process render lock or Mesa `flush_all`.
- early-Z/depth-state behavior is less likely to explain the successful `flush_all` diagnostic.

### Host/FemtoVG GPU submits alone as the sufficient trigger

Status: ruled out for the current repro.

Hypothesis:

- the BDK-501 MMU fault might be triggered entirely by the widget host/FemtoVG workload, and the compositor process
  might only be incidental.
- if so, keeping host rendering active while preventing compositor-side GPU submits should still reproduce the fault.

Tested change:

- used profile `216-link`.
- deployed both configured packages:
  - `core`: `/nix/store/3h8sq5l77awby16sibvis0mmpmgqqin6-bmc-bmc-core`
  - `widget-mining-info`: `/nix/store/9jg8sskz5bsi2clw9nr63mc982mr351q-bmc-widget-mining-info`
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level cross-process render lock.
- set `BMC_DIAG_SKIP_COMPOSITOR_GPU_SUBMITS=1`.
- left `ETNA_MESA_DEBUG` unset.
- kept normal widget lifecycle, neighbor acquisition, Wayland traffic, DMA-BUF import, and host rendering active.
- skipped compositor GLES texture cleanup submits and compositor `render_scene()` submits.

Result:

- first 10-run verifier passed with no new verifier-pattern GPU fault lines:
  - `faults_before=180`
  - `faults_after=180`
  - final-run `wiggle_steps=60 wiggle_end=3 render_allocs=3 render_releases=0`
- repeated 10-run verifier also passed with no new verifier-pattern GPU fault lines:
  - every run started with `faults_before=180`
  - every run ended with `faults_after=180`
  - final line: `BDK-501 verification passed: 10 restarted BFM100 wiggle runs without new GPU fault lines`
  - final-run `wiggle_steps=60 wiggle_end=3 render_allocs=3 render_releases=0`
- final-run compositor log had `79` `diagnostic: skipping compositor render_scene GPU submit` entries.
- host render target allocations still occurred for visible and entering scenes.
- compositor DMA-BUF import still occurred, but compositor render/cleanup GPU submits were skipped.
- repeated-run memory/CMA stayed healthy:
  - minimum sampled `MemAvailable=112972 kB`
  - minimum sampled `CmaFree=51472 kB`
- full repeat verifier log: `/tmp/claude-1001/bdk501-skip-compositor-gpu-submits-verify-20260604.log`.

Conclusion:

- host/FemtoVG GPU submits alone are not sufficient to reproduce BDK-501 in the current BFM100 wiggle repro.
- a compositor-side GPU submit from a separate process/DRM file/MMU context appears to be the missing ingredient.
- this explains why an otherwise empty compositor render can still matter: it is still a second-process etnaviv job and
  can force scheduler/MMU-context interleaving.
- skipping compositor GPU submits is not a fix because scenes being swiped to must remain renderable.

### Compositor texture import and widget texture drawing as necessary trigger

Status: ruled out as necessary for the current repro.

Hypothesis:

- the BDK-501 MMU fault might require compositor-side import or sampling/drawing of widget DMA-BUF textures.
- if so, a compositor render path that submits only a clear/page-flip GPU job, while skipping texture import and widget
  texture drawing, should avoid the fault.

Tested change:

- used profile `217-link`.
- deployed both configured packages:
  - `core`: `/nix/store/r5rimrfkxmppqnv1cpmf7bypigj3kv00-bmc-bmc-core`
  - `widget-mining-info`: `/nix/store/9jg8sskz5bsi2clw9nr63mc982mr351q-bmc-widget-mining-info`
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level cross-process render lock.
- set `BMC_DIAG_COMPOSITOR_CLEAR_ONLY_SUBMIT=1`.
- left `ETNA_MESA_DEBUG` unset.
- did not set `BMC_DIAG_SKIP_COMPOSITOR_GPU_SUBMITS`.
- kept host rendering and normal widget lifecycle/swipe-neighbor acquisition active.
- compositor `render_scene()` still bound a back buffer, cleared it, finished/flushed/finished GLES work, and
  page-flipped.
- compositor `render_scene()` skipped compositor texture import and widget texture drawing.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_before=180`
  - `faults_after_startup=180`
  - `faults_after_wiggle_1=182`
  - fault addresses: `0xffffe9c0`, then `0x000003c0`
- compositor diagnostic evidence:
  - clear-only submit log count: `18`
  - skip-compositor-submit log count: `0`
- memory/CMA at failure detection was not exhausted:
  - `MemAvailable=117080 kB`
  - `CmaFree=10336 kB`
- host `Dropped destroyed buffer`: `0`.
- compositor `Dropped destroyed buffer ObjectId(wl_buffer@11)` appeared after the verifier had detected the new fault.
- full verifier log: `/tmp/claude-1001/bdk501-clear-only-submit-verify-20260604.log`.

Conclusion:

- compositor-side texture import and widget texture drawing are not necessary for the BDK-501 repro.
- a minimal second-process compositor GLES clear/submit/page-flip job is sufficient when host rendering is active and
  the app-level cross-process render lock is disabled.
- the remaining likely failure boundary is host/compositor etnaviv submit or MMU-context interleaving, not the
  compositor texture sampling path.
- clear-only rendering is not a fix because scenes being swiped to must remain renderable.

### Compositor page flip/scanout as a necessary trigger

Status: ruled out as necessary for the current repro.

Hypothesis:

- the clear-only diagnostic might have failed because the compositor still page-flipped a scanout buffer.
- if KMS page flip or display scanout were required, then keeping the same clear-only compositor GLES submit but
  skipping `page_flip()` should avoid the MMU fault.

Tested change:

- used profile `218-link`.
- deployed both configured packages:
  - `core`: `/nix/store/wmynx190syvdz266jgqxssscbhmin1n9-bmc-bmc-core`
  - `widget-mining-info`: `/nix/store/mabw48jvs84ndzdyra0g6qc4zsm5mwfq-bmc-widget-mining-info`
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set `BMC_DIAG_COMPOSITOR_CLEAR_ONLY_SUBMIT=1`.
- set `BMC_DIAG_COMPOSITOR_SKIP_PAGE_FLIP=1`.
- left `ETNA_MESA_DEBUG` unset.
- kept host rendering and normal widget lifecycle/swipe-neighbor acquisition active.
- compositor `render_scene()` still bound a back buffer, cleared it, finished/flushed/finished GLES work, and then
  returned without calling `page_flip()`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_before=186`
  - `faults_after_startup=186`
  - `faults_after_wiggle_1=188`
  - fault addresses: `0xffffe9c0`, then `0x000003c0`
- compositor diagnostic evidence:
  - clear-only submit log count: `25`
  - skip-page-flip log count: `25`
  - skip-compositor-submit log count: `0`
- memory/CMA at failure detection was not exhausted:
  - `MemAvailable=108224 kB`
  - `CmaFree=49760 kB`
- host `Dropped destroyed buffer`: `0`.
- compositor `Dropped destroyed buffer ObjectId(wl_buffer@11)` appeared after the verifier had detected the new fault.
- full verifier log: `/tmp/claude-1001/bdk501-no-pageflip-submit-verify-20260604.log`.

Conclusion:

- compositor page flip and scanout are not necessary for the BDK-501 repro.
- the required compositor-side contribution is narrower: a second-process GLES/etnaviv submit is enough when host
  rendering is active and cross-process render submits are not serialized.
- this further narrows the likely failure boundary to host/compositor etnaviv submit or MMU-context interleaving, rather
  than KMS page-flip handling or display scanout.

### Compositor post-flush `glFinish` as a necessary trigger

Status: ruled out as necessary for the current repro.

Hypothesis:

- the clear-only/no-page-flip diagnostic might have failed because the compositor still called an extra post-flush
  `glFinish`.
- if waiting for compositor GPU completion were required, then skipping that post-flush `glFinish` should avoid the MMU
  fault.

Tested change:

- used profile `219-link`.
- deployed both configured packages:
  - `core`: `/nix/store/dc59fs5j6689imijwxyk40l3h2m8ls8g-bmc-bmc-core`
  - `widget-mining-info`: `/nix/store/mabw48jvs84ndzdyra0g6qc4zsm5mwfq-bmc-widget-mining-info`
- set `BMC_GPU_RENDER_LOCK_PATH=""` to disable the app-level GPU render lock.
- set `BMC_DIAG_COMPOSITOR_CLEAR_ONLY_SUBMIT=1`.
- set `BMC_DIAG_COMPOSITOR_SKIP_PAGE_FLIP=1`.
- set `BMC_DIAG_COMPOSITOR_SKIP_POST_FLUSH_FINISH=1`.
- left `ETNA_MESA_DEBUG` unset.
- kept host rendering and normal widget lifecycle/swipe-neighbor acquisition active.
- compositor `render_scene()` still bound a back buffer, cleared it, finished the GLES frame, and called `glFlush`; it
  then skipped the diagnostic post-flush `glFinish` and skipped `page_flip()`.

Result:

- verifier failed on run 1 after the first wiggle:
  - `faults_before=192`
  - `faults_after_startup=192`
  - `faults_after_wiggle_1=195`
  - fault address: `0xffffe9c0`
- compositor diagnostic evidence:
  - clear-only submit log count: `31`
  - skip-page-flip log count: `31`
  - skip-post-flush-finish warning count: `1`
  - skip-compositor-submit log count: `0`
- memory/CMA at failure detection was not exhausted:
  - `MemAvailable=116372 kB`
  - `CmaFree=43404 kB`
- host `Dropped destroyed buffer`: `0`.
- compositor destroyed-buffer drops appeared after the verifier had detected the new fault.
- full verifier log: `/tmp/claude-1001/bdk501-no-pageflip-no-post-finish-verify-20260604.log`.

Conclusion:

- compositor-side post-flush `glFinish` is not necessary for the BDK-501 repro.
- a second-process compositor GLES frame finish plus `glFlush` submit remains sufficient when host rendering is active
  and cross-process render submits are not serialized.
- this reinforces that the relevant difference is submit ordering/interleaving, not waiting for compositor GPU
  completion.

### Live BMC compositor/Wayland lifecycle as necessary for any concurrent-client GPU recovery

Status: ruled out for etnaviv GPU recovery, but not for the exact MMU-address form.

Hypothesis:

- the BDK-501 failure might require live BMC compositor logic, Wayland buffer lifecycle, scene state, or widget process
  orchestration.
- if so, two independent non-BMC EGL clients replaying the captured host trace concurrently should not perturb etnaviv.

Tested change:

- used profile `220-link`.
- deployed current core, configured `widget-mining-info`, and `nixpkgs-apitrace`.
- stopped `bmc-openwrt`, `bmc-wasm-host`, and `bmc-wasm-thin`.
- replayed `/var/log/bmc/apitrace/bmc-wasm-host.20260603T193510.trace` with:
  - `LD_LIBRARY_PATH=/nix/store/dgb9h7cyagwcr6w4bxwv3sw8sxgrimdy-mesa-armv7l-unknown-linux-gnueabihf-26.0.5/lib`
  - `WAFFLE_PLATFORM=gbm`
  - `WAFFLE_GBM_DEVICE=/dev/dri/renderD128`
  - `/run/current-profile/bin/eglretrace --headless --singlethread`

Result:

- single replay smoke passed:
  - `faults_before=195`
  - `faults_after=195`
- two concurrent replay processes both exited `0`, but produced a new etnaviv recovery line on run 1:
  - `faults_before=195`
  - `faults_after_run_1=196`
  - new dmesg line: `[47925.247393] etnaviv-gpu 59000000.gpu: recover hung GPU!`
- no new paired `MMU fault status` or `MMU 0 fault addr` line was captured for this recovery.
- serial control passed:
  - two replays back-to-back for 10 rounds.
  - `faults_before=196`
  - `faults_after_run_10=196`
- concurrent replay with `ETNA_MESA_DEBUG=flush_all` also passed:
  - two concurrent replay processes for 10 rounds.
  - `faults_before=196`
  - `faults_after_run_10=196`
- concurrent replay with `ETNA_MESA_DEBUG=cflush_all` failed on run 1:
  - both replay processes exited `0`.
  - `faults_before=196`
  - `faults_after_run_1=197`
  - new dmesg line: `[48330.214907] etnaviv-gpu 59000000.gpu: recover hung GPU!`
  - no new paired `MMU fault status` or `MMU 0 fault addr` line was captured for this recovery.
- launching the same two replay processes concurrently but wrapping each in a shared `/usr/bin/flock` passed:
  - `ETNA_MESA_DEBUG` unset.
  - 10 rounds.
  - `faults_before=197`
  - `faults_after_run_10=197`
- memory/CMA was healthy in both concurrent and serial replay tests.
- full logs:
  - `/tmp/claude-1001/bdk501-concurrent-eglretrace-device-20260604.log`
  - `/tmp/claude-1001/bdk501-serial-eglretrace-device-20260604.log`
  - `/tmp/claude-1001/bdk501-concurrent-eglretrace-flush-all-device-20260604.log`
  - `/tmp/claude-1001/bdk501-concurrent-eglretrace-cflush-all-device-20260604.log`
  - `/tmp/claude-1001/bdk501-flocked-eglretrace-device-20260604.log`

Conclusion:

- live BMC compositor/Wayland/widget lifecycle is not necessary to trigger an etnaviv GPU recovery from overlapping
  independent EGL clients.
- this does not fully rule out BMC lifecycle as necessary for the exact BDK-501 MMU-address fault, because the replay
  reproduced a recovery-only line.
- the serial control rules out the trace workload alone; the failing variable was concurrent independent client replay.
- the `flush_all` control further points at Mesa/etnaviv submit granularity as the variable, not memory pressure or
  trace replay correctness.
- the `cflush_all` failure rules out cache flushing alone as sufficient to prevent the replay recovery.
- the `flock` pass confirms that explicit userspace serialization is sufficient to prevent the standalone replay
  recovery without changing the GL trace or Mesa debug flags.

### Original fault apitrace as necessary for standalone concurrent replay recovery

Status: ruled out for recovery-only replay events.

Hypothesis:

- the concurrent replay recovery might depend on the shorter `/var/log/bmc/apitrace/bmc-wasm-host.20260603T193510.trace`
  captured during the original live MMU-address fault.
- if so, concurrently replaying the earlier clean trace should not perturb etnaviv.

Tested change:

- stopped BMC processes.
- used the earlier clean trace: `/var/log/bmc/apitrace/bmc-wasm-host.20260603T193419.trace`.
- replayed through the same GBM/render-node `eglretrace --headless --singlethread` setup.

Result:

- single clean-trace control passed:
  - `faults_before=198`
  - `faults_after=198`
- concurrent clean-trace replay failed on run 1:
  - both replay processes exited `0`.
  - `faults_before=197`
  - `faults_after_run_1=198`
  - new dmesg line: `[48917.241498] etnaviv-gpu 59000000.gpu: recover hung GPU!`
  - no new paired `MMU fault status` or `MMU 0 fault addr` line was captured for this recovery.
- memory/CMA was healthy:
  - concurrent before: `MemAvailable=133948 kB`, `CmaFree=57456 kB`.
  - concurrent after run 1: `MemAvailable=169012 kB`, `CmaFree=92528 kB`.
  - single-control before: `MemAvailable=168688 kB`, `CmaFree=92168 kB`.
  - single-control after: `MemAvailable=165480 kB`, `CmaFree=87828 kB`.
- full logs:
  - `/tmp/claude-1001/bdk501-concurrent-eglretrace-clean-device-20260604.log`
  - `/tmp/claude-1001/bdk501-single-eglretrace-clean-device-20260604.log`

Conclusion:

- the shorter original fault apitrace is not necessary for a standalone concurrent replay recovery.
- single-client replay of the clean trace is clean, so the failed variable remains overlapping independent EGL clients.
- this still does not prove the clean trace can reproduce the exact live MMU-address fault form, because this replay
  result was another recovery-only event.

### GBM/render-node replay path as necessary for standalone concurrent replay recovery

Status: ruled out for recovery-only replay events.

Hypothesis:

- the standalone concurrent replay recovery might depend on Waffle's GBM platform or explicit
  `WAFFLE_GBM_DEVICE=/dev/dri/renderD128` replay path.
- if so, surfaceless concurrent replay should not perturb etnaviv.

Tested change:

- stopped BMC processes.
- used the original fault trace: `/var/log/bmc/apitrace/bmc-wasm-host.20260603T193510.trace`.
- set `WAFFLE_PLATFORM=surfaceless_egl`.
- did not set `WAFFLE_GBM_DEVICE`.
- launched two independent `eglretrace --headless --singlethread` processes concurrently.

Result:

- concurrent surfaceless replay failed on run 1:
  - both replay processes exited `0`.
  - `faults_before=198`
  - `faults_after_run_1=199`
  - new dmesg line: `[49090.780385] etnaviv-gpu 59000000.gpu: recover hung GPU!`
  - no new paired `MMU fault status` or `MMU 0 fault addr` line was captured for this recovery.
- memory/CMA was healthy:
  - before: `MemAvailable=165400 kB`, `CmaFree=86192 kB`.
  - after run 1: `MemAvailable=164188 kB`, `CmaFree=85432 kB`.
- full log: `/tmp/claude-1001/bdk501-concurrent-eglretrace-surfaceless-device-20260604.log`.

Conclusion:

- the GBM/render-node replay path is not necessary for standalone concurrent replay recovery.
- overlapping independent EGL clients remain the relevant variable.
- this still does not prove surfaceless replay can reproduce the exact live MMU-address fault form, because this replay
  result was another recovery-only event.

### `linux-drm-syncobj-v1` as a Smithay-backed explicit-sync path

Status: ruled out on the current device kernel/driver stack.

Hypothesis:

- Smithay already implements `linux-drm-syncobj-v1`; if the device DRM nodes support syncobj timeline/eventfd, using
  Smithay's syncobj path could replace the current host-side blocking fence wait and avoid hand-rolling the older
  `zwp_linux_explicit_synchronization_v1` fd-fence protocol.

Tested change:

- built a small ARMv7 DRM ioctl probe in `/tmp/claude-1001/drm_syncobj_probe.c`.
- copied it to the device as `/tmp/drm_syncobj_probe`.
- checked the render node and both card nodes:
  - `/dev/dri/renderD128`
  - `/dev/dri/card1`
  - `/dev/dri/card0`
- the probe checks:
  - `DRM_CAP_SYNCOBJ`
  - `DRM_CAP_SYNCOBJ_TIMELINE`
  - `DRM_IOCTL_SYNCOBJ_CREATE`
  - `DRM_IOCTL_SYNCOBJ_TIMELINE_WAIT`
  - `DRM_IOCTL_SYNCOBJ_EVENTFD`

Result:

```text
=== renderD128 ===
device: /dev/dri/renderD128
DRM_CAP_SYNCOBJ: value=0
DRM_CAP_SYNCOBJ_TIMELINE: value=0
DRM_IOCTL_SYNCOBJ_CREATE: failed errno=95 (Operation not supported)
=== card1 ===
device: /dev/dri/card1
DRM_CAP_SYNCOBJ: value=0
DRM_CAP_SYNCOBJ_TIMELINE: value=0
DRM_IOCTL_SYNCOBJ_CREATE: failed errno=95 (Operation not supported)
=== card0 ===
device: /dev/dri/card0
DRM_CAP_SYNCOBJ: value=0
DRM_CAP_SYNCOBJ_TIMELINE: value=0
DRM_IOCTL_SYNCOBJ_CREATE: failed errno=95 (Operation not supported)
```

Conclusion:

- the current Linux `5.10.176` etnaviv/DRM stack does not expose syncobj support on the tested DRM nodes.
- Smithay's `linux-drm-syncobj-v1` global should not be advertised on this device; its `supports_syncobj_eventfd` gate
  would fail after syncobj creation/eventfd probing.
- `linux-drm-syncobj-v1` is therefore not a viable near-term replacement for the current producer-side fence wait on
  this device image.
- this is not a missing `CONFIG_DRM_SYNCOBJ` knob. Linux 5.10 has DRM core syncobj UAPI pieces:
  - `include/uapi/drm/drm.h` defines `DRM_CAP_SYNCOBJ`, `DRM_CAP_SYNCOBJ_TIMELINE`, `DRM_IOCTL_SYNCOBJ_CREATE`, and
    timeline wait/query/transfer/signal ioctls.
  - `drivers/gpu/drm/drm_ioctl.c` reports `DRM_CAP_SYNCOBJ` from `drm_core_check_feature(dev, DRIVER_SYNCOBJ)` and
    `DRM_CAP_SYNCOBJ_TIMELINE` from `DRIVER_SYNCOBJ_TIMELINE`.
  - `drivers/gpu/drm/drm_file.c` only opens/releases per-file syncobj state when `DRIVER_SYNCOBJ` is set.
- etnaviv in Linux 5.10 does not opt into that driver feature:
  - `drivers/gpu/drm/etnaviv/etnaviv_drv.c` sets `driver_features = DRIVER_GEM | DRIVER_RENDER`.
  - by contrast, other 5.10 DRM render drivers such as `lima` and `msm` set `DRIVER_SYNCOBJ`, showing the feature exists
    in the kernel but is driver-specific.
- Linux 5.10 also lacks the newer `DRM_IOCTL_SYNCOBJ_EVENTFD` UAPI that Smithay's `supports_syncobj_eventfd` path
  expects. That ioctl exists in the newer local `~/src/linux` tree, but not in `~/src/linux-5.10.y`.
- the remaining explicit-sync options are:
  - the older `zwp_linux_explicit_synchronization_v1` acquire-fence FD protocol, if `EGL_ANDROID_native_fence_sync` can
    export producer fences.
  - a small internal acquire-fence handoff using the same fence FD primitive.
  - keeping the current producer-side GL fence wait.

### `EGL_ANDROID_native_fence_sync` as producer fence source

Status: confirmed available on the current device EGL stack.

Hypothesis:

- even though DRM syncobj is unavailable on this etnaviv/Linux 5.10 stack, Mesa EGL may still support exporting a
  producer `dma_fence` as a native fence FD via `EGL_ANDROID_native_fence_sync`.
- if available, that FD can be used with the older `zwp_linux_explicit_synchronization_v1` acquire-fence protocol, or
  with an internal acquire-fence handoff.

Tested change:

- built a small ARMv7 EGL/GBM/GLES probe in `/tmp/claude-1001/egl_native_fence_probe.c`.
- copied it to the device as `/tmp/egl_native_fence_probe`.
- initialized EGL on `/dev/dri/renderD128` with a GBM display.
- created a surfaceless OpenGL ES context.
- loaded and called:
  - `eglCreateSyncKHR`
  - `eglDestroySyncKHR`
  - `eglDupNativeFenceFDANDROID`
- created an `EGL_SYNC_NATIVE_FENCE_ANDROID`, flushed GL, and duplicated it to a native fence FD.

Result:

```text
device: /dev/dri/renderD128
client_ext EGL_EXT_platform_base: 1
client_ext EGL_KHR_platform_gbm: 1
client_ext EGL_MESA_platform_gbm: 1
egl_version: 1.4
egl_vendor: Mesa Project
egl_version_string: 1.4
display_ext EGL_KHR_fence_sync: 1
display_ext EGL_ANDROID_native_fence_sync: 1
display_ext EGL_KHR_surfaceless_context: 1
proc eglCreateSyncKHR: 1
proc eglDestroySyncKHR: 1
proc eglDupNativeFenceFDANDROID: 1
egl_native_fence_fd: ok fd=9
result: native_fence_usable
```

Conclusion:

- the widget/wasm-host producer side can create native fence FDs after GL/FemtoVG rendering.
- the older fd-fence explicit-sync protocol remains technically viable on this device, even though DRM syncobj is not.
- the next implementation direction should be:
  - add native fence FD export to `bmc-widget::egl::EglContext`.
  - add acquire-fence submission to the widget surface API.
  - make the compositor consume/wait acquire fences before sampling dirty DMA-BUFs.
  - keep the current blocking GL completion path as fallback when explicit sync is not advertised or fence export fails.
