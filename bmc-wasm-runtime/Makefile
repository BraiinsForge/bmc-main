# WASM Widget Runtime — Development Makefile
#
# This file is a dumb shell-autocomplete signpost. Each target should be
# a one-liner delegating to a proper tool (cargo, python3, etc.). Do NOT
# put real logic here — if a target needs more than a trivial command,
# write a Python script in tools/ and call it from here.

WASM_TARGET := wasm32-unknown-unknown
WASM_FILE_DEBUG := examples/target/$(WASM_TARGET)/debug/$(subst -,_,$(EXAMPLE)).wasm
WASM_FILE := examples/target/$(WASM_TARGET)/release/$(subst -,_,$(EXAMPLE)).wasm

CARGO_FEATURES := --features testbed

.PHONY: clean
clean:
	cargo clean
	cd examples && cargo clean

.PHONY: dev
dev:
	@cargo build $(CARGO_FEATURES) --bin testbed
	@cd examples && cargo build -p $(EXAMPLE) --target $(WASM_TARGET)
	RUST_LOG=bmc_wasm_runtime=debug mprocs \
		--names "watch,testbed" \
		"cd examples && cargo watch -x 'build -p $(EXAMPLE) --target $(WASM_TARGET)'" \
		"cargo run $(CARGO_FEATURES) --bin testbed -- $(WASM_FILE_DEBUG) $(ARGS)"

.PHONY: run
run:
	@cd examples && cargo build -p $(EXAMPLE) --release --target $(WASM_TARGET)
	@cargo run $(CARGO_FEATURES) --bin testbed -- $(WASM_FILE) $(ARGS)

# Profiling: make profile [-e hello-widget] [-r 05-my-change] [-n 1200]
.PHONY: profile
profile:
	@tools/perf_record.py -e $(EXAMPLE) $(ARGS)

# WASM binary size: make size [-e hello-widget] [--check] [--limit 65536]
.PHONY: size
size:
	@tools/wasm_size.py -e $(EXAMPLE)

.PHONY: size-check
size-check:
	@tools/wasm_size.py -e $(EXAMPLE) --check

# SDK API docs (opens in browser)
.PHONY: docs
docs:
	@cd sdk && cargo doc --target $(WASM_TARGET) --no-deps --open

# ── Interactive Recording ────────────────────────────────────────────

SIZE ?= full
.PHONY: record
record:
	@cd examples && cargo build -p $(EXAMPLE) --release --target $(WASM_TARGET)
	@cargo run $(CARGO_FEATURES) --bin testbed -- $(WASM_FILE) --record=$(SIZE)

# ── Capture & Visual Regression ──────────────────────────────────────

WIDGETS_DIR := examples
WASM_DIR    := examples/target/$(WASM_TARGET)/release
OUTPUT_DIR  := captures

CAPTURE_BIN := cargo run --features capture --bin capture --

.PHONY: capture
capture:
	@$(CAPTURE_BIN) run-all -e $(EXAMPLE) --widgets-dir $(WIDGETS_DIR) --output-dir $(OUTPUT_DIR) $(ARGS)

.PHONY: capture-all
capture-all:
	@$(CAPTURE_BIN) run-all --widgets-dir $(WIDGETS_DIR) --output-dir $(OUTPUT_DIR) $(ARGS)

.PHONY: update-baselines
update-baselines: capture
	@$(CAPTURE_BIN) set-baseline --capture-dir=$(WIDGETS_DIR)/$(EXAMPLE)/capture --output=$(OUTPUT_DIR)/$(EXAMPLE)

.PHONY: regression-test
regression-test:
	@$(CAPTURE_BIN) verify -e $(EXAMPLE) --widgets-dir $(WIDGETS_DIR) --output-dir $(OUTPUT_DIR)

.PHONY: regression-test-all
regression-test-all:
	@$(CAPTURE_BIN) verify --parallel --widgets-dir $(WIDGETS_DIR) --output-dir $(OUTPUT_DIR)

.PHONY: preview
preview:
	@$(CAPTURE_BIN) preview --output=$(OUTPUT_DIR)/$(EXAMPLE) $(if $(filter command line,$(origin SIZE)),-s $(SIZE)) $(ARGS)
