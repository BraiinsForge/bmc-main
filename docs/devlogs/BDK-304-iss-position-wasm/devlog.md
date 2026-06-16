# BDK-304 — Promote ISS-position WASM widget to production

Ticket: <https://braiins.atlassian.net/browse/BDK-304> — "Create a replica of ISS position widget on the WASM runtime
with live-rendered attributes."

Working notes for the promotion + fix effort. Kept in its own commits so the history can be squashed/reworded at the
end.

## Goal

Take the existing example widget at `bmc-wasm-runtime/examples/iss-position/` (monolithic, fetches `wheretheiss.at`
directly, crashes/freezes on the Deck) and turn it into a production widget under `widgets-wasm/iss-position/` that:

- pulls its data from nexus instead of the upstream API
- is structured like the `weather` production widget (the reference)
- keeps the locally-rendered 3D globe (the whole point of the port — the server-side original could not do local
  rendering)
- runs stably on the device

## Scope decisions (with Josef)

- ISS only. The SpaceX launch widget (ticket BDK-285, deckfeeder `spacex-launch`, nexus
  `/api/v1/data/spacex/next-launch`) is likely next but out of scope here.
- Destination `widgets-wasm/iss-position/`. Full weather-style refactor, not a minimal port.
- Keep the 3D globe shader render.
- Sequence: (1) relocate + refactor + nexus migration, (2) prove correctness on PC (unit tests + capture/verify
  harness), (3) on-device debug of the freeze/crash.

## Nexus data contract

`GET https://nexus.braiinsforge.com/api/v1/data/iss/position` returns one envelope carrying both position and TLE
(replaces the two direct `wheretheiss.at` calls the example makes):

```
{
    "data": {
        "position": {
            latitude,
            longitude,
            altitude,
            velocity,
            visibility,
            footprint,
            timestamp,
            solar_lat,
            solar_lon
        },
        "tle": { line1, line2 } 
    },
  "cache_age_secs",
  "ttl_secs"
}
```

- `visibility` is the lowercase string `"daylight"` | `"eclipsed"`.
- `velocity` km/h, `altitude`/`footprint` km, `timestamp` unix seconds.
- Nexus polls upstream every 30 min (ttl 1800).

## Suspected device-crash hotspots (to confirm in phase 3, not assume)

- Full variant drives a 30 fps loop (`request_frame_after(33)`).
- Every frame recomputes the ground track = 60 SGP4 propagations (`compute_ground_track`, `ORBIT_POINTS = 60`) plus one
  for the live position. The orbit barely moves second-to-second; recomputing 30×/s is near-pure waste.
- Textured sphere shader + atmosphere + per-frame `.transition()` on the embedded GPU.

The obviously-correct structural wins (cache the SGP4 track, decouple data-rate from frame-rate) fall out of the
refactor; whether they actually cure the crash is a phase-3 question.

## Progress

### Phase 1a — relocation + dependency bump (done)

- Moved the tree to `widgets-wasm/iss-position/` (dropped the 198 MB regenerable `tools/.venv` and `__pycache__`).
- Removed `iss-position` from the examples workspace members; added it to the `widgets-wasm` workspace members. (Name
  must be unique across both roots — the nix widget catalog keys by name across roots.)
- Fixed crate paths: SDK dep `../../sdk` → `../../bmc-wasm-runtime/sdk`; manifest `$schema` 3-up → 2-up; justfile
  `mod root` 3-up → 2-up.
- New production `uid` (`0a3973c9-…`) replacing the placeholder.
- `sgp4` 2.3 → 2.4 in the `widgets-wasm` workspace; removed the now-orphaned `sgp4` from the examples workspace. Both
  `Cargo.lock`s regenerated.
- Verified: `cargo build -p iss-position --target wasm32-unknown-unknown` is clean.
- Carried the stale python exclusion: `ty.toml [src] exclude` pointed at `bmc-wasm-runtime/examples/*/tools/`; added
  `widgets-wasm/iss-position/tools/` so the root ty check stays off the texture-tool uv project (own venv/pyproject).
- Open hygiene note: the tool `.venv` was ignored only via a now-dangling global excludesfile (`/home/pepa/.gitignore`);
  in-repo rules cover `__pycache__` but not `.venv`. Repo-wide gap — pending decision on root vs local `.gitignore`.

### Phase 1b — module refactor + nexus migration (next)

### Phase 2 — PC correctness

### Phase 3 — on-device debug
