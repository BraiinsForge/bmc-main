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

//! The `log_*!` macros called from outside the SDK, as a widget calls them.

use std::io;
use std::sync::{Arc, Mutex};

// `fmt!` expands to a `uwrite!` that resolves `ufmt` in the caller's scope.
use bmc_wasm_sdk::ufmt;
use tracing_subscriber::util::SubscriberInitExt as _;

struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("BUG: no test panics while holding the capture")
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// What `log` sends to `tracing`,
/// one `LEVEL message` line per event.
fn captured(log: impl FnOnce()) -> String {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&bytes);
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || Capture(Arc::clone(&sink)))
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .finish();
    let guard = subscriber.set_default();
    log();
    drop(guard);

    let bytes = bytes
        .lock()
        .expect("BUG: no test panics while holding the capture")
        .clone();
    String::from_utf8(bytes).expect("BUG: tracing writes UTF-8")
}

#[test]
fn each_macro_reaches_tracing_at_its_level() {
    let logged = captured(|| {
        bmc_wasm_sdk::log_debug!("d {}", 1);
        bmc_wasm_sdk::log_info!("i {}", 2);
        bmc_wasm_sdk::log_warn!("w {}", 3);
        bmc_wasm_sdk::log_error!("e {}", 4);
    });
    assert_eq!(
        logged.lines().map(str::trim).collect::<Vec<_>>(),
        ["DEBUG d 1", "INFO i 2", "WARN w 3", "ERROR e 4"]
    );
}
