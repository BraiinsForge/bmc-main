SHELL := bash

.PHONY: format
format:
	nix fmt

.PHONY: lint
lint:
	cargo clippy --workspace --all-targets

.PHONY: test
test:
	cargo nextest run --workspace --exclude bmc-display

.PHONY: validate
validate: format lint
	$(MAKE) -C frontend validate
