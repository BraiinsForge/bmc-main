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

use std::io::Write;
use std::os::unix::net::UnixStream;

#[test]
fn idle_loop_errors_on_unsolicited_byte_from_host() {
    let (thin_side, host_side) =
        UnixStream::pair().expect("BUG: test fixture requires UnixStream::pair() to succeed");

    let handle = std::thread::spawn(move || bmc_wasm_thin::host_client::idle_until_exit(thin_side));

    std::thread::sleep(std::time::Duration::from_millis(50));
    (&host_side)
        .write_all(&[0x42])
        .expect("BUG: local socketpair write_all must succeed");

    let res = handle
        .join()
        .expect("BUG: idle_until_exit thread must not panic");
    assert!(
        res.is_err(),
        "expected idle loop to fail on unsolicited byte, got {res:?}"
    );
}
