# BDK-546 — Asset-cache observability: verification evidence

Proof that the per-instance flash asset cache (write-at-decode, restore-on-wake, dormant RAM reclaim) works end-to-end,
captured for the MR. All events ride the profiling channel (`bmc_render::profile`, target `mesh::profile`), gated behind
the `profiling` feature, so they appear only on a debug/profiling build.

## How to reproduce

- Build: debug/profiling (compositor + wasm host `profiling` feature on).
  - VM: `just virt::run image-cache` (always debug).
  - Device: `nix run .#deck deploy --device <ip> --profile debug`.
- Config: a **4-scene** image config (`bmc-virt/data/configs/image-cache.json`), one fullscreen image widget per scene,
  distinct URLs, `refresh_seconds: 300`.
- Cycling is on by default (`SceneCycling::default()` — enabled, 30 s, slide), so the scenes auto-rotate with no extra
  config.

## Why 4 scenes (not 2)

`widget_tracker::lifecycle_states` marks every widget `Dormant`, then promotes the active scene to `Visible` and
**both** ±1 neighbours to `Prepared`. So a scene only reaches `Dormant` when it is neither active nor an immediate
neighbour:

- 2 scenes → the off-screen scene is always the neighbour → never `Dormant`.
- 3 scenes → active + 2 neighbours = all three → never `Dormant`.
- **4 scenes → the opposite scene is `Dormant`** (smallest cycle that reaches it).

The eviction/restore fire on the `has_render_target` flip (`bmc-wasm-host::lifecycle::lifecycle_hook`):
`Prepared→Dormant` → Dormant hook (evict), `Dormant→Prepared` → Wake hook (restore). Rotation produces exactly one of
each per step.

## VM evidence (4-scene image-cache, debug build)

Source: `/var/log/bmc/run-bmc-wasm-host-sdk-v0.log`.

Write-at-decode (per-instance bucket, front-loaded at startup):

```
INFO mesh::profile: cache write tag=image written=2457600 entries=1 bytes=2457658
```

One rotation step — dormant eviction then wake-restore:

```
slot lifecycle applied previous=Prepared current=Dormant render_target=false
lifecycle: render target released state=Dormant
mesh::profile: dormant eviction namespace=3 evicted=1 resident=7372800

slot lifecycle applied previous=Dormant current=Prepared render_target=true
lifecycle: render target allocated state=Prepared w=1280 h=480
mesh::profile: cache restore hit tag=image age_ms=29527 resident=7372800
```

Resident-bytes oscillation across rotation (texture = 1280×480×4 = 2,457,600 B):

| resident  | textures | meaning              |
| --------- | -------- | -------------------- |
| 9,830,400 | 4        | all resident         |
| 7,372,800 | 3        | one dormant, evicted |
| 4,915,200 | 2        | two dormant, evicted |

Each `dormant eviction` drops resident by one texture; each `cache restore hit` raises it by one — the RAM is genuinely
reclaimed and rebuilt from flash.

Key signals:

- **0 restore misses** — every wake hit the cache; no refetch on the wake path.
- Outbound image fetches clustered only at startup (16:57:54–16:58:05); none correlated with later wakes.
- `age_ms` climbs monotonically `29527 → 59528 → 80082 → 110503 → 151144 → 181152` (≈ +30 s, the rotation period) —
  every wake reads the *same* blob written once at startup, not a refetch.
- **`vm_rss_kb` flat at ~56,768 kB** across every cycle — no leak from repeated dormancy/wake; the reclaim holds.
- Buckets on flash: `/mnt/data/bmc/widget-cache/<uuid>-<extent>/image.blob`, one per widget instance.

## Device evidence

Real Deck (`braiins,stm32mp157c-ii3-bmc1`), `nix run .#deck deploy --profile debug`, with the 4-image config merged into
`/etc/bmc_config.json` (scene cycling + accounts preserved). Source: the **live** host log
`/var/log/bmc/run-bmc-wasm-host-sdk-v0.log` — not `bmc-wasm-host.log`, which is a stale legacy file.

Tallies after rotating the scenes:

| event             | count |
| ----------------- | ----- |
| cache write       | 4     |
| dormant eviction  | 48    |
| cache restore hit | 44    |
| restore miss      | 0     |

```
dormant eviction namespace=2 evicted=1 resident=7372800
cache restore hit tag=image age_ms=223506 resident=9830400
dormant eviction namespace=4 evicted=1 resident=4915200
cache restore hit tag=image age_ms=223629 resident=7372800
```

- Same texture footprint as the VM (2,457,600 B each); resident oscillates
  `9,830,400 (4) ↔ 7,372,800 (3) ↔ 4,915,200 (2)` as widgets evict on `Dormant` and restore on `Wake`.
- **0 restore misses** across 44 wakes; `age_ms` climbs monotonically (…141852 → 223825…) — every wake reuses the cached
  blob, no refetch.
- `vm_rss_kb` bounded at 48,092–50,492 kB across all 48 evict / 44 restore cycles — no leak.
- Buckets on flash: `/mnt/data/bmc/widget-cache/<uuid>-full/image.blob`, one per widget instance.
