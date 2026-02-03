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
	RUST_LOG=bmc_wasm_runtime=debug mprocs \
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

# Profiling with samply (install: cargo install samply)
# Requires: echo '-1' | sudo tee /proc/sys/kernel/perf_event_paranoid
.PHONY: profile
profile:
	cd $(EXAMPLE_DIR) && cargo build --release --target $(WASM_TARGET)
	cargo build --profile profiling --features testbed --bin testbed
	$(HOME)/.cargo/bin/samply record ../target/profiling/testbed $(subst /debug/,/release/,$(WASM_FILE))

# Measure WASM binary size (install: brew install binaryen)
WASM_RELEASE := $(EXAMPLE_DIR)/target/$(WASM_TARGET)/release/$(subst -,_,$(EXAMPLE)).wasm
WASM_OPT := $(EXAMPLE_DIR)/target/$(WASM_TARGET)/release/$(subst -,_,$(EXAMPLE)).opt.wasm

.PHONY: size
size:
	@which wasm-opt > /dev/null || (echo "Error: wasm-opt not found. Install with: brew install binaryen" && exit 1)
	@which wasm-objdump > /dev/null || (echo "Error: wasm-objdump not found. Install with: brew install wabt" && exit 1)

	@cd $(EXAMPLE_DIR) && cargo build --release --target $(WASM_TARGET)
	@wasm-opt -Oz $(WASM_RELEASE) -o $(WASM_OPT)

	@echo "=== WASM Binary Size: $(EXAMPLE) ==="
	@echo "Release:   $$(du -h $(WASM_RELEASE) | cut -f1) ($$(wc -c < $(WASM_RELEASE)) bytes)"
	@echo "Optimized: $$(du -h $(WASM_OPT) | cut -f1) ($$(wc -c < $(WASM_OPT)) bytes)"

	@echo ""
	@echo "Sections:"
	@wasm-objdump -h $(WASM_OPT) 2>/dev/null || true
