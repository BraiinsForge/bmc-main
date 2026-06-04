# BDK-501 debugging outcome, 2026-06-04

## What we found

- MMUv2 is confirmed active for the affected BFM100 path.
- The BFM100 mining widgets still reproduce the etnaviv MMU fault when the cross-process GPU render lock is disabled.
- Explicit DMA-BUF acquire fences did not prevent the fault. The compositor was using the host buffer only after the
  exported fence, but the fault still reproduced.
- Buffer destruction did not line up as the immediate trigger in the captured failure.
- The issue also reproduced with independent concurrent `eglretrace` processes, while serial replay did not. This points
  away from a compositor-only or Wayland-only bug.
- Host/widget-side `ETNA_MESA_DEBUG=flush_all` avoided the repro, but `cflush_all` did not. The useful part appears to
  be changed Gallium/etnaviv submit granularity, not cache flushing alone.
- Pinning GPU submitters to one CPU core is not equivalent to the lock, because CPU affinity does not serialize
  `DRM_ETNAVIV_GEM_SUBMIT`.
- The current evidence points at an etnaviv submit/MMUv2 context interleaving problem in the kernel/driver/hardware
  stack.

## Current workaround

Keep the cross-process GPU render lock enabled.

- Default lock path: `/run/bmc-gpu-render.lock`
- Override with: `BMC_GPU_RENDER_LOCK_PATH=/some/path`
- Disable only for diagnostics with: `BMC_GPU_RENDER_LOCK_PATH=""`

The lock serializes GPU rendering between `bmc-wasm-host` and the compositor. It is not a neighbor-scene hiding
workaround; all scenes still need to render.

## Current best kernel test

Patch linux-stm etnaviv to set:

```c
static int etnaviv_hw_jobs_limit = 1;
```

in:

```text
drivers/gpu/drm/etnaviv/etnaviv_sched.c
```

Then rerun the no-lock BFM100 mining-widget wiggle repro and the concurrent `eglretrace` repro.

If this passes, the fault depends on multiple in-flight etnaviv hardware jobs. The next narrower test should serialize
only cross-MMUv2-context submits, or strengthen the MMUv2 context-switch/MMU-flush barrier, instead of relying on a
global userspace `flock` as the final fix.
