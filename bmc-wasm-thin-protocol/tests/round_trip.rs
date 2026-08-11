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

use std::io::Read;
use std::mem::size_of;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use bmc_wasm_thin_protocol::{
    AckDecoder, AckMsg, HelloMsg, MAX_FRAME_LEN, PROTOCOL_VERSION, recv_hello_with_fd,
    send_hello_with_fd, write_ack,
};
use serial_test::serial;

fn decode_ack(sock: &UnixStream) -> std::io::Result<AckMsg> {
    let mut dec = AckDecoder::new();
    let mut buf = [0_u8; 64];
    let mut sock_r = sock;
    loop {
        let n = sock_r.read(&mut buf)?;
        if let Some(msg) = dec.push(&buf[..n])? {
            return Ok(msg);
        }
    }
}

fn pair() -> (UnixStream, UnixStream) {
    UnixStream::pair()
        .expect("BUG: test fixture requires UnixStream::pair() to succeed on this platform")
}

fn int_cmsg_space_for(n: usize) -> usize {
    unsafe {
        libc::CMSG_SPACE(
            u32::try_from(n * size_of::<libc::c_int>())
                .expect("BUG: n * sizeof(c_int) fits in u32 for small n"),
        ) as usize
    }
}

fn int_cmsg_len_for(n: usize) -> usize {
    unsafe {
        libc::CMSG_LEN(
            u32::try_from(n * size_of::<libc::c_int>())
                .expect("BUG: n * sizeof(c_int) fits in u32 for small n"),
        ) as usize
    }
}

const FRAME_HEADER_LEN: usize = 6;

fn build_frame_bytes<E: bincode::Encode>(version: u16, payload: &E) -> Vec<u8> {
    let body = bincode::encode_to_vec(payload, bincode::config::standard())
        .expect("BUG: encode HelloMsg/AckMsg payload");
    let frame_len = u32::try_from(body.len()).expect("BUG: encoded body fits u32");
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + body.len());
    frame.extend_from_slice(&version.to_le_bytes());
    frame.extend_from_slice(&frame_len.to_le_bytes());
    frame.extend_from_slice(&body);
    frame
}

fn raw_sendmsg_with_fd(sender: &UnixStream, payload: &[u8], fd: std::os::fd::BorrowedFd<'_>) {
    let iov = libc::iovec {
        iov_base: payload.as_ptr().cast_mut().cast::<libc::c_void>(),
        iov_len: payload.len(),
    };
    let fd_int: libc::c_int = fd.as_raw_fd();
    let cmsg_space = int_cmsg_space_for(1);
    let mut cmsg_buf = vec![0_u8; cmsg_space];

    let hdr = libc::msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: (&raw const iov).cast_mut(),
        msg_iovlen: 1,
        msg_control: cmsg_buf.as_mut_ptr().cast::<libc::c_void>(),
        msg_controllen: cmsg_space,
        msg_flags: 0,
    };
    unsafe {
        let cmsg: *mut libc::cmsghdr = libc::CMSG_FIRSTHDR(&raw const hdr);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = int_cmsg_len_for(1);
        // CMSG_DATA alignment is guaranteed by the kernel ABI.
        std::ptr::write(libc::CMSG_DATA(cmsg).cast::<libc::c_int>(), fd_int);
        let n = libc::sendmsg(sender.as_raw_fd(), &raw const hdr, libc::MSG_NOSIGNAL);
        assert!(
            n > 0,
            "raw sendmsg must succeed: {}",
            std::io::Error::last_os_error(),
        );
    }
}

#[test]
#[serial]
fn hello_with_fd_round_trip() {
    let (a, b) = pair();
    let (fd_a, _fd_b) = pair();

    send_hello_with_fd(
        &a,
        &HelloMsg::Load {
            wasm_path: "/path/to/widget.wasm".into(),
        },
        fd_a.as_fd(),
    )
    .expect(
        "BUG: test fixture expects send_hello_with_fd to succeed on connected local socket pair",
    );

    let (msg, recovered_fd) = recv_hello_with_fd(&b).expect("BUG: test fixture expects recv_hello_with_fd to decode bytes written by send_hello_with_fd");
    match msg {
        HelloMsg::Load { wasm_path } => assert_eq!(wasm_path, "/path/to/widget.wasm"),
    }
    assert!(
        recovered_fd.as_raw_fd() >= 0,
        "fd should be a valid kernel handle"
    );
}

#[test]
#[serial]
fn ack_ok_and_err_round_trip() {
    let (writer, reader) = pair();
    write_ack(&writer, &AckMsg::Ok).expect("BUG: local socketpair write_ack Ok must succeed");
    assert!(matches!(
        decode_ack(&reader).expect("BUG: local socketpair decode_ack Ok must succeed"),
        AckMsg::Ok,
    ));

    let (writer2, reader2) = pair();
    write_ack(&writer2, &AckMsg::Err("boom".into()))
        .expect("BUG: local socketpair write_ack Err must succeed");
    match decode_ack(&reader2).expect("BUG: local socketpair decode_ack Err must succeed") {
        AckMsg::Err(msg) => assert_eq!(msg, "boom"),
        AckMsg::Ok => panic!("expected Err"),
    }
}

#[test]
#[serial]
fn oversize_frame_is_rejected_before_alloc() {
    // The receiver checks the cmsg before parsing the frame length, so we attach a valid
    // SCM_RIGHTS payload to actually exercise the MAX_FRAME_LEN cap on the receiver. We
    // splice a too-large frame_len field by hand because no real encoded payload can
    // legitimately exceed MAX_FRAME_LEN (build_frame on the sender side rejects it first).
    let (sender, receiver) = pair();
    let (fd_a, _fd_b) = pair();

    let oversize_len: u32 = MAX_FRAME_LEN + 1;
    let mut payload: Vec<u8> = Vec::with_capacity(FRAME_HEADER_LEN);
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    payload.extend_from_slice(&oversize_len.to_le_bytes());

    raw_sendmsg_with_fd(&sender, &payload, fd_a.as_fd());
    drop(sender);

    let err = recv_hello_with_fd(&receiver).expect_err("BUG: oversize frame_len must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let msg = err.to_string();
    assert!(
        msg.contains("MAX_FRAME_LEN"),
        "oversize-length branch must produce the size-cap error, got: {msg}",
    );
}

#[test]
#[serial]
fn hello_without_scm_rights_is_protocol_error() {
    use std::io::Write;

    let (sender, receiver) = pair();
    let frame = build_frame_bytes(
        PROTOCOL_VERSION,
        &HelloMsg::Load {
            wasm_path: "/x".into(),
        },
    );
    (&sender)
        .write_all(&frame)
        .expect("BUG: local socketpair write_all must succeed");
    drop(sender);

    let err = recv_hello_with_fd(&receiver).expect_err("BUG: missing SCM_RIGHTS must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[cfg(target_os = "linux")]
#[test]
#[serial]
fn too_many_fds_are_rejected_without_leaking_received_fd() {
    fn fd_target(fd: i32) -> std::path::PathBuf {
        std::fs::read_link(format!("/proc/self/fd/{fd}"))
            .expect("BUG: linux test fixture expects fd links in /proc/self/fd")
    }

    fn open_fd_count_for_target(target: &Path) -> usize {
        std::fs::read_dir("/proc/self/fd")
            .expect("BUG: linux test fixture expects /proc/self/fd")
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|link| link == target)
            .count()
    }

    let (sender, receiver) = pair();
    let (fd_a, _fd_b) = pair();
    let (extra_a, _extra_b) = pair();
    let primary_target = fd_target(fd_a.as_raw_fd());
    let extra_target = fd_target(extra_a.as_raw_fd());

    let payload = build_frame_bytes(
        PROTOCOL_VERSION,
        &HelloMsg::Load {
            wasm_path: "/x".into(),
        },
    );

    let iov = libc::iovec {
        iov_base: payload.as_ptr().cast_mut().cast::<libc::c_void>(),
        iov_len: payload.len(),
    };
    let cmsg_space = int_cmsg_space_for(2);
    let mut cmsg_buf = vec![0_u8; cmsg_space];

    let hdr = libc::msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: (&raw const iov).cast_mut(),
        msg_iovlen: 1,
        msg_control: cmsg_buf.as_mut_ptr().cast::<libc::c_void>(),
        msg_controllen: cmsg_space,
        msg_flags: 0,
    };
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const hdr);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = int_cmsg_len_for(2);
        // CMSG_DATA alignment is guaranteed by the kernel ABI.
        let data = libc::CMSG_DATA(cmsg).cast::<libc::c_int>();
        std::ptr::write(data, fd_a.as_raw_fd());
        std::ptr::write(data.add(1), extra_a.as_raw_fd());
        let n = libc::sendmsg(sender.as_raw_fd(), &raw const hdr, libc::MSG_NOSIGNAL);
        assert!(
            n > 0,
            "raw sendmsg with two fds must succeed: {}",
            std::io::Error::last_os_error(),
        );
    }

    let primary_baseline = open_fd_count_for_target(&primary_target);
    let extra_baseline = open_fd_count_for_target(&extra_target);
    let err = recv_hello_with_fd(&receiver).expect_err("BUG: too many fds must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        open_fd_count_for_target(&primary_target),
        primary_baseline,
        "recv_hello_with_fd must close any primary fd it received before rejecting extra fds",
    );
    assert_eq!(
        open_fd_count_for_target(&extra_target),
        extra_baseline,
        "recv_hello_with_fd must close any extra fd it received before rejecting extra fds",
    );
}

#[test]
#[serial]
fn hello_with_wrong_version_is_rejected_before_payload() {
    let (a, b) = pair();
    let dummy = UnixStream::pair()
        .expect("BUG: test fixture requires UnixStream::pair to succeed")
        .0;

    let frame = build_frame_bytes(
        0xFFFF,
        &HelloMsg::Load {
            wasm_path: "/tmp/x.wasm".into(),
        },
    );
    raw_sendmsg_with_fd(&a, &frame, dummy.as_fd());

    let err = recv_hello_with_fd(&b).expect_err("must reject bogus version");
    assert!(err.to_string().contains("protocol version"), "got {err}");
}

#[test]
#[serial]
fn ack_decoder_rejects_runaway_input() {
    let mut dec = AckDecoder::new();
    // Feed a header that claims a body almost as large as MAX_FRAME_LEN, then drip bytes until
    // the decoder's buffer-cap check trips.
    let oversize: u32 = MAX_FRAME_LEN;
    let mut header = PROTOCOL_VERSION.to_le_bytes().to_vec();
    header.extend_from_slice(&oversize.to_le_bytes());
    dec.push(&header)
        .expect("BUG: AckDecoder must accept the frame header");
    let chunk = vec![0_u8; 8 * 1024];
    let mut last: Result<Option<_>, _> = Ok(None);
    for _ in 0..32 {
        last = dec.push(&chunk);
        if last.is_err() {
            break;
        }
    }
    assert!(
        last.is_err(),
        "decoder must bail once internal buffer exceeds the cap"
    );
}

#[test]
#[serial]
fn ack_decoder_rejects_oversize_frame_len() {
    let mut dec = AckDecoder::new();
    let oversize: u32 = MAX_FRAME_LEN + 1;
    let mut header = PROTOCOL_VERSION.to_le_bytes().to_vec();
    header.extend_from_slice(&oversize.to_le_bytes());
    let err = dec
        .push(&header)
        .expect_err("oversize frame_len must reject");
    assert!(err.to_string().contains("MAX_FRAME_LEN"), "got {err}");
}

#[test]
#[serial]
fn ack_decoder_rejects_wrong_version() {
    let mut dec = AckDecoder::new();
    let mut header = 0xFFFF_u16.to_le_bytes().to_vec();
    header.extend_from_slice(&0_u32.to_le_bytes());
    let err = dec.push(&header).expect_err("wrong version must reject");
    assert!(err.to_string().contains("protocol version"), "got {err}");
}
