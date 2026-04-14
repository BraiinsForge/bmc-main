# Analysis: split `runtime_wasmi.rs`

**Reviewed:** 2026-04-13  
**Branch:** `jku/BDK-331/regression-testing`  
**HEAD:** `9b01f9b1b201a3eda1863644eea9ce6474c3d62e`

## Summary

`bmc-wasm-runtime/src/runtime_wasmi.rs` is too large and mixes too many
independent responsibilities.

Current size:

- file size: **4802 lines**
- host imports registered in one function: **58** `linker.func_wrap(...)`
- host import registration block: **1687 lines**
- runtime lifecycle + host-to-guest event delivery block: **1275 lines**
- background networking/discovery/http worker block: **972 lines**

This is no longer a "runtime backend" file. It is currently all of these at
once:

- public runtime API
- wasmi module instantiation and SDK validation
- guest import registration
- guest memory marshalling
- frame scheduling and fuel handling
- host-to-guest event delivery
- networking background workers
- SSDP/mDNS/HTTP protocol helpers
- image decode / KV / TLS helper code
- calendar/time formatting helpers
- unit tests

The file has already started to split naturally:

- host state lives in `bmc-wasm-runtime/src/host_api.rs`
- runtime limits live in `bmc-wasm-runtime/src/runtime_limits.rs`
- XML indexing now lives in `bmc-wasm-runtime/src/xml.rs`

The next step should follow the same pattern instead of letting
`runtime_wasmi.rs` keep absorbing unrelated code.

## Current responsibility map

### 1. Runtime types and initialization

`bmc-wasm-runtime/src/runtime_wasmi.rs:37-239`

Owns:

- `RenderStatus`
- `FetchInterceptor`
- `FetchObserver`
- `RuntimeConfig`
- `WasmWidgetRuntime`
- wasmi engine/store/module setup
- SDK version validation

This is the real "runtime backend" core.

### 2. Guest import registration

`bmc-wasm-runtime/src/runtime_wasmi.rs:261-1947`

One function, `register_host_functions()`, registers 58 imports spanning:

- drawing and interaction
- frame control
- images/icons/bitmaps
- fetch
- websocket/tcp/tls
- mDNS / SSDP / UDP broadcast
- KV persistence
- HTTP listener
- logging
- JSON / XML
- formatting / dates / timezones / calendars

This is the largest single responsibility and the main readability problem.

### 3. Runtime render loop and host-facing API

`bmc-wasm-runtime/src/runtime_wasmi.rs:1957-3231`

Owns:

- `render()`
- cached-tree rendering
- fuel overlays
- time injection
- fixture injection
- renderer accessors
- hit testing accessors
- all `deliver_*` methods
- all `has_*` polling predicates

This is conceptually coherent, but it is too large because the event delivery
code for every protocol is duplicated inline here.

### 4. Guest memory + small helpers

`bmc-wasm-runtime/src/runtime_wasmi.rs:3237-3486`

Owns:

- `read_string`
- `read_bytes`
- `read_optional_bytes`
- `parse_headers`
- `write_to_wasm`
- XML lookup glue
- KV validation
- image decode limits
- TLS config builder entrypoints

These helpers are used by several unrelated domains and should not live at the
bottom of the runtime file.

### 5. Background workers and protocol parsing

`bmc-wasm-runtime/src/runtime_wasmi.rs:3489-4460`

Owns:

- websocket background thread
- plain TCP background thread
- TLS background thread
- mDNS browse thread
- SSDP search thread
- UDP broadcast thread
- HTTP listener thread
- SSDP response parsing
- HTTP fetch implementation

This is effectively a networking subsystem embedded inside the runtime file.

### 6. Time/calendar pure helpers

`bmc-wasm-runtime/src/runtime_wasmi.rs:4511-4610`

Owns:

- RRULE expansion
- timezone conversion

These are pure helpers and can live independently.

### 7. Inline unit tests

`bmc-wasm-runtime/src/runtime_wasmi.rs:4686-4802`

Current tests mostly target helper functions:

- KV validation
- image decode limits
- TLS config construction
- XML lookup behavior

These tests belong next to the helper modules they exercise.

## Why this matters

The problem is not just line count.

### Reviewability

Small changes to one domain get buried in unrelated code. A review about XML,
TLS, or event delivery still forces the reviewer through a 4.8k-line file.

### Ownership boundaries

There is no clear answer to "where does fetch behavior live?" or "where do we
add a new host import?" because several answers are simultaneously true.

### Testability

Pure helpers, import registration, runtime lifecycle, and background workers
need different test styles. Co-locating them in one file discourages focused
tests and pushes everything toward ad hoc bottom-of-file unit tests.

### Safe refactoring

The import registration closures repeatedly duplicate memory reads, guest
allocation, and event delivery patterns. With everything in one file, it is
hard to extract common helpers without creating another large internal tangle.

### Compile and merge friction

Even unrelated edits touch the same file, so conflicts are more likely and the
module becomes a hotspot.

## Recommended split

Keep `bmc-wasm-runtime/src/runtime.rs` as the public facade module, consistent
with the repository's Rust 2018 module style. Replace the single
`runtime_wasmi.rs` backend blob with focused files under `bmc-wasm-runtime/src/runtime/`.

### Target structure

```text
bmc-wasm-runtime/src/
├── runtime.rs                    # public facade, reexports
└── runtime/
    ├── backend.rs                # WasmWidgetRuntime, RuntimeConfig, RenderStatus, new(), render()
    ├── memory.rs                 # read/write guest memory, header parsing, alloc/copy helpers
    ├── delivery.rs               # deliver_* methods, has_* predicates, fixture injection
    ├── imports.rs                # register_host_functions() entrypoint
    ├── imports/
    │   ├── render.rs             # drawing, frame control, interaction, icon/bitmap/image imports
    │   ├── data.rs               # logging, KV, JSON, XML, formatting, date/time/calendar imports
    │   └── network.rs            # fetch/ws/socket/mdns/ssdp/udp/http imports
    ├── background.rs             # shared background helpers / reexports
    ├── background/
    │   ├── fetch.rs              # do_fetch()
    │   ├── socket.rs             # ws/tcp/tls threads, TLS config, verifier, connect helpers
    │   ├── discovery.rs          # mdns/ssdp/udp threads + SSDP parsing helpers
    │   └── http.rs               # http_listener_thread()
    └── time.rs                   # number formatting, RRULE expansion, timezone conversion
```

## Why this split

### `backend.rs`

This keeps the public runtime surface in one place:

- runtime construction
- render loop
- fuel/dead-widget handling
- core accessors

That is the part callers actually depend on.

### `imports/*`

The guest ABI is already domain-shaped. The file should reflect that.

- render imports change with GPU/UI work
- data imports change with parsing/formatting/KV work
- network imports change with transport/discovery work

The single `imports.rs` file should only glue these together.

### `delivery.rs`

All `deliver_*` methods follow the same pattern:

1. drain host-side channels
2. optionally record fixture events
3. allocate guest memory
4. call the guest callback export
5. clean up closed resources

That shared pattern should live together. Today it is mixed into the runtime
public API block, making both harder to read.

### `memory.rs`

Guest memory marshalling is cross-cutting infrastructure. It should not be
copied inline in each host import or event delivery path.

This module is also the right place for a small helper like:

- `alloc_and_copy_to_guest(...)`
- `call_guest_with_bytes(...)`

Those would reduce a large amount of repeated `__alloc` + bounds-check +
copy-to-memory code.

### `background/*`

The runtime file should not contain socket loops, HTTP servers, SSDP parsing,
and TLS verifier details. These are host-side services, not runtime core logic.

Splitting by transport/discovery protocol is preferable to one giant
`network.rs`, because the responsibilities are already distinct and likely to
evolve separately.

### `time.rs`

The calendar/time helpers are pure and stable. They should be cheap to test and
review in isolation.

## Boundaries to preserve

The split should be structural, not architectural churn.

Keep these rules:

- do not change the guest ABI names (`host_*`, `__on_*`, `__alloc`)
- do not redesign `HostState` during the split unless a moved module clearly
  needs a small helper method
- do not introduce deep generic abstractions just to deduplicate everything
- do not merge unrelated protocols under one trait hierarchy
- keep `runtime.rs` as the stable public entrypoint
- keep already-extracted modules (`host_api.rs`, `runtime_limits.rs`, `xml.rs`)
  where they are

## Recommended first-pass refactors

Some extractions pay off immediately and reduce later churn:

### 1. Introduce guest memory helpers early

Before splitting by domain, extract the repeated patterns for:

- reading strings/bytes from guest memory
- writing strings/bytes to guest memory
- allocating guest memory for callback payloads

This will shrink both import closures and `deliver_*` methods.

### 2. Extract delivery before background workers

`deliver_*` methods are closer to the public runtime API than the worker
threads. Moving them first makes the remaining backend file easier to reason
about and clarifies what host-side services still need to be extracted.

### 3. Split imports by domain, not alphabetically

The import registration function is too large because it mixes domains. Keep the
registration grouped by runtime concern instead of one-file-per-host-function.

### 4. Move tests with the modules they exercise

Pure helper tests should leave the runtime backend file as soon as those helpers
move.

## Non-goals

This split should not try to solve every duplication issue at once.

Specifically out of scope for the first pass:

- redesigning fixture recording/replay
- replacing channel-based delivery with async runtimes
- changing the renderer ownership model
- rewriting protocol parsing behavior
- converting the runtime into a trait-heavy architecture

## Recommended order of commits

To keep review diffs readable, prefer this commit shape:

1. create `src/runtime/` module skeleton and move pure helpers
2. split import registration into `imports/*`
3. extract delivery code into `delivery.rs`
4. extract background workers into `background/*`
5. final cleanup, docs, and test moves

If a stage grows too large, split by domain rather than mixing structural moves
with behavior changes.

## Conclusion

The correct split is not "break the file into arbitrary chunks". The right goal
is to align modules with the runtime's existing subsystems:

- backend lifecycle
- guest imports
- guest memory marshalling
- host-to-guest delivery
- background networking/discovery/http workers
- pure time/formatting helpers

That gives a structure that is easier to review, easier to test, and easier to
extend without re-creating another 4k-line hotspot.
