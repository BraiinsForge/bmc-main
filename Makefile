SHELL := bash

## Commented out temporarily because the base branch
## is a mess and running these produces A LOT of changes
## in a code I do not own and should not change!
#.PHONY: format
#format:
#	nix fmt
#
#.PHONY: lint
#lint:
#	cargo clippy --workspace --all-targets
#	nix-shell -p ruff uv --run "ruff check && uvx ty check --exclude 'bmc-wasm-runtime/examples/*/tools/'"
#
#.PHONY: cargo-deny
#cargo-deny:
#	cargo deny check bans sources
#
#.PHONY: test
#test:
#	cargo nextest run --workspace --exclude bmc-display
#
#.PHONY: validate
#validate: format lint cargo-deny test
#	$(MAKE) -C frontend validate

.PHONY: fmt-svg
fmt-svg:
	nix run .#fmt-svg

.PHONY: validate-wasm
validate-wasm: fmt-svg
	nix fmt -- bmc-wasm-runtime
	nix-shell -p ruff uv --run "ruff check bmc-wasm-runtime && uvx ty check bmc-wasm-runtime --exclude 'bmc-wasm-runtime/examples/*/tools/'"
	@for dir in bmc-wasm-runtime/examples/*/; do \
		if [ -f "$$dir/Makefile" ] && grep -qE '^lint:' "$$dir/Makefile"; then \
			echo "── Python lint in $${dir} ──"; \
			$(MAKE) -C "$$dir" lint || exit 1; \
		fi; \
	done
	cargo clippy -p bmc-wasm-runtime --all-targets --features testbed
	cargo clippy -p bmc-wasm-runtime --bin capture --features capture
	cargo nextest run -p bmc-wasm-runtime
	@for dir in bmc-wasm-runtime/examples/*/; do \
		echo "── Building $${dir} ──"; \
		(cd "$$dir" && cargo build --target wasm32-unknown-unknown) || exit 1; \
	done
