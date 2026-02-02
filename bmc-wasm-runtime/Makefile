# WASM Widget Runtime - Development Makefile
# Usage: make dev [EXAMPLE=hello-widget]

EXAMPLE ?= hello-widget
EXAMPLE_DIR := examples/$(EXAMPLE)
WASM_TARGET := wasm32-unknown-unknown
WASM_FILE := $(EXAMPLE_DIR)/target/$(WASM_TARGET)/debug/$(subst -,_,$(EXAMPLE)).wasm

.PHONY: clean
clean:
	cargo clean
	cd $(EXAMPLE_DIR) && cargo clean

.PHONY: dev
dev:
	@cargo build --features testbed --bin testbed
	@cd $(EXAMPLE_DIR) && cargo build --target $(WASM_TARGET)
	mprocs \
		--names "validate,watch,testbed" \
		"make validate" \
		"cd $(EXAMPLE_DIR) && cargo watch -x 'build --target $(WASM_TARGET)'" \
		"cargo run --features testbed --bin testbed -- $(WASM_FILE)"

.PHONY: run
run:
	cd $(EXAMPLE_DIR) && cargo build --release --target $(WASM_TARGET)
	cargo run --features testbed --bin testbed -- $(subst /debug/,/release/,$(WASM_FILE))

.PHONY: validate
validate:
	@cargo fmt
	@cargo clippy --all-targets --all-features -- -D warnings
