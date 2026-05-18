// Copyright (C) 2026  Braiins Systems s.r.o.

//! Test helper binary used by `tests/spawn_lock.rs`. The thin spawn path
//! exec's `Config::host_bin`; pointing it at this binary stands in for a
//! real `bmc-wasm-host` during tests.

use std::env;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::Write as IoWrite;
use std::os::unix::net::UnixListener;
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
    if let Ok(report_path) = env::var("BMC_THIN_FAKE_HOST_FD_REPORT") {
        report_inherited_fds(Path::new(&report_path), lock_fd);
    }
    let listener = UnixListener::bind(&socket).expect("BUG: fake host bind");
    std::thread::sleep(Duration::from_millis(100));
    unsafe {
        libc::close(lock_fd);
    }
    let mut accepted = Vec::new();
    for _ in 0..accepts {
        let (stream, _addr) = listener.accept().expect("BUG: fake host accept");
        accepted.push(stream);
    }
    std::process::exit(0);
}

fn report_inherited_fds(report_path: &Path, lock_fd: i32) {
    // opendir/readdir directly so we can identify and filter the
    // inspection fd; std::fs::read_dir would hold an additional fd we
    // cannot identify.
    let dirp = unsafe { libc::opendir(c"/proc/self/fd".as_ptr()) };
    assert!(!dirp.is_null(), "BUG: opendir /proc/self/fd");
    let inspect_fd = unsafe { libc::dirfd(dirp) };
    let mut fds: Vec<i32> = Vec::new();
    loop {
        let entry = unsafe { libc::readdir(dirp) };
        if entry.is_null() {
            break;
        }
        let name_ptr = unsafe { (*entry).d_name.as_ptr() };
        let cstr = unsafe { std::ffi::CStr::from_ptr(name_ptr) };
        let s = cstr.to_string_lossy();
        if let Ok(fd) = s.parse::<i32>()
            && fd != inspect_fd
        {
            fds.push(fd);
        }
    }
    unsafe {
        libc::closedir(dirp);
    }
    fds.retain(|&fd| fd > 2);
    let mut out = File::create(report_path).expect("BUG: create fd report");
    let mut line = String::new();
    for fd in &fds {
        writeln!(line, "{fd}").expect("BUG: write fd line");
    }
    writeln!(line, "LOCK={lock_fd}").expect("BUG: write lock line");
    IoWrite::write_all(&mut out, line.as_bytes()).expect("BUG: write fd report");
}
