# bmc-storybook

Visual catalog and interactive playground for widget SDK renderer components.

## Purpose

Provides a desktop (egui + femtovg) application that renders stories through the real `bmc-render` GPU pipeline. Widget
developers use it to inspect, tweak, and document UI components with interactive knobs.

Supports two modes:

- **Static** (`just storybook`) -- stories compiled in via `include!()`, discovered by `build_stories.rs` which reads
  `[workspace] members` from the root `Cargo.toml` and recursively scans every member directory for `*.stories.rs`
  files. Adding a new workspace member never silently misses stories.
- **Hot-reload** (`just storybook-hot`, `--hot-reload`) -- stories loaded from a cdylib `.so` at runtime via
  `libloading`, with `notify` watching for rebuilds.

Stories are registered through the `inventory` crate and rendered in an egui sidebar with fuzzy search
(`sublime_fuzzy`).

## Hot-reload ABI caveat

The cdylib (`bmc-storybook-stories`) exports `__init_registrars()` and `__story_entries()` as `extern "Rust"` — **not**
`extern "C"`. The return type `StoryManifest` contains `Vec<StoryEntry>` and `Vec<StoryGroupMeta>`, neither of which has
a stable C ABI. This is sound only because cargo compiles the shell and the cdylib with the *same* rustc, toolchain
version, and codegen settings within one workspace build (`just storybook-hot` does this). Running
`cargo build -p bmc-storybook` and `cargo build -p bmc-storybook-stories` separately under different toolchains (e.g.
switching `rustup default` between commands) would produce silent UB at the ABI boundary.

If this becomes a real risk (e.g. someone wants to reload a `.so` built outside the workspace), the boundary needs to
move to `#[repr(C)]` plus pointer + length: `extern "C" fn __story_entries() -> *const StoryEntry` with a separate
length getter. Until then, the workspace-build invariant is the contract.

## Boundaries

**IS its responsibility:**

- Story discovery and registration (build-time scan + inventory)
- egui shell (sidebar, knobs UI, preview panel)
- Hot-reload infrastructure (cdylib loading, file watching)
- Rendering stories through the real `bmc-render` pipeline

**IS NOT its responsibility:**

- The rendering engine itself (that is `bmc-render`)
- Widget SDK types and tree building (that is `bmc-wasm-sdk`)
- Knob/story API types (that is `bmc-storybook-api`)
- Story proc macros (that is `bmc-storybook-macros`)
