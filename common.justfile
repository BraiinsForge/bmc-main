# Settings every justfile shares.
# They cross an `import` but not a `mod`, so every justfile imports this one.
# Paths anchor on `source_directory()`, which stays here whatever imports it.

# A first recipe is often a `clean` or a `validate`; bare `just` lists instead.
set default-list

# Never the dev shell's tree, or each toolchain rebuilds the other's.
CARGO_TARGET_ROOT := env('CARGO_TARGET_DIR', source_directory() / ".tmp/cargo-target")
export CARGO_TARGET_DIR := CARGO_TARGET_ROOT

# Only this catches all of ruff's cache; the package `cache-dir` configs miss some.
export RUFF_CACHE_DIR := source_directory() / ".tmp/ruff_cache"

export RUST_LOG := env('RUST_LOG', 'bmc_wasm_runtime=debug,testbed=debug,bmc_gallery=info')

# Whether a coding agent reads the output — its transcript truncates
# a long log, which buries the one line that says what failed.
# Detected the way `std-env` does, by the variables each agent sets.
AGENT_ENVS := 'CLAUDECODE|CURSOR_AGENT|GEMINI_CLI|CODEX_THREAD_ID|OPENCODE'
AGENT := shell("env | grep -qE '^(" + AGENT_ENVS + ")=.' && echo 1 || true")

# Colour for a person even through `just`'s pipes;
# none for an agent, whose transcript shows the escape codes as noise.
# `NO_COLOR` is never exported: tools read its presence,
# not its value, so an empty one strips a person's colour too.
# pytest reads `FORCE_COLOR` by presence as well, so it gets its own switch.
export FORCE_COLOR := if AGENT == '1' { '0' } else { '1' }
export PY_COLORS := if AGENT == '1' { '0' } else { '1' }
export CARGO_TERM_COLOR := env('CARGO_TERM_COLOR', if AGENT == '1' { 'never' } else { 'auto' })

# For an agent, every tool's own quiet mode: cargo drops its per-crate status lines,
# nextest its per-test ones (the `agent` profile in each `.config/nextest.toml`),
# pytest its dots, and nix its build logs — it still prints the failing derivation's tail.
export CARGO_TERM_QUIET := env('CARGO_TERM_QUIET', if AGENT == '1' { 'true' } else { 'false' })
export NEXTEST_PROFILE := env('NEXTEST_PROFILE', if AGENT == '1' { 'agent' } else { 'default' })
export PYTEST_ADDOPTS := env('PYTEST_ADDOPTS', if AGENT == '1' { '-q' } else { '' })
export NIX_LOG := if AGENT == '1' { '' } else { '-L' }

# The compositor checks are Linux-only; on darwin nix routes them to a `linux-builder`.
# Built from just's own `arch()`, which names architectures the way nix does:
# a `$(…)` would expand only inside recipe lines and export the command text,
# and a `shell("nix eval …")` has no nix to call inside a nix build.
export NIX_SYSTEM := if os() == "macos" { "aarch64-linux" } else { arch() + "-linux" }
