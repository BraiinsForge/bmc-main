# BDK-437 Memory Reduction Options

Captured: 2026-05-11

## Context

The current measurements point to a mixed failure mode:

- each fullscreen `extruded` flip-clock duplicates about `~9 MB` of private dirty memory
- each widget also adds about `~1.6-2.4 MB` of shared-memory / GPU-visible pressure
- scene transitions add another `~10-15 MB` burst
- the device has only `~112 MB` of effective non-CMA RAM because `128 MB` of the `256 MB` total is reserved for CMA

The result is that `3` heavy fullscreen scenes can sit near the cliff in steady state, then cross into global OOM during
a transition.

## Goal

Reduce the memory cliff enough that:

- `3` fullscreen flip-clock scenes are safe to navigate
- scene transitions do not trigger global OOM
- the solution is compatible with the current widget/compositor model, or clearly justifies changing it

## Option 1: Reduce Transition-Time Overlap

### Idea

Keep fewer live widget renderers active during scene changes.

Examples:

- run only the current scene live
- keep at most one neighbor scene warm
- use cached compositor snapshots for inactive scenes during swipe
- delay spawning or waking the destination scene until transition start/end
- tear down the source scene sooner after transition completion

### Why it helps

This directly targets the proven failure moment: the system does not die during calm startup, it dies when multiple
heavy scenes overlap during navigation.

### Expected savings

- likely recovers most of the `~10-15 MB` transition burst
- may also reduce compositor-side texture and buffer churn

### Pros

- fastest path to a user-visible stability improvement
- keeps widget isolation model mostly intact
- lower implementation risk than architectural renderer sharing

### Cons

- does not remove the underlying per-widget private EGL/GL duplication
- steady-state cost of multiple live scenes may still remain too high
- transitions may become visually less "live" if cached snapshots are used

### Assessment

**Best short-term mitigation.**

## Option 2: Share Render State Across Multiple Flip-Clocks

### Idea

Stop paying one full EGL/GL renderer cost per flip-clock process.

Possible shapes:

- one process hosts multiple flip-clock instances and one EGL context
- a dedicated flip-clock renderer service owns one EGL context and renders many logical clocks
- the compositor renders flip-clocks itself

### Why it helps

The biggest duplicated cost is common per-widget renderer/context state:

- `EglState::new` adds `~3.2 MB`
- `Renderer::new` adds another `~4.0 MB`
- these costs recur in every widget process

### Expected savings

- potentially `~5-9 MB` per additional flip-clock, depending on how much state can be shared in practice
- this is the biggest single lever on duplicated private memory

### Pros

- attacks the real root cause, not just the transition symptom
- scales much better as scene count increases
- likely improves startup cost too

### Cons

- medium to high implementation complexity
- weakens or changes process isolation assumptions
- requires a clearer ownership model for widget lifecycle, params, and surfaces

### Assessment

**Best medium-term structural fix.**

## Option 3: Make Extruded Mode Cheaper

### Idea

Reduce the extra memory cost of the 3D path specifically.

Examples:

- lower-cost extruded rendering path
- fewer or smaller depth/render targets
- simpler mesh or shader setup
- pre-rendered 3D look instead of fully dynamic 3D geometry
- automatic downgrade to flat mode under memory pressure

### Why it helps

Compared with flat mode, extruded mode adds:

- roughly `~2 MB` of extra `/dev/dri/renderD128` private dirty mappings
- higher `Pss_Shmem`
- more GPU/render-path overhead during transitions

### Expected savings

- moderate, especially for the heaviest scenes
- enough to widen the margin, but not enough to solve the common per-widget renderer duplication by itself

### Pros

- focused scope within one widget
- preserves overall multi-process architecture
- can be paired with runtime policy such as "downgrade heavy scenes"

### Cons

- does not solve the main `Renderer::new` / context duplication
- may reduce visual quality or change the widget's look

### Assessment

**Good secondary optimization, not the primary fix.**

## Option 4: Lower CMA Reserve

### Idea

Reduce the kernel CMA reservation from `128 MB` if the real workload does not need that much contiguous memory.

### Why it helps

A `128 MB` CMA reserve leaves only about `~112 MB` of effective non-CMA RAM for everything else. Lowering it would
increase the ordinary RAM budget.

### Expected savings

- increases effective non-CMA headroom directly
- does not reduce widget memory use, but makes the system more tolerant

### Pros

- can improve global OOM margin without touching widget logic

### Cons

- risky without careful validation
- if the GPU/display path really needs the current CMA size, reducing it can trade one failure mode for another
- this treats the platform budget, not the application duplication

### Assessment

**Potential platform-level mitigation, but only after application-side reductions are understood.**

## Option 5: Keep All Scenes Live, But Share More Assets

### Idea

Preserve the current architecture, but deduplicate specific resources:

- content-addressed texture cache in compositor
- shared DMA-BUF-exported glyph textures
- more aggressive texture atlas reuse

### Why it helps

This can cut repeated GPU/shmem content, especially identical digit textures.

### Expected savings

- useful for texture duplication
- does not remove the much larger per-process renderer/context cost

### Pros

- compatible with current process model
- can be generalized to other widgets later

### Cons

- lower payoff than render-state sharing
- adds protocol/cache complexity
- does not solve the `Renderer::new` / Mesa state duplication

### Assessment

**Worthwhile only after the bigger sources are addressed.**

## Recommended Order

### 1. Short-term mitigation

Implement **Option 1** first:

- reduce live overlap during scene transitions
- prefer cached compositor snapshots for inactive scenes

This is the most direct way to stop the OOM-triggering transition burst.

### 2. Structural fix

Design **Option 2** next:

- share one renderer/context across multiple flip-clocks

This is the most effective way to remove the duplicated private memory that pushes the system close to the cliff in the
first place.

### 3. Secondary optimization

Apply **Option 3** where it is cheap:

- reduce the extra extruded-path overhead

This widens the margin further, especially for the heaviest scenes.

### 4. Platform tuning

Evaluate **Option 4** only after the application-side work is measured.

Changing CMA before reducing duplicated renderer state risks masking the real problem or creating a different one.

## Summary

If the question is "what should we do first?", the answer is:

1. stop keeping so many heavy scenes fully live during transitions
2. then remove per-widget renderer/context duplication
3. then trim extruded-mode overhead

That sequence gives the best combination of immediate stability and long-term memory scalability.
