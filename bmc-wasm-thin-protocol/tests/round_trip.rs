// Copyright (C) 2026  Braiins Systems s.r.o.

use std::mem::size_of;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::net::UnixStream;

use bmc_wasm_thin_protocol::{
    AckMsg, HelloMsg, PROTOCOL_VERSION, read_ack, recv_hello_with_fd, send_hello_with_fd, write_ack,
};

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

#[test]
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
fn ack_ok_and_err_round_trip() {
    let (writer, reader) = pair();
    write_ack(&writer, &AckMsg::Ok).expect("BUG: local socketpair write_ack Ok must succeed");
    assert!(matches!(
        read_ack(&reader).expect("BUG: local socketpair read_ack Ok must succeed"),
        AckMsg::Ok,
    ));

    let (writer2, reader2) = pair();
    write_ack(&writer2, &AckMsg::Err("boom".into()))
        .expect("BUG: local socketpair write_ack Err must succeed");
    match read_ack(&reader2).expect("BUG: local socketpair read_ack Err must succeed") {
        AckMsg::Err(msg) => assert_eq!(msg, "boom"),
        AckMsg::Ok => panic!("expected Err"),
    }
}

#[test]
fn oversize_string_is_rejected_before_alloc() {
    // The receiver checks the cmsg before parsing the length, so we have to attach a
    // valid SCM_RIGHTS payload to actually exercise the 64 KiB cap on `wasm_path`.
    // We bypass `send_hello_with_fd` because its `write_lenstr` rejects oversize bytes
    // on the sender side; the cap on the receiver is what we want to lock in.
    let (sender, receiver) = pair();
    let (fd_a, _fd_b) = pair();

    let oversize_len: u32 = (64 * 1024) + 1;
    let mut payload: Vec<u8> = Vec::with_capacity(7);
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    payload.push(0x01_u8); // TAG_HELLO_LOAD
    payload.extend_from_slice(&oversize_len.to_le_bytes());

    let iov = libc::iovec {
        iov_base: payload.as_ptr().cast_mut().cast::<libc::c_void>(),
        iov_len: payload.len(),
    };
    let fd_int: libc::c_int = fd_a.as_raw_fd();
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
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
        )]
        std::ptr::write(libc::CMSG_DATA(cmsg).cast::<libc::c_int>(), fd_int);
        let n = libc::sendmsg(sender.as_raw_fd(), &raw const hdr, libc::MSG_NOSIGNAL);
        assert!(
            n > 0,
            "raw sendmsg must succeed: {}",
            std::io::Error::last_os_error(),
        );
    }
    drop(sender);

    let err =
        recv_hello_with_fd(&receiver).expect_err("BUG: oversize wasm_path length must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    let msg = err.to_string();
    assert!(
        msg.contains("64 KiB") || msg.contains("wasm_path exceeds"),
        "oversize-length branch must produce the size-cap error, got: {msg}",
    );
}

#[test]
fn hello_without_scm_rights_is_protocol_error() {
    use std::io::Write;

    let (sender, receiver) = pair();
    let path = "/x".as_bytes();
    let mut buf = PROTOCOL_VERSION.to_le_bytes().to_vec();
    buf.push(0x01_u8); // TAG_HELLO_LOAD
    buf.extend_from_slice(
        &u32::try_from(path.len())
            .expect("BUG: path length fits in u32")
            .to_le_bytes(),
    );
    buf.extend_from_slice(path);
    (&sender)
        .write_all(&buf)
        .expect("BUG: local socketpair write_all must succeed");
    drop(sender);

    let err = recv_hello_with_fd(&receiver).expect_err("BUG: missing SCM_RIGHTS must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
}

#[cfg(target_os = "linux")]
#[test]
fn too_many_fds_are_rejected_without_leaking_received_fd() {
    fn open_fd_count() -> usize {
        std::fs::read_dir("/proc/self/fd")
            .expect("BUG: linux test fixture expects /proc/self/fd")
            .count()
    }

    let (sender, receiver) = pair();
    let (fd_a, _fd_b) = pair();
    let (extra_a, _extra_b) = pair();

    let path = "/x".as_bytes();
    let mut payload = PROTOCOL_VERSION.to_le_bytes().to_vec();
    payload.push(0x01_u8); // TAG_HELLO_LOAD
    payload.extend_from_slice(
        &u32::try_from(path.len())
            .expect("BUG: path length fits in u32")
            .to_le_bytes(),
    );
    payload.extend_from_slice(path);

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
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
        )]
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

    let baseline = open_fd_count();
    let err = recv_hello_with_fd(&receiver).expect_err("BUG: too many fds must be rejected");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert_eq!(
        open_fd_count(),
        baseline,
        "recv_hello_with_fd must close any primary fd it received before rejecting extra fds",
    );
}

#[test]
fn hello_with_wrong_version_is_rejected_before_payload() {
    use std::os::fd::AsFd;

    let (a, b) = pair();
    let dummy = UnixStream::pair()
        .expect("BUG: test fixture requires UnixStream::pair to succeed")
        .0;

    bmc_wasm_thin_protocol::test_helpers::send_hello_with_fd_versioned(
        &a,
        0xFFFF,
        bmc_wasm_thin_protocol::test_helpers::TAG_HELLO_LOAD_VALUE,
        b"/tmp/x.wasm",
        dummy.as_fd(),
    )
    .expect("BUG: test fixture expects raw sendmsg to succeed on connected socketpair");

    let err = recv_hello_with_fd(&b).expect_err("must reject bogus version");
    assert!(err.to_string().contains("protocol version"), "got {err}");
}

#[test]
fn ack_decoder_rejects_runaway_input() {
    use bmc_wasm_thin_protocol::{AckDecoder, MAX_STRING_LEN, PROTOCOL_VERSION};

    let mut dec = AckDecoder::new();
    dec.push(&PROTOCOL_VERSION.to_le_bytes())
        .expect("BUG: AckDecoder must accept the protocol version prefix");
    dec.push(&[0x01])
        .expect("BUG: AckDecoder must accept the Err tag byte");
    dec.push(&MAX_STRING_LEN.to_le_bytes())
        .expect("BUG: AckDecoder must accept the length prefix");
    let chunk = vec![0_u8; 8 * 1024];
    let mut last: Result<Option<_>, _> = Ok(None);
    for _ in 0..16 {
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
fn ack_decoder_rejects_unknown_tag_distinctly() {
    use bmc_wasm_thin_protocol::{AckDecoder, PROTOCOL_VERSION};

    let mut dec = AckDecoder::new();
    dec.push(&PROTOCOL_VERSION.to_le_bytes())
        .expect("BUG: AckDecoder must accept the protocol version prefix");
    let err = dec.push(&[0x42]).expect_err("unknown tag must reject");
    assert!(
        format!("{err}").contains("unknown Ack tag")
            || format!("{err}").to_lowercase().contains("unknown"),
        "got {err}"
    );
}
