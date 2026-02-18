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
