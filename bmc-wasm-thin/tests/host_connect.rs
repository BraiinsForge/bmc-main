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

use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use bmc_wasm_thin::args::Config;
use bmc_wasm_thin::host_connect::connect;

fn config(host_socket: PathBuf, host_wait: Duration) -> Config {
    Config {
        wasm: PathBuf::from("widget.wasm"),
        asset_root: None,
        host_socket,
        host_wait,
        ack_wait: Duration::from_secs(10),
    }
}

#[test]
fn connects_to_a_running_host() {
    let temp = tempfile::tempdir().expect("BUG: create temporary directory");
    let socket = temp.path().join("host.sock");
    let listener = UnixListener::bind(&socket).expect("BUG: bind host socket");

    let stream = connect(&config(socket, Duration::ZERO));

    assert!(
        stream.is_ok(),
        "a running host must be immediately available"
    );
    drop(listener);
}

#[test]
fn waits_for_the_compositor_to_start_the_host() {
    let temp = tempfile::tempdir().expect("BUG: create temporary directory");
    let socket = temp.path().join("host.sock");
    let server_socket = socket.clone();
    let server = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        let listener = UnixListener::bind(server_socket).expect("BUG: bind delayed host socket");
        listener.accept().expect("BUG: accept thin connection");
    });

    connect(&config(socket, Duration::from_secs(1)))
        .expect("thin must wait for the compositor-owned host");

    server.join().expect("BUG: delayed host thread must finish");
}

#[test]
fn missing_host_times_out_without_creating_its_socket_directory() {
    let temp = tempfile::tempdir().expect("BUG: create temporary directory");
    let socket = temp.path().join("missing").join("host.sock");
    let started = Instant::now();

    let error = connect(&config(socket.clone(), Duration::from_millis(50)))
        .expect_err("a missing compositor-owned host must time out");

    assert!(
        error
            .to_string()
            .contains("timed out waiting for bmc-wasm-host")
    );
    assert!(started.elapsed() >= Duration::from_millis(45));
    assert!(
        !socket.parent().is_some_and(Path::exists),
        "the thin must not create host-owned paths"
    );
}
