CI_TOOLS_REV := "c75e453c0e3fd5fe167a9437b86e48b54c2aa81c"

validate: format clippy validate-harness

validate-harness:
    cd bmc-virt/harness && nix develop --command just validate

format:
    nix fmt
    nix run .#fmt-svg

rust-pedantic:
    nix run "git+ssh://git@gitlab.ii.zone/nix/ci-tools?rev={{CI_TOOLS_REV}}#check-rust-diff" \
      "$(git merge-base origin/master HEAD)" HEAD

clippy:
    cargo clippy \
      --profile fast \
      --workspace \
      --features bmc-display/slint-embed-files \
      --keep-going \
      --all-targets -- \
        -D warnings

# Find unused workspace dependencies declared in Cargo.toml.
cargo-machete: (_cargo-tool "machete")

# Suggest version pins that collapse duplicate dependency versions.
cargo-dedupe: (_cargo-tool "dedupe")

_cargo-tool tool:
    @command -v cargo-{{tool}} >/dev/null 2>&1 || { \
      printf 'cargo-{{tool}} is not installed. Install with `cargo install cargo-{{tool}}`? [y/N] '; \
      read -r reply; \
      [ "$reply" = "y" ] || [ "$reply" = "Y" ] || exit 1; \
      cargo install cargo-{{tool}}; \
    }
    cargo {{tool}}
