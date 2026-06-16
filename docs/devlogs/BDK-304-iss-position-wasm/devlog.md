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

### Phase 1b — module refactor + nexus migration (done)

- Split the monolithic `lib.rs` into `model.rs` (nexus payload + `TryFrom`), `orbit.rs` (pure SGP4/projection math),
  `render.rs` + `render/{panels,globe}`, and a thin wasm-glue `lib.rs`.
- `orbit.rs` functions take `now_unix`/`&Tle` as inputs (no clock, no globals), so the orbital path is unit-tested on
  the host — 9 tests green.
- Nexus migration: one `register_poll` fetch of `/api/v1/data/iss/position` replaces the two direct `wheretheiss.at`
  calls; position + TLE arrive together, so the separate TLE fetch and the "wait for TLE" state machine are gone.
- Keep-last-good on a failed refresh (the globe keeps propagating from the cached TLE) instead of dropping to an error
  state; hard error only before the first load. No "stale" banner — the displayed position stays accurate via local
  propagation, so a staleness banner would mislead.
- TTL-driven refresh: next poll aligns with the remaining nexus cache lifetime (`ttl_secs`/`cache_age_secs`), floored at
  60 s, baseline 30 min.
- Dropped the debug-only scaffolding (`DEBUG_LANDMARKS`, `TIME_SPEED`, `project_point_to_globe`) and the misleading
  "Next update" countdown row.
- nexus also carries `altitude`/`footprint`/`timestamp`; left commented in `model.rs` to document the wire contract, not
  surfaced by any variant yet.
- `just validate-wasm` green; `sgp4` 2.4 validates TLE checksums (the test TLE needed correct check digits).

### Phase 1c — SDK units layer + table widening (done)

Comparing against the original Figma design (node 12164-106669) showed the server-image platform had cut capabilities
the local runtime can restore. Decisions (with Josef):

- **SDK units layer.** New `Length`/`Speed` dimensional newtypes in `bmc-wasm-runtime/sdk/src/units.rs` — named by
  dimension, units only as `from_*`/`as_*` I/O, canonical SI internally. `.format()` delegates to the host formatters so
  localisation stays in one place. Private module, public types (the `units` name would clash with the `lib/units` crate
  until that folds in later). First, focused slice; weather/`lib-units` migrate separately.
- **Host `format_distance`.** Added to the host formatter family (`runtime/imports/data.rs`) + `format_distance!` in the
  SDK — the host had no length formatter (km↔mi). Per-format work stays a cheap host FFI; wasm never pulls `core::fmt`
  (`fmt!` is `ufmt`-backed, `.format()` returns host bytes).
- **Altitude** restored (nexus already sends it) as a table row — `Length`, localised km/mi.
- **Velocity A1** — `7.66 km/s (27 571 km/h)`: km/s is the universal orbital convention shown to all; the parenthetical
  localises (km/h/mph) via the host. Narrow variants (medium/small) show km/s only.
- **Visibility** is `Sunlit`/`Eclipsed`, dropped on the full variant (the globe terminator already shows it), kept on
  the table-only variants.
- **Next pass in** (the design's headline real-time counter) **deferred**: needs an observer lat/lon, i.e. a `location`
  param, which only pays off with a frontend name→lat/lon autocomplete. Skipped for now; the slot shows Altitude.
- Parsing to newtypes happens once at fetch (`model::TryFrom`); render only formats. Both `just validate` and
  `just validate-wasm` green.

### Phase 2 — PC correctness

The capture fixture (`capture/fixtures/full.jsonl.gz`) and baselines still hold the old `wheretheiss.at` shape — they
must be regenerated for the nexus envelope before `just wasm::verify-all` is meaningful. Baseline re-bless needs a human
look at the rendered globe first.

### Phase 3 — on-device debug
