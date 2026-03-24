# WASM Widget Runtime — Development Makefile
#
# This file is a dumb shell-autocomplete signpost. Each target should be
# a one-liner delegating to a proper tool (cargo, python3, etc.). Do NOT
# put real logic here — if a target needs more than a trivial command,
# write a Python script in tools/ and call it from here.

EXAMPLE ?= hello-widget
EXAMPLE_DIR := examples/$(EXAMPLE)
WASM_TARGET := wasm32-unknown-unknown
WASM_FILE := $(EXAMPLE_DIR)/target/$(WASM_TARGET)/debug/$(subst -,_,$(EXAMPLE)).wasm

CARGO_FEATURES := --features testbed
PYTHON := uv run --python 3.12

.PHONY: clean
clean:
	cargo clean
	cd $(EXAMPLE_DIR) && cargo clean

.PHONY: dev
dev:
	@cargo build $(CARGO_FEATURES) --bin testbed
	@cd $(EXAMPLE_DIR) && cargo build --target $(WASM_TARGET)
	RUST_LOG=bmc_wasm_runtime=debug mprocs \
		--names "watch,testbed" \
		"cd $(EXAMPLE_DIR) && cargo watch -x 'build --target $(WASM_TARGET)'" \
		"cargo run $(CARGO_FEATURES) --bin testbed -- $(WASM_FILE) $(ARGS)"

.PHONY: run
run:
	cd $(EXAMPLE_DIR) && cargo build --release --target $(WASM_TARGET)
	cargo run $(CARGO_FEATURES) --bin testbed -- $(subst /debug/,/release/,$(WASM_FILE)) $(ARGS)

# Profiling: make profile [-e hello-widget] [-r 05-my-change] [-n 1200]
.PHONY: profile
profile:
	$(PYTHON) tools/perf_record.py -e $(EXAMPLE) $(ARGS)

# WASM binary size: make size [-e hello-widget] [--check] [--limit 65536]
.PHONY: size
size:
	$(PYTHON) tools/wasm_size.py -e $(EXAMPLE)

.PHONY: size-check
size-check:
	$(PYTHON) tools/wasm_size.py -e $(EXAMPLE) --check

# SDK API docs (opens in browser)
.PHONY: docs
docs:
	cd sdk && cargo doc --target $(WASM_TARGET) --no-deps --open
