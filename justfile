mod manifest 'bmc-widget-manifest/justfile'
mod netsim 'bmc-netsim/justfile'
mod virt 'bmc-virt/justfile'
mod wasm 'bmc-wasm-runtime/justfile'
mod fe 'frontend/justfile'

NIX_SYSTEM := "$(nix eval --impure --raw --expr builtins.currentSystem)"

# Auto-enter the dev shell for recipes that link against system libs
# (e.g. `cargo nextest` needs wayland-client to compile bmc).
# `cargo check`/`clippy` skip linking, so they don't need this wrap.
# No-op when already inside a `nix develop` shell (IN_NIX_SHELL set
# by nix); otherwise wrap the command in `nix develop --command`.
NIX_DEV := if env("IN_NIX_SHELL", "") == "" { "nix develop --command" } else { "" }

# Global env vars

export FORCE_COLOR := "1"
# Default tracing filter for `just wasm::dev` and friends; overridable by the caller's env.
export RUST_LOG := env('RUST_LOG', 'bmc_wasm_runtime=debug,testbed=debug,bmc_storybook=info')

[private]
default:
    @just --justfile {{ justfile() }} --list

# === Quick local validation (default; LLM-friendly) ===

# Fast sanity check, not CI-reproducible (use `validate-full` for that).
validate: format clippy python validate-wasm
    just manifest::check-schema
    # Native crates are clippy-gated above but not otherwise tested here: a
    # `--workspace` run cannot build (bmc-wasm-sdk loses bmc_render_macros under
    # workspace-wide feature unification), so each has to be named.
    just test bmc-netsim
    nix run ".#content-checks"
    nix build -L ".#checks.{{ NIX_SYSTEM }}.docs-wasm"
    @echo "validate: OK"

# Auto-format everything (nix fmt + SVG pass).
format:
    nix fmt
    nix run .#fmt-svg
    {{ NIX_DEV }} ruff format bmc-tui bmc-virt/harness

# Stamp the `.license.tpl` GPL header onto any first-party source missing it.
license-fix:
    scripts/check_license_headers.sh --fix

# Cargo clippy with the workspace's pedantic lints (mem-box caps memory).
clippy:
    scripts/mem-box.sh cargo clippy \
      --profile fast \
      --workspace \
      --all-features \
      --keep-going \
      --all-targets -- \
        -D warnings

# Lint, type-check and test the Python uv workspace (bmc-tui + harness).
# ruff/ty are nix-provided (PyPI binaries don't run in the pure-nix CI); pytest
# runs through uv. `uv sync` first so ty sees the workspace .venv.
python:
    {{ NIX_DEV }} uv sync
    {{ NIX_DEV }} ruff check bmc-tui bmc-virt/harness
    {{ NIX_DEV }} ruff format --check bmc-tui bmc-virt/harness
    # Fail on @deprecated APIs; must be a CLI flag — ty ignores [tool.ty.rules] here.
    {{ NIX_DEV }} ty check --error deprecated bmc-tui bmc-virt/harness
    {{ NIX_DEV }} uv run pytest

# Run nextest for a single crate with mem-box caps (auto-enters nix shell).
test crate:
    {{ NIX_DEV }} scripts/mem-box.sh cargo nextest run -p {{ crate }}

# Compress images under the given paths (default: cwd).
fmt-images *PATHS:
    nix run .#fmt-images -- {{ PATHS }}

# === Reproducible / slow ===

# Full nix-driven checks (matches CI's main stage) - Very heavy
validate-full:
    scripts/mem-box.sh nix flake check -L --option max-jobs 4

# === WASM Runtime ===

# Validate the wasm runtime + widget workspaces (format, lint, clippy, test, build).
validate-wasm: format validate-wasm-deny validate-wasm-no-fmt
    cargo clippy -p bmc-wasm-runtime --all-targets --features testbed -- -D warnings
    cargo clippy -p bmc-wasm-runtime --bin capture --features capture -- -D warnings
    # Wasm crates only: a full --workspace build breaks feature unification (bmc-wasm-sdk loses bmc_render_macros).
    cargo nextest run -p bmc-wasm-runtime -p bmc-wasm-sdk -p bmc-wasm-sdk-macros -p bmc-wasm-protocol -p bmc-wasm-thin -p bmc-wasm-thin-protocol
    # Production widgets are lint-gated (matches CI clippy-wasm-widgets); examples below build but aren't held to -D warnings.
    (cd widgets-wasm && cargo clippy --target wasm32-unknown-unknown --workspace -- -D warnings)
    for root in $(bmc-wasm-runtime/tools/widget_root.py); do (cd "$root" && cargo build --target wasm32-unknown-unknown --workspace) || exit 1; done
    # Widget logic has native unit tests; the wasm32 build above can't run them.
    (cd widgets-wasm && cargo nextest run --workspace)

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

# Recreate/repair the per-skill symlinks from .ai/skills into both tool dirs.
link-skills:
    cd "{{ justfile_directory() }}" && for s in .ai/skills/*/; do n="$(basename "$s")"; \
      ln -sfn "../../.ai/skills/$n" ".claude/skills/$n"; \
      ln -sfn "../../.ai/skills/$n" ".agents/skills/$n"; \
    done

_cargo-tool tool:
    @command -v cargo-{{ tool }} >/dev/null 2>&1 || { \
      printf 'cargo-{{ tool }} is not installed. Install with `cargo install cargo-{{ tool }}`? [y/N] '; \
      read -r reply; \
      [ "$reply" = "y" ] || [ "$reply" = "Y" ] || exit 1; \
      cargo install cargo-{{ tool }}; \
    }
    scripts/mem-box.sh cargo {{ tool }}
