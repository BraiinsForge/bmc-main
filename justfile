import 'common.justfile'

# Frontend build, lint, test, and mock-backend serving.
mod fe 'frontend/justfile'
# Gallery shows of components & other UI building blocks of UI
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

    # Generated code against its sources; stale output otherwise surfaces only in CI.
    # The protobuf bindings:
    just gen-check
    # Every widget's manifest_params.rs, bare like the cargo runs below,
    # so one toolchain owns the target dir.
    just nextest -p bmc-widget-codegen

    # Native crates are clippy-gated above but not otherwise tested here: a
    # `--workspace` run cannot build (bmc-wasm-sdk loses bmc_render_macros under
    # workspace-wide feature unification), so each has to be named.
    just test bmc-netsim
    nix run ".#content-checks"
    nix run ".#ast-grep"

    # Dependency policy, host and wasm.
    nix build {{ NIX_LOG }} ".#checks.{{ NIX_SYSTEM }}.cargo-deny"
    nix build {{ NIX_LOG }} ".#checks.{{ NIX_SYSTEM }}.cargo-deny-wasm"
    nix build {{ NIX_LOG }} ".#checks.{{ NIX_SYSTEM }}.cargo-deny-wasm-examples"

    # Public widget asset contract (icon paths, extension allowlist).
    nix build {{ NIX_LOG }} ".#checks.{{ NIX_SYSTEM }}.public-widget-assets"

    # Wasm lint; production widgets are gated, examples only have to build.
    cargo clippy --profile fast -p bmc-wasm-runtime --all-targets --features testbed -- -D warnings
    cargo clippy --profile fast -p bmc-wasm-runtime --bin capture --features capture -- -D warnings
    # Both runs above enable `host`; this one builds `hermetic-host` without it.
    cargo clippy --profile fast -p bmc-wasm-runtime --all-targets --features hermetic-host -- -D warnings
    (cd widgets-wasm && cargo clippy --profile fast --target wasm32-unknown-unknown --workspace -- -D warnings)

    # Crate list, not --workspace: that loses bmc-wasm-sdk's bmc_render_macros feature.
    # Left unwrapped like the clippy runs above: `nix develop` swaps the toolchain
    # and the sccache wrapper, and two rustc setups over one target dir do not link.
    just nextest \
        -p bmc-svg-compiler \
        -p bmc-ui \
        -p bmc-wasm-protocol \
        -p bmc-wasm-runtime \
        -p bmc-wasm-sdk \
        -p bmc-wasm-sdk-macros \
        -p bmc-wasm-thin \
        -p bmc-wasm-thin-protocol

    # The SDK's `#[wasm_test]`s again, on wasm32 under the wasm test runner.
    just test-wasm-sdk

    # Widget logic has native tests; the wasm32 builds below can't run them.
    just nextest --manifest-path widgets-wasm/Cargo.toml --workspace

    # Slowest last: every widget workspace for wasm32, then the SDK docs.
    for root in $(bmc-wasm-runtime/tools/widget_root.py); do (cd "$root" && cargo build --target wasm32-unknown-unknown --workspace) || exit 1; done
    nix build {{ NIX_LOG }} ".#checks.{{ NIX_SYSTEM }}.docs-wasm"

    @echo "validate: OK"

# Auto-format everything (nix fmt + SVG pass).
format:
    # `--no-cache` matches the CI fmt job: a cache hit hides drift that CI then
    # rejects, so without it a clean `just validate` says nothing about the fmt job.
    nix fmt -- --no-cache
    nix run .#fmt-svg
    # The two workspace members only; `scripts/` is treefmt's,
    # and handing it to both leaves them fighting for style.
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

# Convert a markdown file to Atlassian Document Format on stdout (for Jira descriptions).
md-to-adf FILE *ARGS:
    # Silent: the ADF is meant to be redirected into a file, so nothing else may
    # reach stdout.
    @{{ NIX_DEV }} uv run scripts/md_to_adf.py {{ FILE }} {{ ARGS }}

# Lint, type-check and test the Python uv workspace (bmc-tui + harness).
python:
    # ruff/basedpyright are nix-provided, matching the pure-nix CI; pytest
    # runs through uv. `uv sync` first so basedpyright sees the workspace .venv.
    {{ NIX_DEV }} uv sync
    {{ NIX_DEV }} ruff check bmc-tui bmc-virt/harness scripts
    {{ NIX_DEV }} ruff format --check bmc-tui bmc-virt/harness
    {{ NIX_DEV }} basedpyright
    # coverage.py opens its data file without creating the directory above it,
    # and this one lives under the scratch tree, which gets cleaned out —
    # so make it on every run rather than once.
    mkdir -p .tmp/coverage/python
    {{ NIX_DEV }} uv run pytest

# Run nextest for a single crate (auto-enters nix shell).
test crate *FEATURES="--all-features":
    {{ NIX_DEV }} just nextest -p {{ crate }} {{ FEATURES }}

# nextest streamed for a person, or condensed to a line per binary by `scripts/nextest_report.py` for an agent.
[positional-arguments]
[private]
nextest *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail

    if [ "{{ AGENT }}" != "1" ]; then
        exec cargo nextest run "$@"
    fi

    # The JSON stream is nextest's only reporter hook; the flag is its stability caveat.
    NEXTEST_EXPERIMENTAL_LIBTEST_JSON=1 \
        cargo nextest run "$@" --message-format libtest-json-plus \
        | scripts/nextest_report.py

# Run one widget's (or shared lib's) `#[wasm_test]`s on wasm32 under the wasm test runner; ARGS reach nextest.
[positional-arguments]
test-widget WIDGET *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail

    widget=$1
    shift
    # Bare cargo, as `validate` runs the widget workspaces:
    # `nix develop` swaps the toolchain and the sccache wrapper,
    # and one target dir cannot link against two rustc setups.
    for root in $(bmc-wasm-runtime/tools/widget_root.py); do
        if [ -d "$root/$widget" ] || [ -d "$root/lib/$widget" ]; then
            exec just nextest --manifest-path "$root/Cargo.toml" -p "$widget" --target wasm32-unknown-unknown "$@"
        fi
    done

    echo "no widget '$widget' under any widget workspace" >&2
    exit 1

# Run the SDK's `#[wasm_test]`s on wasm32, built in `widgets-wasm` as CI builds them; ARGS reach nextest.
[positional-arguments]
test-wasm-sdk *ARGS:
    just nextest --manifest-path widgets-wasm/Cargo.toml -p bmc-wasm-sdk --target wasm32-unknown-unknown "$@"

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

# === Codegen ===

# Regenerate the protobuf bindings: Rust under bmc-grpc/src/gen, TypeScript under frontend/src/proto/gen.
gen:
    # buf and the Rust plugins come from the dev shell; protoc-gen-es is the frontend's.
    PATH="{{ justfile_directory() }}/frontend/node_modules/.bin:$PATH" {{ NIX_DEV }} buf generate

# Fail when the checked-in bindings differ from what `gen` writes, showing the difference.
gen-check:
    # Always `-L`, unlike `NIX_LOG`: with the log tail off, it is all that shows a failure's diff.
    nix build -L --log-lines 0 ".#checks.{{ NIX_SYSTEM }}.proto-bindings"

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
