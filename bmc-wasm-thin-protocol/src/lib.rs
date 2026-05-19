// Copyright (C) 2026  Braiins Systems s.r.o.

//! Hello/Ack wire format for the bmc-wasm-host control socket.

#![deny(unsafe_op_in_unsafe_fn)]

use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

pub const MAX_STRING_LEN: u32 = 64 * 1024;
pub const PROTOCOL_VERSION: u16 = 1;
const TAG_HELLO_LOAD: u8 = 0x01;
const TAG_ACK_OK: u8 = 0x00;
const TAG_ACK_ERR: u8 = 0x01;

#[must_use]
pub fn socket_path_for_sdk_major(major: u16) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/run/bmc/wasm-host-sdk-v{major}.sock"))
}

#[must_use]
pub fn default_socket_path() -> std::path::PathBuf {
    socket_path_for_sdk_major(bmc_wasm_protocol::SDK_VERSION.0)
}

#[must_use]
pub fn default_lockfile_path() -> std::path::PathBuf {
    derive_lockfile_path(&default_socket_path())
}

#[must_use]
pub fn derive_lockfile_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    if socket_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sock"))
    {
        let mut p = socket_path.to_path_buf();
        p.set_extension("lock");
        return p;
    }
    let mut s = socket_path.as_os_str().to_owned();
    s.push(".lock");
    std::path::PathBuf::from(s)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloMsg {
    Load { wasm_path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AckMsg {
    Ok,
    Err(String),
}

fn write_lenstr<W: Write>(w: &mut W, s: &str) -> io::Result<()> {
    let bytes = s.as_bytes();
    if bytes.len() > MAX_STRING_LEN as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "string exceeds 64 KiB",
        ));
    }
    let len = u32::try_from(bytes.len())
        .expect("BUG: length already checked against MAX_STRING_LEN which fits in u32")
        .to_le_bytes();
    w.write_all(&len)?;
    w.write_all(bytes)
}

fn int_cmsg_space() -> usize {
    unsafe {
        libc::CMSG_SPACE(
            u32::try_from(size_of::<libc::c_int>())
                .expect("BUG: sizeof(c_int) fits in u32 on all supported platforms"),
        ) as usize
    }
}

fn int_cmsg_len() -> usize {
    unsafe {
        libc::CMSG_LEN(
            u32::try_from(size_of::<libc::c_int>())
                .expect("BUG: sizeof(c_int) fits in u32 on all supported platforms"),
        ) as usize
    }
}

fn cmsg_hdr_size() -> usize {
    unsafe { libc::CMSG_LEN(0) as usize }
}

fn sendmsg_with_fd(sock: &UnixStream, payload: &[u8], fd: BorrowedFd<'_>) -> io::Result<()> {
    let iov = libc::iovec {
        iov_base: payload.as_ptr().cast_mut().cast::<libc::c_void>(),
        iov_len: payload.len(),
    };

    let fd_int: libc::c_int = fd.as_raw_fd();
    let cmsg_space = int_cmsg_space();
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
        (*cmsg).cmsg_len = int_cmsg_len();
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
        )]
        std::ptr::write(libc::CMSG_DATA(cmsg).cast::<libc::c_int>(), fd_int);
    }

    let n = unsafe { libc::sendmsg(sock.as_raw_fd(), &raw const hdr, libc::MSG_NOSIGNAL) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n.cast_unsigned() != payload.len() {
        return Err(io::Error::new(io::ErrorKind::WriteZero, "partial sendmsg"));
    }
    Ok(())
}

pub fn send_hello_with_fd(
    sock: &UnixStream,
    msg: &HelloMsg,
    wayland_fd: BorrowedFd<'_>,
) -> io::Result<()> {
    let mut payload: Vec<u8> = Vec::with_capacity(2 + 1 + 4 + 256);
    payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    match msg {
        HelloMsg::Load { wasm_path } => {
            payload.push(TAG_HELLO_LOAD);
            write_lenstr(&mut payload, wasm_path)?;
        }
    }
    sendmsg_with_fd(sock, &payload, wayland_fd)
}

#[expect(
    clippy::too_many_lines,
    reason = "cmsg handling cannot be split without introducing unsafety across function boundaries"
)]
pub fn recv_hello_with_fd(sock: &UnixStream) -> io::Result<(HelloMsg, OwnedFd)> {
    // Read the entire fixed-size header (version u16 LE + tag + u32 LE length = 7 bytes) plus
    // the SCM_RIGHTS cmsg in a single recvmsg(2). The cmsg is delivered with the first byte of
    // the call's data; doing this in one syscall keeps the cmsg paired with the message and
    // avoids the SOCK_STREAM hazard of splitting the cmsg-bearing read from later data reads.
    // The variable-length wasm_path payload is then drained with read_exact since no further
    // cmsg is expected.
    let mut header = [0_u8; 7];
    let mut iov = libc::iovec {
        iov_base: header.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: header.len(),
    };
    let cmsg_space = int_cmsg_space();
    let mut cmsg_buf = vec![0_u8; cmsg_space];

    let mut hdr = libc::msghdr {
        msg_name: std::ptr::null_mut(),
        msg_namelen: 0,
        msg_iov: &raw mut iov,
        msg_iovlen: 1,
        msg_control: cmsg_buf.as_mut_ptr().cast::<libc::c_void>(),
        msg_controllen: cmsg_space,
        msg_flags: 0,
    };

    let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &raw mut hdr, libc::MSG_CMSG_CLOEXEC) };
    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed"));
    }
    if n < 7 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short read: header truncated",
        ));
    }
    // Reject a truncated control message before trusting the cmsg payload. We sized
    // `cmsg_buf` for exactly one int, so a sender attempting to attach more than one fd
    // would land here. Treating MSG_CTRUNC as InvalidData also belt-and-suspenders the
    // n_fds extra-fd check below for senders that smuggled a second cmsg in a
    // way the kernel surfaced via MSG_CTRUNC rather than via a tail cmsg.
    if (hdr.msg_flags & libc::MSG_CTRUNC) != 0 {
        // The kernel may have delivered some fds even though the cmsg was truncated.
        // Close every fd visible in the truncated buffer to avoid leaking kernel handles.
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&raw const hdr);
            while !c.is_null() {
                if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                    let hdr_size = cmsg_hdr_size();
                    let total = (*c).cmsg_len;
                    if total > hdr_size {
                        #[expect(
                            clippy::integer_division,
                            reason = "intentional: count whole ints in the data area"
                        )]
                        let n_fds = (total - hdr_size) / size_of::<libc::c_int>();
                        #[expect(
                            clippy::cast_ptr_alignment,
                            reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
                        )]
                        let data_start = libc::CMSG_DATA(c).cast::<libc::c_int>();
                        for i in 0..n_fds {
                            let fd = std::ptr::read(data_start.add(i));
                            let _ = libc::close(fd);
                        }
                    }
                }
                c = libc::CMSG_NXTHDR(&raw const hdr, c);
            }
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SCM_RIGHTS control message truncated",
        ));
    }

    let fd: OwnedFd = unsafe {
        let cmsg: *const libc::cmsghdr = libc::CMSG_FIRSTHDR(&raw const hdr);
        if cmsg.is_null()
            || (*cmsg).cmsg_level != libc::SOL_SOCKET
            || (*cmsg).cmsg_type != libc::SCM_RIGHTS
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing or malformed SCM_RIGHTS",
            ));
        }
        // Close any extra fds before we decide whether to accept this cmsg.
        // On Linux, CMSG_SPACE(1 int) == CMSG_SPACE(2 ints) so the kernel may deliver
        // two fds in one cmsg without triggering MSG_CTRUNC. Compute how many ints the
        // cmsg actually carries and reject anything other than exactly one.
        let hdr_size = cmsg_hdr_size();
        let total = (*cmsg).cmsg_len;
        #[expect(
            clippy::integer_division,
            reason = "intentional: count whole ints in the data area"
        )]
        let n_fds = if total > hdr_size {
            (total - hdr_size) / size_of::<libc::c_int>()
        } else {
            0
        };
        if n_fds != 1 {
            #[expect(
                clippy::cast_ptr_alignment,
                reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
            )]
            let data = libc::CMSG_DATA(cmsg).cast::<libc::c_int>();
            for i in 0..n_fds {
                let _ = libc::close(std::ptr::read(data.add(i)));
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing or malformed SCM_RIGHTS",
            ));
        }
        // Take ownership of the primary fd before inspecting any tail cmsg so that
        // every early-return path below closes the kernel handle via OwnedFd::drop.
        // MSG_CMSG_CLOEXEC only sets the close-on-exec bit; without OwnedFd taking
        // possession here, returning Err in the extra-cmsg branch leaks the fd.
        #[expect(
            clippy::cast_ptr_alignment,
            reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
        )]
        let raw = std::ptr::read(libc::CMSG_DATA(cmsg).cast::<libc::c_int>());
        let primary = OwnedFd::from_raw_fd(raw);
        let next = libc::CMSG_NXTHDR(&raw const hdr, cmsg);
        if !next.is_null() {
            #[expect(
                clippy::cast_ptr_alignment,
                reason = "CMSG_DATA alignment is guaranteed by the kernel ABI"
            )]
            let extra_fd = std::ptr::read(libc::CMSG_DATA(next).cast::<libc::c_int>());
            let _ = libc::close(extra_fd);
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "more than one fd in SCM_RIGHTS",
            ));
        }
        primary
    };

    let mut sock_r = sock;

    let version = u16::from_le_bytes([header[0], header[1]]);
    if version != PROTOCOL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("protocol version mismatch: expected {PROTOCOL_VERSION}, got {version}"),
        ));
    }
    let tag = header[2];
    if tag != TAG_HELLO_LOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unknown Hello tag",
        ));
    }
    let len = u32::from_le_bytes([header[3], header[4], header[5], header[6]]);
    if len > MAX_STRING_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "wasm_path exceeds 64 KiB",
        ));
    }
    let mut buf = vec![0_u8; len as usize];
    sock_r.read_exact(&mut buf)?;
    let wasm_path =
        String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    Ok((HelloMsg::Load { wasm_path }, fd))
}

pub fn write_ack(sock: &UnixStream, msg: &AckMsg) -> io::Result<()> {
    let mut buf: Vec<u8> = Vec::with_capacity(2 + 64);
    buf.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    match msg {
        AckMsg::Ok => buf.push(TAG_ACK_OK),
        AckMsg::Err(m) => {
            buf.push(TAG_ACK_ERR);
            write_lenstr(&mut buf, m)?;
        }
    }
    let mut sock_w = sock;
    sock_w.write_all(&buf)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum AckDecoderState {
    #[default]
    NeedVersion,
    NeedTag,
    NeedErrLen,
    NeedErrBody {
        len: usize,
    },
    Done,
}

#[derive(Debug, Default)]
pub struct AckDecoder {
    buf: Vec<u8>,
    state: AckDecoderState,
}

impl AckDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> io::Result<Option<AckMsg>> {
        const VERSION_BYTES: usize = 2;
        const TAG_BYTES: usize = 1;
        const LEN_BYTES: usize = 4;
        let max_decoder_buf = VERSION_BYTES
            + TAG_BYTES
            + LEN_BYTES
            + usize::try_from(MAX_STRING_LEN).expect("BUG: MAX_STRING_LEN fits in usize");
        self.buf.extend_from_slice(bytes);
        if self.buf.len() > max_decoder_buf {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Ack decoder buffer exceeded cap; peer is dripping garbage",
            ));
        }
        loop {
            match self.state {
                AckDecoderState::Done => {
                    // Any bytes arriving after a complete frame are a protocol error;
                    // the Stage 5 Ack contract is exactly one frame per Hello.
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "trailing bytes after Ack frame",
                    ));
                }
                AckDecoderState::NeedVersion => {
                    if self.buf.len() < 2 {
                        return Ok(None);
                    }
                    let ver = u16::from_le_bytes([self.buf[0], self.buf[1]]);
                    if ver != PROTOCOL_VERSION {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "protocol version mismatch: expected {PROTOCOL_VERSION}, got {ver}"
                            ),
                        ));
                    }
                    self.buf.drain(..2);
                    self.state = AckDecoderState::NeedTag;
                }
                AckDecoderState::NeedTag => {
                    if self.buf.is_empty() {
                        return Ok(None);
                    }
                    let tag = self.buf[0];
                    self.buf.drain(..1);
                    match tag {
                        TAG_ACK_OK => {
                            self.state = AckDecoderState::Done;
                            if !self.buf.is_empty() {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "trailing bytes after Ack frame",
                                ));
                            }
                            return Ok(Some(AckMsg::Ok));
                        }
                        TAG_ACK_ERR => {
                            self.state = AckDecoderState::NeedErrLen;
                        }
                        other => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("unknown Ack tag: {other:#04x}"),
                            ));
                        }
                    }
                }
                AckDecoderState::NeedErrLen => {
                    if self.buf.len() < 4 {
                        return Ok(None);
                    }
                    let len =
                        u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
                    if len > MAX_STRING_LEN {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "string exceeds 64 KiB",
                        ));
                    }
                    self.buf.drain(..4);
                    self.state = AckDecoderState::NeedErrBody { len: len as usize };
                }
                AckDecoderState::NeedErrBody { len } => {
                    if self.buf.len() < len {
                        return Ok(None);
                    }
                    let body: Vec<u8> = self.buf.drain(..len).collect();
                    let s = String::from_utf8(body)
                        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                    self.state = AckDecoderState::Done;
                    if !self.buf.is_empty() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "trailing bytes after Ack frame",
                        ));
                    }
                    return Ok(Some(AckMsg::Err(s)));
                }
            }
        }
    }
}

pub mod test_helpers {
    use std::os::fd::BorrowedFd;
    use std::os::unix::net::UnixStream;

    use super::{TAG_HELLO_LOAD, sendmsg_with_fd};

    pub const TAG_HELLO_LOAD_VALUE: u8 = TAG_HELLO_LOAD;

    pub fn send_hello_with_fd_versioned(
        sock: &UnixStream,
        version: u16,
        tag: u8,
        body: &[u8],
        fd: BorrowedFd<'_>,
    ) -> std::io::Result<()> {
        let mut payload: Vec<u8> = Vec::with_capacity(2 + 1 + 4 + body.len());
        payload.extend_from_slice(&version.to_le_bytes());
        payload.push(tag);
        let len =
            u32::try_from(body.len()).expect("BUG: body length must fit in u32 for test frames");
        payload.extend_from_slice(&len.to_le_bytes());
        payload.extend_from_slice(body);
        sendmsg_with_fd(sock, &payload, fd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_decoder_yields_ok_for_single_byte() {
        let mut d = AckDecoder::new();
        let mut frame = PROTOCOL_VERSION.to_le_bytes().to_vec();
        frame.push(TAG_ACK_OK);
        let got = d.push(&frame).expect("BUG: Ack::Ok is well formed");
        assert_eq!(got, Some(AckMsg::Ok));
    }

    #[test]
    fn ack_decoder_buffers_err_across_multiple_pushes() {
        let mut d = AckDecoder::new();
        assert_eq!(
            d.push(&PROTOCOL_VERSION.to_le_bytes())
                .expect("BUG: partial frame must not error"),
            None
        );
        assert_eq!(
            d.push(&[TAG_ACK_ERR])
                .expect("BUG: partial frame must not error"),
            None
        );
        let len_bytes = 5_u32.to_le_bytes();
        assert_eq!(
            d.push(&len_bytes[..2])
                .expect("BUG: partial length must not error"),
            None
        );
        assert_eq!(
            d.push(&len_bytes[2..])
                .expect("BUG: completed length must not error"),
            None
        );
        assert_eq!(
            d.push(b"he").expect("BUG: partial body must not error"),
            None
        );
        let got = d.push(b"llo").expect("BUG: complete frame must decode");
        assert_eq!(got, Some(AckMsg::Err("hello".into())));
    }

    #[test]
    fn ack_decoder_rejects_unknown_tag() {
        let mut d = AckDecoder::new();
        let mut frame = PROTOCOL_VERSION.to_le_bytes().to_vec();
        frame.push(0xFF);
        let err = d.push(&frame).expect_err("unknown tag must be InvalidData");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn ack_decoder_rejects_trailing_bytes_after_ok() {
        let mut d = AckDecoder::new();
        let mut frame = PROTOCOL_VERSION.to_le_bytes().to_vec();
        frame.extend_from_slice(&[TAG_ACK_OK, 0x00]);
        let err = d
            .push(&frame)
            .expect_err("trailing bytes after Ack::Ok must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn ack_decoder_rejects_trailing_bytes_after_err() {
        let mut d = AckDecoder::new();
        let mut frame = PROTOCOL_VERSION.to_le_bytes().to_vec();
        frame.push(TAG_ACK_ERR);
        frame.extend_from_slice(&2_u32.to_le_bytes());
        frame.extend_from_slice(b"hi");
        frame.push(0xAB);
        let err = d
            .push(&frame)
            .expect_err("trailing bytes after Ack::Err must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn ack_decoder_rejects_oversize_string_length() {
        let mut d = AckDecoder::new();
        let mut frame = PROTOCOL_VERSION.to_le_bytes().to_vec();
        frame.push(TAG_ACK_ERR);
        frame.extend_from_slice(&(MAX_STRING_LEN + 1).to_le_bytes());
        let err = d
            .push(&frame)
            .expect_err("oversize Ack::Err length must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn ack_decoder_rejects_invalid_utf8() {
        let mut d = AckDecoder::new();
        let mut frame = PROTOCOL_VERSION.to_le_bytes().to_vec();
        frame.push(TAG_ACK_ERR);
        frame.extend_from_slice(&2_u32.to_le_bytes());
        frame.extend_from_slice(&[0xFF, 0xFE]);
        let err = d
            .push(&frame)
            .expect_err("invalid UTF-8 in Ack::Err must be rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
