# Settings every justfile shares.
# They cross an `import` but not a `mod`, so every justfile imports this one.
# Paths anchor on `source_directory()`, which stays here whatever imports it.

# A first recipe is often a `clean` or a `validate`; bare `just` lists instead.
set default-list

# Never the dev shell's tree, or each toolchain rebuilds the other's.
export CARGO_TARGET_DIR := env('CARGO_TARGET_DIR', source_directory() / ".tmp/cargo-target")

# Only this catches all of ruff's cache; the package `cache-dir` configs miss some.
export RUFF_CACHE_DIR := source_directory() / ".tmp/ruff_cache"

export RUST_LOG := env('RUST_LOG', 'bmc_wasm_runtime=debug,testbed=debug,bmc_gallery=info')

export FORCE_COLOR := "1"

# The compositor checks are Linux-only; on darwin nix routes them to a `linux-builder`.
NIX_SYSTEM := if os() == "macos" { "aarch64-linux" } else { "$(nix eval --impure --raw --expr builtins.currentSystem)" }
