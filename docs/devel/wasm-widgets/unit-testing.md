# WASM Widget Unit Testing

A widget's unit tests compile to `wasm32` and run under the wasm test runner, in `wasmi`, the interpreter the Deck runs
widgets in. Mark a test with `#[bmc_wasm_sdk::wasm_test]` where a native test would take `#[test]`.

These are logic tests. What a widget draws is checked by [regression testing](regression-testing.md).

## Write A Test

```rust
#[cfg(test)]
mod tests {
    use bmc_wasm_sdk::wasm_test;

    use super::*;

    #[wasm_test]
    fn month_abbr_maps_valid_months() {
        assert_eq!(month_abbr(1), "Jan");
    }

    #[wasm_test]
    #[should_panic(expected = "month must be 1..=12")]
    fn month_abbr_asserts_on_zero() {
        month_abbr(0);
    }
}
```

`#[should_panic]` and `#[ignore]` go after the attribute, as they would after `#[test]`. Off `wasm32` the attribute
becomes a plain `#[test]`, so a native `cargo test` still runs the same test. A plain `#[test]` runs natively only; the
runner never sees it.

A test that installs a system snapshot with `system::set_current` needs the SDK's `test-support` feature:

```toml
[dev-dependencies]
bmc-wasm-sdk = { path = "../../bmc-wasm-runtime/sdk", features = ["test-support"] }
```

## Run Tests

```sh
just test-widget clock
just test-widget clock screens
just test-wasm-sdk
```

`test-widget` takes a widget's directory name, or a shared library's under `lib/`. Whatever follows the name reaches
nextest, so a substring of the test names narrows the run. `test-wasm-sdk` runs the SDK's own tests.

Plain cargo works as well. `.cargo/config.toml` names `scripts/wasm_test_runner.sh` as the runner for `wasm32` binaries,
so any test command with `--target wasm32-unknown-unknown` goes through it:

```sh
cargo nextest run --manifest-path widgets-wasm/Cargo.toml -p clock --target wasm32-unknown-unknown
cargo test --manifest-path widgets-wasm/Cargo.toml -p clock --target wasm32-unknown-unknown
```

The script builds the runner on first use, into `.tmp/wasm-test-runner/target`. Point `BMC_WASM_TEST_RUNNER` at a runner
binary to skip that build.

## What A Test Runs Against

Every test gets a fresh instance of the module, so nothing carries over from another test. The host around it is
hermetic:

- Network requests, WebSockets, TCP and TLS are refused, and a test ends before any of them delivers to the widget.
- KV storage lives in memory, and the asset cache is the test's own, capped at the size of a widget's bucket.
- The clock reads 2026-01-01 12:00 UTC throughout, and the random number generator has a fixed seed.
- `widget_viewport()` and `display_info()` report the default platform's first viewport.
- Text measures exactly as on the device. Asset tags register, but their payloads are neither decoded nor checked.
- Submitting a UI tree traps, and touch readbacks return nothing.

## Budgets And Output

Each test runs on the fuel budget of one device frame, and running out fails it with the budget in the message. A
wall-clock watchdog ends the process when a test runs past a minute. Both can be raised:

| Limit   | Flag                  | Environment             |
| ------- | --------------------- | ----------------------- |
| Fuel    | `--fuel <units>`      | `BMC_WASM_TEST_FUEL`    |
| Timeout | `--timeout <seconds>` | `BMC_WASM_TEST_TIMEOUT` |

The flags reach the runner after `--` in `cargo test`. nextest forwards only libtest's own arguments, so use the
environment there.

What the widget logs is kept and printed with a failing test. With `--nocapture`, or nextest's `--no-capture`, it prints
as it happens.

Tests from one binary run one at a time, since a runtime cannot leave the thread it was built on. nextest runs each test
in a process of its own, so it still runs them in parallel.
