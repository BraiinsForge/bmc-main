SHELL := bash

.PHONY: format
format:
	nix fmt

.PHONY: lint
lint:
	cargo clippy --workspace --all-targets
	nix-shell -p ruff uv --run "ruff check && uvx ty check --exclude 'bmc-wasm-runtime/examples/*/tools/'"

.PHONY: cargo-deny
cargo-deny:
	cargo deny check bans sources

.PHONY: test
test:
	cargo nextest run --workspace --exclude bmc-display

.PHONY: validate
validate: format lint cargo-deny test
	$(MAKE) -C frontend validate

.PHONY: validate-wasm
validate-wasm:
	nix fmt -- bmc-wasm-runtime
	cargo clippy -p bmc-wasm-runtime --all-targets
	cargo nextest run -p bmc-wasm-runtime
