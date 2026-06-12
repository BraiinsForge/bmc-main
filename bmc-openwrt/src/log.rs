// Copyright (C) 2025  Braiins Systems s.r.o.

use std::panic::PanicHookInfo;
use std::path::Path;
use std::thread;
use tracing::error;

const BMC_LOG_PATH: &str = "/var/log/bmc/bmc.log";

const WIDGETS_LOG_PATH: &str = "/var/log/bmc/widgets.log";

/// Initialize tracing via [`bmc_log`]: rotated log files with
/// `log_to_file` (bmc's own events plus captured widget output),
/// stderr otherwise.
///
/// # Panics
///
/// Panics when `log_to_file` is set but the log files cannot be opened:
/// silently falling back to stderr would stream unbounded logs into
/// syslog via procd, which file logging exists to avoid.
pub fn init(log_to_file: bool) {
    if log_to_file {
        bmc_log::init_file_with_widget_capture(
            Path::new(BMC_LOG_PATH),
            Path::new(WIDGETS_LOG_PATH),
        )
        .expect("BUG: --log-to-file given but the log files cannot be opened");
    } else {
        bmc_log::init_console();
    }
}

/// Build a panic hook that prints the panic message and a backtrace.
/// This is the same as the default panic hook, but it also uses the tracing
/// log to print the panic message and backtrace.
#[must_use]
pub fn build_panic_hook_with_tracing() -> Box<dyn Fn(&PanicHookInfo<'_>) + Send + Sync> {
    Box::new(|info| {
        // If this is a double panic, make sure that we print a backtrace
        // for this panic. Otherwise only print it if logging is enabled.
        let backtrace = std::backtrace::Backtrace::capture();

        // The current std implementation always returns `Some` but this is not guaranteed in the future
        let location = if let Some(l) = info.location() {
            l.to_string()
        } else {
            "unknown location".to_owned()
        };

        // Print the panic message if it is a string or write "Box<dyn Any>" otherwise that is same as
        // default panic hook.
        let msg = match info.payload().downcast_ref::<&'static str>() {
            Some(s) => *s,
            None => match info.payload().downcast_ref::<String>() {
                #[expect(clippy::string_slice)]
                Some(s) => &s[..],
                None => "Box<dyn Any>",
            },
        };
        let thread = thread::current();
        let name = thread.name().unwrap_or("<unnamed>");

        // It is safe to use the tracing log here because in bmc we dont use realtime threads
        error!("thread '{name}' panicked at '{msg}', {location}");
        error!("{backtrace:#}");
        // If the backtrace is disabled, print a note sam as the default panic hook
        if backtrace.status() == std::backtrace::BacktraceStatus::Disabled {
            error!("note: the `RUST_BACKTRACE=full` environment variable may help in debugging");
        }
    })
}
