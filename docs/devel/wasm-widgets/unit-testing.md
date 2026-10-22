# WASM Widget Unit Testing

A widget's unit tests compile to `wasm32` and run under the wasm test runner, in `wasmi`, the interpreter the BMC
application runs widgets in. Import `bmc_wasm_sdk::wasm_test` and mark a test with `#[wasm_test]` where a native test
would take `#[test]`.

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
becomes a plain `#[test]`, but widget tests run on `wasm32` only: natively, a test that builds an asset-bearing node
panics for want of an asset host.

A plain `#[test]` never reaches the runner, so the `no-plain-test-in-widgets` ast-grep rule refuses one in
`widgets-wasm/` and `widgets-wasm-examples/`. Its fix swaps in `#[wasm_test]`, which is how the widgets were ported; the
module then needs `use bmc_wasm_sdk::wasm_test;`:

```sh
nix run .#ast-grep -- --filter no-plain-test-in-widgets --update-all
```

A test that installs a system snapshot with `system::set_current` needs the SDK's `test-support` feature:

```toml
[dev-dependencies]
bmc-wasm-sdk = { path = "../../bmc-wasm-runtime/sdk", features = ["test-support"] }
```

## Run Tests

```sh
just test-widget clock
just test-widget clock screens
just test-wasm-widgets
just test-wasm-sdk
```

`test-widget` takes a widget's directory name, or a shared library's under `lib/`. Whatever follows the name reaches
nextest, so a substring of the test names narrows the run. `test-wasm-widgets` runs every widget's tests in both widget
workspaces, as `just validate` does, and `test-wasm-sdk` runs the SDK's own tests.

Plain cargo works as well. `.cargo/config.toml` names `scripts/wasm_test_runner.sh` as the runner for `wasm32` binaries,
so any test command with `--target wasm32-unknown-unknown` goes through it:

```sh
cargo nextest run --manifest-path widgets-wasm/Cargo.toml -p clock --target wasm32-unknown-unknown
cargo test --manifest-path widgets-wasm/Cargo.toml -p clock --target wasm32-unknown-unknown
```

Inside `widgets-wasm/` and `widgets-wasm-examples/`, their own `.cargo/config.toml` makes `wasm32` the build target, so
a bare `cargo test` there, or rust-analyzer's Run Test, goes through the runner too.

Runnable doctests don't run on `wasm32`: a doctest binary holds no `#[wasm_test]`s, so the runner reports `0 passed`.
Run them natively, with `--target` set to the host. `compile_fail` doctests are the exception: rustdoc only compiles
them, so check them on `wasm32`, where every item they name exists.

The script builds the runner on first use, into `.tmp/wasm-test-runner/target`. Point `BMC_WASM_TEST_RUNNER` at a runner
binary to skip that build.

nextest warns that it `saw package ID … which wasn't produced by cargo metadata` for crates a build script or proc macro
pulls in on the host only, such as `libc`. Its package graph for `wasm32` leaves them out, and since they hold no tests,
the warning is harmless.

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

Each test runs on the fuel budget of one device frame, and running out fails it with the budget in the message. A test
that needs more names its own budget, and the attribute takes one only with a reason:

```rust
#[wasm_test(fuel = 250e6, reason = "parses 2,048 candles")]
```

Fuel is a whole number of units, written out or in exponent form.

Fuel stops a looping guest, but not a host call that never returns. nextest ends a wasm32 test after a minute, per the
`slow-timeout` in each workspace's `.config/nextest.toml`. A plain `cargo test` has no timeout, like a native one.

What the widget logs is kept and printed with a failing test. With `--nocapture`, or nextest's `--no-capture`, it prints
as it happens.

Tests run in parallel, each on a runtime of its own, as libtest runs native ones. `--test-threads 1` runs them serially.
