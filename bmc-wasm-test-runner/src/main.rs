// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! `wasm-test`: runs a wasm32 test binary built with `#[wasm_test]` under the hermetic host,
//! behind the libtest command line that cargo, nextest and rust-analyzer drive.
//!
//! Cargo calls it as the wasm32 target's runner, with the test binary first and libtest's arguments after.
//! `--fuel <units>` and `--timeout <seconds>` are the runner's own and come off before libtest parses the rest;
//! each has an environment fallback, because nextest forwards only libtest's arguments.

mod listing;

use std::ffi::OsString;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use bmc_render::renderer::Renderer;
use bmc_render::renderer::test_support::ShapingRecorder;
use bmc_wasm_runtime::hermetic_host::HermeticHost;
use bmc_wasm_runtime::platform_catalog::{self, Target};
use bmc_wasm_runtime::{EntryPolicy, GuestLogSink, GuestPanic, TestExportError};
use bmc_wasm_test_abi::parse_fuel;
use indoc::formatdoc;
use libtest_mimic::{Arguments, Failed, Trial};

use listing::{ListedTest, list_tests};

const FUEL_ENV: &str = "BMC_WASM_TEST_FUEL";
const TIMEOUT_ENV: &str = "BMC_WASM_TEST_TIMEOUT";
/// nextest's default slow-timeout period, so a plain `cargo test` gives up where nextest would warn.
const DEFAULT_TIMEOUT: Duration = Duration::from_mins(1);
/// Libtest's exit code for a run that did not pass.
const FAILED: i32 = 101;

#[derive(Debug, Clone, Copy)]
struct Settings {
    /// `--fuel` or its environment fallback, which overrides every test's own budget.
    fuel: Option<u64>,
    timeout: Duration,
    nocapture: bool,
}

fn main() {
    let (test_binary, settings, mut arguments) = match parse(std::env::args_os()) {
        Ok(parsed) => parsed,
        Err(error) => exit_with(&error),
    };
    // A runtime cannot leave the thread it was built on, so each test runs on the main thread.
    arguments.test_threads = Some(1);
    match trials(&test_binary, settings) {
        Ok(trials) => libtest_mimic::run(&arguments, trials).exit(),
        Err(error) => exit_with(&error),
    }
}

fn exit_with(error: &anyhow::Error) -> ! {
    eprintln!("wasm-test: {error:#}");
    std::process::exit(FAILED)
}

fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<(PathBuf, Settings, Arguments)> {
    let mut arguments = arguments.into_iter();
    let program = arguments.next().context("no program name")?;
    let test_binary = arguments
        .next()
        .map(PathBuf::from)
        .context("usage: wasm-test <test binary .wasm> [libtest arguments]")?;
    let mut fuel = env_number(FUEL_ENV, parse_fuel)?;
    let mut timeout =
        env_number(TIMEOUT_ENV, seconds)?.map_or(DEFAULT_TIMEOUT, Duration::from_secs);
    let mut libtest = vec![program];
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--fuel") => fuel = Some(number("--fuel", arguments.next(), parse_fuel)?),
            Some("--timeout") => {
                timeout = Duration::from_secs(number("--timeout", arguments.next(), seconds)?);
            }
            _ => libtest.push(argument),
        }
    }
    let libtest = Arguments::from_iter(libtest);
    let settings = Settings {
        fuel,
        timeout,
        nocapture: libtest.nocapture,
    };
    Ok((test_binary, settings, libtest))
}

fn number(flag: &str, value: Option<OsString>, parse: fn(&str) -> Option<u64>) -> Result<u64> {
    let value = value.with_context(|| format!("{flag} takes a number"))?;
    value
        .to_str()
        .and_then(parse)
        .with_context(|| format!("{flag} takes a number, not `{}`", value.display()))
}

fn env_number(name: &str, parse: fn(&str) -> Option<u64>) -> Result<Option<u64>> {
    std::env::var_os(name)
        .map(|value| number(name, Some(value), parse))
        .transpose()
}

fn seconds(text: &str) -> Option<u64> {
    text.parse().ok()
}

/// The catalog's default platform at its first viewport.
fn target() -> Target {
    let platform = platform_catalog::default_platform();
    Target {
        platform,
        viewport: &platform.viewports[0],
    }
}

fn trials(test_binary: &std::path::Path, settings: Settings) -> Result<Vec<Trial>> {
    let host = Arc::new(HermeticHost::prepare(test_binary)?);
    let target = target();
    let tests = list_tests(&host, target)
        .with_context(|| format!("list the tests in {}", test_binary.display()))?;
    if tests.is_empty() {
        eprintln!(
            "wasm-test: {} holds no `#[wasm_test]`, so nothing ran on wasm32 \
             (doctests and plain `#[test]`s only run natively)",
            test_binary.display()
        );
    }
    Ok(tests
        .into_iter()
        .map(|test| {
            let host = Arc::clone(&host);
            let name = test.name.clone();
            let ignored = test.ignored;
            Trial::test(name, move || run(&host, target, &test, settings))
                .with_ignored_flag(ignored)
        })
        .collect())
}

fn run(
    host: &HermeticHost,
    target: Target,
    test: &ListedTest,
    settings: Settings,
) -> Result<(), Failed> {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut config = host.config().map_err(|error| failure(&error))?;
    config.fuel_per_frame = fuel_for(test, settings);
    config.guest_log_sink = Some(log_sink(Arc::clone(&log), settings.nocapture));
    let mut runtime = host
        .instantiate(EntryPolicy::TestModule, target, config)
        .map_err(|error| failure(&error))?;
    let side = |pixels: u32| {
        f32::from(u16::try_from(pixels).expect("BUG: catalog viewports are narrower than 65536 px"))
    };
    let mut renderer =
        ShapingRecorder::new(side(target.viewport.width), side(target.viewport.height));

    let watchdog = Watchdog::arm(&test.name, settings.timeout);
    let raw: *mut dyn Renderer = std::ptr::addr_of_mut!(renderer);
    let outcome = runtime.with_renderer(
        NonNull::new(raw).expect("BUG: addr_of_mut! is never null"),
        |runtime| runtime.call_test_export(&test.run_export),
    );
    drop(watchdog);

    judge(test, outcome, runtime.take_guest_panic()).map_err(|reason| {
        let log = log.lock().expect("BUG: the log lock is never poisoned");
        if log.is_empty() {
            Failed::from(reason)
        } else {
            Failed::from(format!("{reason}\n\nguest log:\n{}", log.join("\n")))
        }
    })
}

/// The budget a test runs on: the runner's override, then the test's own, then the default.
fn fuel_for(test: &ListedTest, settings: Settings) -> u64 {
    settings
        .fuel
        .or(test.fuel)
        .unwrap_or(HermeticHost::FUEL_PER_TEST)
}

fn failure(error: &anyhow::Error) -> Failed {
    Failed::from(format!("{error:#}"))
}

fn log_sink(log: Arc<Mutex<Vec<String>>>, nocapture: bool) -> GuestLogSink {
    Box::new(move |level, line| {
        let entry = format!("{level:>5} {line}");
        if nocapture {
            println!("{entry}");
        } else {
            log.lock()
                .expect("BUG: the log lock is never poisoned")
                .push(entry);
        }
    })
}

/// Libtest's verdict, with libtest's wording where it has one.
fn judge(
    test: &ListedTest,
    outcome: Result<(), TestExportError>,
    panic: Option<GuestPanic>,
) -> Result<(), String> {
    let error = match outcome {
        Ok(()) if test.should_panic.is_some() => {
            return Err("test did not panic as expected".to_owned());
        }
        Ok(()) => return Ok(()),
        Err(TestExportError::OutOfFuel(budget)) => {
            return Err(format!(
                "the test ran out of fuel after {budget} units; raise the budget with \
                 `#[wasm_test(fuel = <units>, reason = \"…\")]`, --fuel <units> or {FUEL_ENV}"
            ));
        }
        Err(error) => error,
    };
    match (panic, &test.should_panic) {
        (Some(panic), Some(expected)) if panic.message.contains(expected.as_str()) => Ok(()),
        (Some(panic), Some(expected)) => Err(formatdoc!(
            "
            panic did not contain expected string
                  panic message: {:?},
             expected substring: {expected:?}",
            panic.message
        )),
        (Some(panic), None) => Err(formatdoc!(
            "
            panicked at {}:{}:
            {}",
            panic.file,
            panic.line,
            panic.message
        )),
        (None, _) => Err(format!("{:#}", anyhow!(error))),
    }
}

/// Ends the process when a test blocks in host work past its timeout:
/// fuel stops a looping guest, but nothing can interrupt a host call from another thread.
struct Watchdog {
    _done: mpsc::Sender<()>,
}

impl Watchdog {
    fn arm(name: &str, timeout: Duration) -> Self {
        let (done, finished) = mpsc::channel::<()>();
        let name = name.to_owned();
        thread::spawn(move || {
            if finished.recv_timeout(timeout) == Err(RecvTimeoutError::Timeout) {
                // Off the line libtest left open with `test <name> ... `.
                eprintln!(
                    "\ntest {name} ran past its {}s timeout; raise it with --timeout <seconds> or {TIMEOUT_ENV}",
                    timeout.as_secs()
                );
                std::process::exit(FAILED);
            }
        });
        Self { _done: done }
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    fn listed(should_panic: Option<&str>) -> ListedTest {
        ListedTest {
            name: "tests::probe".to_owned(),
            run_export: "__bmc_test_run:probe::tests::probe".to_owned(),
            ignored: false,
            should_panic: should_panic.map(str::to_owned),
            fuel: None,
        }
    }

    fn panic(message: &str) -> GuestPanic {
        GuestPanic {
            message: message.to_owned(),
            file: "src/lib.rs".to_owned(),
            line: 7,
        }
    }

    fn trap() -> TestExportError {
        TestExportError::Trap(wasmi::Error::new("unreachable"))
    }

    #[test]
    fn a_panic_containing_the_expected_message_passes() {
        let verdict = judge(
            &listed(Some("month")),
            Err(trap()),
            Some(panic("month must be 1..=12")),
        );

        assert_eq!(verdict, Ok(()));
    }

    #[test]
    fn a_panic_without_the_expected_message_fails() {
        let verdict = judge(
            &listed(Some("month")),
            Err(trap()),
            Some(panic("day must be 1..=31")),
        )
        .expect_err("BUG: a panic of another message fails a should_panic test");

        assert_eq!(
            verdict,
            indoc! {r#"
                panic did not contain expected string
                      panic message: "day must be 1..=31",
                 expected substring: "month""#}
        );
    }

    #[test]
    fn a_trap_without_a_panic_fails_even_a_should_panic_test() {
        for should_panic in [None, Some("month")] {
            let verdict = judge(&listed(should_panic), Err(trap()), None)
                .expect_err("BUG: a bare trap is no panic, so it fails the test");

            assert!(
                verdict.contains("unreachable"),
                "{should_panic:?}: {verdict}"
            );
        }
    }

    #[test]
    fn a_should_panic_test_that_returns_fails() {
        assert!(judge(&listed(Some("month")), Ok(()), None).is_err());
    }

    #[test]
    fn an_unexpected_panic_reports_where_it_happened() {
        let verdict = judge(&listed(None), Err(trap()), Some(panic("boom")));

        assert_eq!(
            verdict,
            Err(indoc! {"
                panicked at src/lib.rs:7:
                boom"}
            .to_owned())
        );
    }

    #[test]
    fn running_out_of_fuel_names_the_knob() {
        let verdict = judge(&listed(None), Err(TestExportError::OutOfFuel(10)), None)
            .expect_err("BUG: running out of fuel fails the test");

        assert!(verdict.contains(FUEL_ENV), "{verdict}");
    }

    #[test]
    fn the_runner_takes_its_own_flags_off_the_libtest_arguments() {
        let (binary, settings, libtest) = parse(
            [
                "wasm-test",
                "tests.wasm",
                "--fuel",
                "5e2",
                "--exact",
                "tests::probe",
            ]
            .map(OsString::from),
        )
        .expect("BUG: a well-formed invocation parses");

        assert_eq!(
            (
                binary,
                settings.fuel,
                libtest.exact,
                libtest.filter.as_deref()
            ),
            (
                PathBuf::from("tests.wasm"),
                Some(500),
                true,
                Some("tests::probe")
            )
        );
    }

    #[test]
    fn the_runner_override_beats_the_test_budget_which_beats_the_default() {
        let settings = |fuel| Settings {
            fuel,
            timeout: DEFAULT_TIMEOUT,
            nocapture: false,
        };
        let budgeted = ListedTest {
            fuel: Some(300),
            ..listed(None)
        };

        assert_eq!(
            [
                fuel_for(&budgeted, settings(Some(500))),
                fuel_for(&budgeted, settings(None)),
                fuel_for(&listed(None), settings(None)),
            ],
            [500, 300, HermeticHost::FUEL_PER_TEST]
        );
    }
}
