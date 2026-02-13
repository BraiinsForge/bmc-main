SHELL := bash

.PHONY: format
format:
	nix fmt

.PHONY: lint
lint:
	cargo clippy --workspace --all-targets

.PHONY: cargo-deny
cargo-deny:
	cargo deny check bans sources

.PHONY: test
test:
	cargo nextest run --workspace --exclude bmc-display

.PHONY: validate
validate: format lint cargo-deny test
	$(MAKE) -C frontend validate
