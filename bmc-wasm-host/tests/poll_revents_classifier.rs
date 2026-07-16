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

use bmc_wasm_host::main_loop::{
    FatalError, PollDecision, classify_listener_revents, classify_poll_errno,
};

#[test]
fn listener_pollin_is_ok() {
    classify_listener_revents(libc::POLLIN).expect("BUG: POLLIN is not a fatal listener revent");
}

#[test]
fn listener_zero_is_ok() {
    classify_listener_revents(0).expect("BUG: zero revents is not fatal");
}

#[test]
fn listener_pollerr_is_fatal() {
    assert!(matches!(
        classify_listener_revents(libc::POLLERR).expect_err("BUG: POLLERR must be fatal"),
        FatalError::ListenerLost(_),
    ));
}

#[test]
fn listener_pollhup_is_fatal() {
    assert!(matches!(
        classify_listener_revents(libc::POLLHUP).expect_err("BUG: POLLHUP must be fatal"),
        FatalError::ListenerLost(_),
    ));
}

#[test]
fn listener_pollnval_is_fatal() {
    assert!(matches!(
        classify_listener_revents(libc::POLLNVAL).expect_err("BUG: POLLNVAL must be fatal"),
        FatalError::ListenerLost(_),
    ));
}

#[test]
fn listener_pollin_with_pollerr_is_fatal() {
    assert!(matches!(
        classify_listener_revents(libc::POLLIN | libc::POLLERR)
            .expect_err("BUG: POLLERR remains fatal when POLLIN is also set"),
        FatalError::ListenerLost(_),
    ));
}

#[test]
fn poll_eintr_means_retry() {
    let e = std::io::Error::from_raw_os_error(libc::EINTR);
    assert!(matches!(classify_poll_errno(&e), PollDecision::Retry));
}

#[test]
fn poll_other_errno_is_fatal() {
    let e = std::io::Error::from_raw_os_error(libc::EBADF);
    assert!(matches!(classify_poll_errno(&e), PollDecision::Fatal(_)));
}
