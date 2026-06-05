# Weather BMM101 Scaling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the weather widget's Large layout fit BMM101 by scaling authored metrics with `WidgetSize::fit()` and
giving the location label a full-width non-wrapping row.

**Architecture:** Keep SDK size classification unchanged: BMM101 remains `SizeVariant::Large`. Add weather-specific
metric structs that scale visual constants, pass those metrics through shared render helpers, and restructure only the
Large layout so the location label is outside the narrow current-conditions column.

**Tech Stack:** Rust, `bmc_wasm_sdk` render tree nodes, native `cargo test -p weather`, wasm-target clippy.

---

### Task 1: Add Testable Large Metrics

**Files:**

- Modify: `widgets-wasm/weather/src/render/large.rs`

- Test: `widgets-wasm/weather/src/render/large.rs`

- [ ] **Step 1: Write failing metric tests**

Add a `#[cfg(test)]` module to `large.rs` with tests that assert
`LargeMetrics::for_size(WidgetSize::from_dimensions(480, 320))` scales representative Large values:

```rust
#[cfg(test)]
mod tests {
    use super::LargeMetrics;
    use bmc_wasm_sdk::WidgetSize;

    #[test]
    fn bmm101_large_metrics_scale_by_fit() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(480, 320));

        assert_eq!(metrics.location_font_size, 11);
        assert_eq!(metrics.temperature_font_size, 43);
        assert_eq!(metrics.current_icon_size, 45.0);
        assert_eq!(metrics.padding, 10.666_667);
        assert_eq!(metrics.forecast_bar_width, 93.333_336);
    }

    #[test]
    fn canonical_large_metrics_stay_authored() {
        let metrics = LargeMetrics::for_size(WidgetSize::from_dimensions(638, 480));

        assert_eq!(metrics.location_font_size, 16);
        assert_eq!(metrics.temperature_font_size, 64);
        assert_eq!(metrics.current_icon_size, 68.0);
        assert_eq!(metrics.padding, 16.0);
        assert_eq!(metrics.forecast_bar_width, 140.0);
    }
}
```

- [ ] **Step 2: Run tests to verify RED**

Run from `widgets-wasm/`:
`nix develop ../.#fast -c cargo test -p weather large::tests::bmm101_large_metrics_scale_by_fit`

Expected: FAIL because `LargeMetrics` does not exist.

- [ ] **Step 3: Implement `LargeMetrics`**

In `large.rs`, add `LargeMetrics` with authored Large values and `for_size(size: WidgetSize) -> Self`. Use
`bmc_wasm_sdk::scale_font` for font sizes and multiply floating metrics by `size.fit()`.

- [ ] **Step 4: Run tests to verify GREEN**

Run from `widgets-wasm/`: `nix develop ../.#fast -c cargo test -p weather large::tests`

Expected: PASS.

### Task 2: Make Shared Forecast Helpers Accept Scaled Metrics

**Files:**

- Modify: `widgets-wasm/weather/src/render/common.rs`

- Modify: `widgets-wasm/weather/src/render/full.rs`

- Modify: `widgets-wasm/weather/src/render/large.rs`

- [ ] **Step 1: Extend helper input structs**

Add `label_size` and `temperature_size` to `HourStyle`. Add a new public `ForecastRowStyle` with `row_gap`,
`label_size`, `icon_size`, `temperature_size`, `temperature_cell_width`, `bar_width`, and `bar_height`.

- [ ] **Step 2: Update helper implementations**

Update `hour_cell`, `temp_box`, and `forecast_row` to use the provided sizes instead of constants.

- [ ] **Step 3: Update callers**

In `full.rs`, set `HOUR_STYLE` to keep existing Full values. In `large.rs`, build a scaled `ForecastRowStyle` from
`LargeMetrics` and pass it to `forecast_row`.

- [ ] **Step 4: Run focused tests**

Run from `widgets-wasm/`: `nix develop ../.#fast -c cargo test -p weather large::tests`

Expected: PASS.

### Task 3: Restructure Large Location Row

**Files:**

- Modify: `widgets-wasm/weather/src/render/large.rs`

- Test: `widgets-wasm/weather/src/render/large.rs`

- [ ] **Step 1: Write failing layout test**

Add a test that renders Large with `WidgetSize::from_dimensions(480, 320)`, finds the `Prague, Czech Republic` text
node, and asserts:

- it is a direct child of the Large root column;

- its `TextStyle.text_overflow` is `TextOverflow::Clip`;

- its `TextStyle.max_width` equals `480.0 - 2.0 * metrics.padding`;

- its font size is the scaled location font size.

- [ ] **Step 2: Run test to verify RED**

Run from `widgets-wasm/`:
`nix develop ../.#fast -c cargo test -p weather large::tests::bmm101_location_uses_full_non_wrapping_row`

Expected: FAIL because the location is still inside `current_left` and uses wrapping text style.

- [ ] **Step 3: Implement layout change**

Move location rendering out of `current_left`. Add a `location_row(weather, &metrics, size.width)` helper. Use
`TextOverflow::Clip` and `max_width = size.width as f32 - 2.0 * metrics.padding`.

- [ ] **Step 4: Run tests to verify GREEN**

Run from `widgets-wasm/`: `nix develop ../.#fast -c cargo test -p weather large::tests`

Expected: PASS.

### Task 4: Compile and Lint the Weather Widget

**Files:**

- Modify only files already touched by Tasks 1-3 as needed for compiler and clippy feedback.

- [ ] **Step 1: Run native tests**

Run from `widgets-wasm/`: `nix develop ../.#fast -c cargo test -p weather`

Expected: PASS.

- [ ] **Step 2: Run wasm-target clippy**

Run from `widgets-wasm/`:
`nix develop ../.#fast -c cargo clippy -p weather --target wasm32-unknown-unknown -- -D warnings`

Expected: PASS.

- [ ] **Step 3: Run formatter**

Run:
`nix develop .#fast -c rustfmt widgets-wasm/weather/src/render/common.rs widgets-wasm/weather/src/render/full.rs widgets-wasm/weather/src/render/large.rs`

Expected: files are formatted with no command failure.

### Task 5: Visual Check

**Files:**

- No planned source changes unless the visual check exposes a concrete issue.

- [ ] **Step 1: Run BMM101 testbed**

Run: `just wasm::run weather "--platform BMM101"`

Expected: local testbed starts and renders Weather at `480x320`.

- [ ] **Step 2: Inspect the BMM101 layout**

Confirm `Prague, Czech Republic` stays on one line, the current block and stats fit, and daily forecast rows stay inside
the viewport.

- [ ] **Step 3: Stop the testbed**

Stop the dev server/process before finishing the task.
