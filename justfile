CI_TOOLS_REV := "c75e453c0e3fd5fe167a9437b86e48b54c2aa81c"

# === Quick local validation (default; LLM-friendly) ===

# Fast sanity check: format + clippy (not CI-reproducible — use `validate-full` for that).
validate: format clippy

# Auto-format everything (nix fmt + SVG pass).
format:
    nix fmt
    nix run .#fmt-svg

# Cargo clippy with the workspace's pedantic lints (mem-box caps memory).
clippy:
    scripts/mem-box.sh cargo clippy \
      --profile fast \
      --workspace \
      --features bmc-display/slint-embed-files \
      --keep-going \
      --all-targets -- \
        -D warnings

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
