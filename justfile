mod manifest 'bmc-widget-manifest/justfile'
mod virt 'bmc-virt/justfile'
mod wasm 'bmc-wasm-runtime/justfile'

CI_TOOLS_REV := "c75e453c0e3fd5fe167a9437b86e48b54c2aa81c"
NIX_SYSTEM := "$(nix eval --impure --raw --expr builtins.currentSystem)"

# Auto-enter the dev shell for recipes that link against system libs
# (e.g. `cargo nextest` needs wayland-client to compile bmc).
# `cargo check`/`clippy` skip linking, so they don't need this wrap.
# No-op when already inside a `nix develop` shell (IN_NIX_SHELL set
# by nix); otherwise wrap the command in `nix develop --command`.
NIX_DEV := if env("IN_NIX_SHELL", "") == "" { "nix develop --command" } else { "" }

# Global env vars

export FORCE_COLOR := "1"

[private]
default:
    @just --justfile {{ justfile() }} --list

# === Quick local validation (default; LLM-friendly) ===

# Fast sanity check, not CI-reproducible (use `validate-full` for that).
validate: format clippy
    just manifest::check-schema
    nix build -L ".#checks.{{ NIX_SYSTEM }}.content"
    nix build -L ".#checks.{{ NIX_SYSTEM }}.docs-wasm"
    @echo "validate: OK"

# Auto-format everything (nix fmt + SVG pass).
format:
    nix fmt
    nix run .#fmt-svg

# Cargo clippy with the workspace's pedantic lints (mem-box caps memory).
clippy:
    scripts/mem-box.sh cargo clippy \
      --profile fast \
      --workspace \
      --all-features \
      --keep-going \
      --all-targets -- \
        -D warnings

# Run nextest for a single crate with mem-box caps (auto-enters nix shell).
test crate:
    {{ NIX_DEV }} scripts/mem-box.sh cargo nextest run -p {{ crate }}

# Pedantic rust diff vs master — stricter than clippy (mem-box caps memory).
rust-pedantic:
    scripts/mem-box.sh nix run "git+ssh://git@gitlab.ii.zone/nix/ci-tools?rev={{ CI_TOOLS_REV }}#check-rust-diff" \
      "$(git merge-base origin/master HEAD)" HEAD

# Compress images under the given paths (default: cwd).
fmt-images *PATHS:
    nix run .#fmt-images -- {{ PATHS }}

# === Reproducible / slow ===

# Full nix-driven checks (matches CI's main stage) - Very heavy
validate-full:
    scripts/mem-box.sh nix flake check -L --option max-jobs 4

# === WASM Runtime ===

# Validate the bmc-wasm-runtime crate (format, lint, clippy, test, build all wasm widget workspaces).
validate-wasm: format validate-wasm-deny validate-wasm-no-fmt
    cargo clippy -p bmc-wasm-runtime --all-targets --features testbed -- -D warnings
    cargo clippy -p bmc-wasm-runtime --bin capture --features capture -- -D warnings
    cargo nextest run -p bmc-wasm-runtime
    for root in $(bmc-wasm-runtime/tools/widget_root.py); do (cd "$root" && cargo build --target wasm32-unknown-unknown --workspace) || exit 1; done

# Block bloat crates from creeping into the wasm32 dep graph (source: `nix/checks.nix::cargo-deny-wasm`).
validate-wasm-deny:
    nix build -L ".#checks.{{ NIX_SYSTEM }}.cargo-deny-wasm"

# Block allocating fmt macros in widget code (source: `nix/checks.nix::no-fmt-in-wasm`).
validate-wasm-no-fmt:
    nix build -L ".#checks.{{ NIX_SYSTEM }}.no-fmt-in-wasm"

# Fast local ast-grep scan (same `rules/` as validate-wasm-no-fmt, no nix build).
ast-grep:
    nix-shell -p ast-grep --run "ast-grep scan --error"

# === Storybook ===

# Run the widget storybook (interactive component catalog).
storybook:
    cargo run -p bmc-storybook

# Run the storybook with hot-reload (rebuilds stories cdylib on change).
storybook-hot:
    cargo run -p bmc-storybook -- --hot-reload

# === Tooling ===

# Find unused workspace dependencies declared in Cargo.toml.
cargo-machete: (_cargo-tool "machete")

# Suggest version pins that collapse duplicate dependency versions.
cargo-dedupe: (_cargo-tool "dedupe")

_cargo-tool tool:
    @command -v cargo-{{ tool }} >/dev/null 2>&1 || { \
      printf 'cargo-{{ tool }} is not installed. Install with `cargo install cargo-{{ tool }}`? [y/N] '; \
      read -r reply; \
      [ "$reply" = "y" ] || [ "$reply" = "Y" ] || exit 1; \
      cargo install cargo-{{ tool }}; \
    }
    scripts/mem-box.sh cargo {{ tool }}
