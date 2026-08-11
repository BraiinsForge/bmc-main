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

//! Test helper binary used by `tests/spawn_lock.rs`. The thin spawn path
//! exec's `Config::host_bin`; pointing it at this binary stands in for a
//! real `bmc-wasm-host` during tests.

use std::env;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io;
use std::io::Write as IoWrite;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

fn main() {
    let socket = env::var("BMC_THIN_FAKE_HOST_SOCKET").expect("BUG: fake host socket env");
    let lock_fd: i32 = env::var("BMC_THIN_FAKE_HOST_RELEASE_LOCK_FD")
        .expect("BUG: fake host lock fd env")
        .parse()
        .expect("BUG: fake host lock fd integer");
    let accepts: usize = env::var("BMC_THIN_FAKE_HOST_ACCEPTS")
        .ok()
        .map_or(1, |s| s.parse().expect("BUG: fake host accepts integer"));
    if env::var_os("BMC_THIN_FAKE_HOST_IGNORE_TERM").is_some() {
        unsafe {
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
        }
    }
    if let Ok(report_path) = env::var("BMC_THIN_FAKE_HOST_FD_REPORT") {
        report_inherited_fds(Path::new(&report_path), lock_fd);
    }
    let listener = bind_listener(Path::new(&socket));
    std::thread::sleep(Duration::from_millis(100));
    unsafe {
        libc::close(lock_fd);
    }
    let err_ack_drop_marker = env::var("BMC_THIN_FAKE_HOST_ERR_ACK_DROP").ok();
    let mut accepted = Vec::new();
    for _ in 0..accepts {
        let (mut stream, _addr) = listener.accept().expect("BUG: fake host accept");
        if let Some(marker) = &err_ack_drop_marker {
            IoWrite::write_all(&mut stream, b"err-ack").expect("BUG: fake host err ack");
            drop(stream);
            File::create(marker).expect("BUG: fake host err ack marker");
        } else {
            accepted.push(stream);
        }
    }
    if env::var_os("BMC_THIN_FAKE_HOST_HOLD").is_some() {
        loop {
            std::thread::park();
        }
    }
    std::process::exit(0);
}

fn bind_listener(socket: &Path) -> UnixListener {
    match UnixListener::bind(socket) {
        Ok(listener) => listener,
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            match UnixStream::connect(socket) {
                Ok(_) => panic!("BUG: fake host socket already has a listener"),
                Err(connect_error) if connect_error.raw_os_error() == Some(libc::ECONNREFUSED) => {
                    std::fs::remove_file(socket).expect("BUG: remove stale fake host socket");
                    UnixListener::bind(socket).expect("BUG: fake host rebind")
                }
                Err(connect_error) => panic!("BUG: fake host stale probe: {connect_error}"),
            }
        }
        Err(error) => panic!("BUG: fake host bind: {error}"),
    }
}

fn report_inherited_fds(report_path: &Path, lock_fd: i32) {
    // Snapshot /proc/self/fd, then drop the iterator so its transient
    // dirfd is closed. Filter the snapshot by re-checking each entry via
    // `read_link` on `/proc/self/fd/<n>`: read_link does not allocate a
    // new fd, so the dirfd's symlink now resolves to ENOENT and that
    // entry drops out, leaving only the stable application fds.
    let reader = std::fs::read_dir("/proc/self/fd").expect("BUG: read_dir /proc/self/fd");
    let snapshot: Vec<i32> = reader
        .filter_map(|entry| {
            let entry = entry.expect("BUG: read_dir entry");
            entry.file_name().to_string_lossy().parse::<i32>().ok()
        })
        .collect();
    let mut fds: Vec<i32> = snapshot
        .into_iter()
        .filter(|&fd| std::fs::read_link(format!("/proc/self/fd/{fd}")).is_ok())
        .collect();
    fds.retain(|&fd| fd > 2);
    fds.sort_unstable();
    let mut out = File::create(report_path).expect("BUG: create fd report");
    let mut line = String::new();
    for fd in &fds {
        writeln!(line, "{fd}").expect("BUG: write fd line");
    }
    writeln!(line, "LOCK={lock_fd}").expect("BUG: write lock line");
    IoWrite::write_all(&mut out, line.as_bytes()).expect("BUG: write fd report");
}
