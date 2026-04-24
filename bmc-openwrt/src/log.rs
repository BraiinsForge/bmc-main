// Copyright (C) 2025  Braiins Systems s.r.o.

use file_rotate::compression::Compression;
use file_rotate::suffix::AppendCount;
use file_rotate::{ContentLimit, FileRotate};
use std::panic::PanicHookInfo;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use tracing::{Level, error};
use tracing_subscriber::filter::{Directive, FilterExt, LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const LOGS_PATH: &str = "/var/log";

const BMC_LOG_FILE: &str = "bmc/bmc.log";

/// Rotate the log file after crossing this threshold.
const BMC_LOG_ROTATE_THRESHOLD: usize = 512 * 1024;

/// Keep this number of old compressed files.
const BMC_LOG_ROTATE_FILES_KEEP: usize = 9;

/// Initialize tracing-subscriber. If `log_to_file` is false, then a trivial
/// env-filtered subscriber is used (`RUST_LOG`, defaults to `info`).
/// FileRotate is used instead of logrotate.
pub fn init(log_to_file: bool) {
    if log_to_file {
        let log_path = Path::new(LOGS_PATH).join(BMC_LOG_FILE);
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)
                .expect("BUG: cannot create log directory — FileRotate would fail on first write");
        }
        let bmc_writer = FileRotate::new(
            log_path,
            AppendCount::new(BMC_LOG_ROTATE_FILES_KEEP),
            ContentLimit::BytesSurpassed(BMC_LOG_ROTATE_THRESHOLD),
            Compression::OnRotate(0),
            None,
        );

        // Apply environment-based filtering (RUST_LOG) with TRACE as default
        let log_filter = Targets::new()
            .with_default(LevelFilter::TRACE)
            .and(env_filter());

        let bmc_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(Mutex::new(bmc_writer))
            .with_filter(log_filter);

        // this sends all log events to both layers
        tracing_subscriber::registry().with(bmc_layer).init();
    } else {
        bmc::log::init();
    }
}

/// Returns an EnvFilter that filters spans and events based on the standard
/// env variable `RUST_LOG`, and uses `info` if the variable is not present.
/// EnvFilter !impl Clone --> It needs to be created from scratch for each use.
fn env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive(Directive::from(Level::INFO)))
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
