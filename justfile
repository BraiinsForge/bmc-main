import 'common.justfile'

# Frontend build, lint, test, and mock-backend serving.
mod fe 'frontend/justfile'
mod gallery 'bmc-gallery/justfile'
# Widget manifest schema generation and drift checks.
mod manifest 'bmc-widget-manifest/justfile'
# Simulated miner network for widget development.
mod netsim 'bmc-netsim/justfile'
# Development VM: lifecycle, logs, and guest harness.
mod virt 'bmc-virt/justfile'
# WASM widget tooling: dev testbed, capture, profiling, size gates.
mod wasm 'bmc-wasm-runtime/justfile'
# Per-widget dev recipes.
mod widgets 'widgets-wasm'

# Auto-enter the dev shell for recipes that link against system libs
# (e.g. `cargo nextest` needs wayland-client to compile bmc).
# `cargo check`/`clippy` skip linking, so they don't need this wrap.
# No-op when already inside a `nix develop` shell (IN_NIX_SHELL set
# by nix); otherwise wrap the command in `nix develop --command`.
NIX_DEV := if env("IN_NIX_SHELL", "") == "" { "nix develop --command" } else { "" }

# === Quick local validation (default; LLM-friendly) ===

# Fast sanity check, not CI-reproducible (use `validate-full` for that).
validate: format clippy python
    # Cheap static gates.
    just manifest::check-schema
    # The other generated-artifact guard: every widget's manifest_params.rs
    # against its manifest. Stale output otherwise surfaces only in CI.
    # Bare like the cargo runs below, so one toolchain owns the target dir.
    cargo nextest run -p bmc-widget-codegen
    # Native crates are clippy-gated above but not otherwise tested here: a
    # `--workspace` run cannot build (bmc-wasm-sdk loses bmc_render_macros under
    # workspace-wide feature unification), so each has to be named.
    just test bmc-netsim
    nix run ".#content-checks"

    # Dependency policy, host and wasm.
    nix build -L ".#checks.{{ NIX_SYSTEM }}.cargo-deny"
    nix build -L ".#checks.{{ NIX_SYSTEM }}.cargo-deny-wasm"
    nix build -L ".#checks.{{ NIX_SYSTEM }}.cargo-deny-wasm-examples"

    # No allocating fmt machinery in widget code.
    nix build -L ".#checks.{{ NIX_SYSTEM }}.no-fmt-in-wasm"

    # Public widget asset contract (icon paths, extension allowlist).
    nix build -L ".#checks.{{ NIX_SYSTEM }}.public-widget-assets"

    # Wasm lint; production widgets are gated, examples only have to build.
    cargo clippy --profile fast -p bmc-wasm-runtime --all-targets --features testbed -- -D warnings
    cargo clippy --profile fast -p bmc-wasm-runtime --bin capture --features capture -- -D warnings
    (cd widgets-wasm && cargo clippy --profile fast --target wasm32-unknown-unknown --workspace -- -D warnings)

    # Crate list, not --workspace: that loses bmc-wasm-sdk's bmc_render_macros feature.
    # Left unwrapped like the clippy runs above: `nix develop` swaps the toolchain
    # and the sccache wrapper, and two rustc setups over one target dir do not link.
    cargo nextest run -p bmc-wasm-runtime -p bmc-wasm-sdk -p bmc-wasm-sdk-macros -p bmc-wasm-protocol -p bmc-svg-compiler -p bmc-wasm-thin -p bmc-wasm-thin-protocol

    # Widget logic has native tests; the wasm32 builds below can't run them.
    (cd widgets-wasm && cargo nextest run --workspace)

    # Slowest last: every widget workspace for wasm32, then the SDK docs.
    for root in $(bmc-wasm-runtime/tools/widget_root.py); do (cd "$root" && cargo build --target wasm32-unknown-unknown --workspace) || exit 1; done
    nix build -L ".#checks.{{ NIX_SYSTEM }}.docs-wasm"

    @echo "validate: OK"

# Auto-format everything (nix fmt + SVG pass).
format:
    # `--no-cache` matches the CI fmt job: a cache hit hides drift that CI then
    # rejects, so without it a clean `just validate` says nothing about the fmt job.
    nix fmt -- --no-cache
    nix run .#fmt-svg
    {{ NIX_DEV }} ruff format bmc-tui bmc-virt/harness

# Stamp the `.license.tpl` GPL header onto any first-party source missing it.
license-fix:
    scripts/check_license_headers.sh --fix

# Cargo clippy with the workspace's pedantic lints.
clippy:
    cargo clippy \
      --profile fast \
      --workspace \
      --all-features \
      --keep-going \
      --all-targets -- \
        -D warnings

# Lint, type-check and test the Python uv workspace (bmc-tui + harness).
python:
    # ruff/ty are nix-provided (PyPI binaries don't run in the pure-nix CI); pytest
    # runs through uv. `uv sync` first so ty sees the workspace .venv.
    {{ NIX_DEV }} uv sync
    {{ NIX_DEV }} ruff check bmc-tui bmc-virt/harness
    {{ NIX_DEV }} ruff format --check bmc-tui bmc-virt/harness
    # Fail on @deprecated APIs; must be a CLI flag — ty ignores [tool.ty.rules] here.
    {{ NIX_DEV }} ty check --error deprecated bmc-tui bmc-virt/harness
    {{ NIX_DEV }} uv run pytest

# Run nextest for a single crate (auto-enters nix shell).
test crate *FEATURES="--all-features":
    {{ NIX_DEV }} cargo nextest run -p {{ crate }} {{ FEATURES }}

# Run one widget's native tests, FILTER naming a substring of the test names.
test-widget WIDGET FILTER="":
    #!/usr/bin/env bash
    set -euo pipefail

    # Bare cargo, as `validate` runs the widget workspaces:
    # `nix develop` swaps the toolchain and the sccache wrapper,
    # and one target dir cannot link against two rustc setups.
    for root in $(bmc-wasm-runtime/tools/widget_root.py); do
        if [ -d "$root/{{ WIDGET }}" ]; then
            cd "$root"
            if [ -n "{{ FILTER }}" ]; then
                exec cargo nextest run -p {{ WIDGET }} "{{ FILTER }}"
            fi
            exec cargo nextest run -p {{ WIDGET }}
        fi
    done

    echo "no widget '{{ WIDGET }}' under any widget workspace" >&2
    exit 1

# Compress images under the given paths (default: cwd).
fmt-images *PATHS:
    nix run .#fmt-images -- {{ PATHS }}

# === Reproducible / slow ===

# Full nix-driven checks (matches CI's main stage) - Very heavy
validate-full:
    nix flake check -L --option max-jobs 4

# One nix check attribute, as the CI check jobs run it: `just check nextest`,
# `check clippy`, `check build`. These build in the `ci` profile, which is where
# Mesa and surfaceless EGL come from — the GL-harness tests cannot run outside it.
check ATTR:
    nix build .#{{ ATTR }} -L --no-link

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
    cargo {{ tool }}
