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

//! Hello/Ack wire format for the bmc-wasm-host control socket.

#![deny(unsafe_op_in_unsafe_fn)]

use std::io::{self, Write};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

pub const PROTOCOL_VERSION: u16 = 3;
const FRAME_HEADER_LEN: usize = 6; // u16 version + u32 frame_len, both little-endian
pub const MAX_FRAME_LEN: u32 = 128 * 1024; // upper bound that bounds peer memory commitment

#[must_use]
pub fn socket_path_for_sdk_major(major: u16) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("/run/bmc/wasm-host-sdk-v{major}.sock"))
}

#[must_use]
pub fn default_socket_path() -> std::path::PathBuf {
    socket_path_for_sdk_major(bmc_wasm_protocol::SDK_VERSION.0)
}

/// Persistent root for per-instance widget asset buckets (`<root>/<bucket>`).
pub const WIDGET_CACHE_DIR: &str = "/mnt/data/bmc/widget-cache";

/// Per-bucket byte cap for the generic blob cache (content-agnostic).
pub const WIDGET_CACHE_BUCKET_MAX_BYTES: u64 = 16 * 1_024 * 1_024;

#[must_use]
pub fn default_lockfile_path() -> std::path::PathBuf {
    derive_lockfile_path(&default_socket_path())
}

#[must_use]
pub fn default_owner_record_path() -> std::path::PathBuf {
    derive_owner_record_path(&default_socket_path())
}

#[must_use]
pub fn derive_lockfile_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    derive_socket_sibling_path(socket_path, "lock")
}

#[must_use]
pub fn derive_owner_record_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    derive_socket_sibling_path(socket_path, "owner")
}

fn derive_socket_sibling_path(
    socket_path: &std::path::Path,
    extension: &str,
) -> std::path::PathBuf {
    if socket_path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("sock"))
    {
        let mut p = socket_path.to_path_buf();
        p.set_extension(extension);
        return p;
    }
    let mut s = socket_path.as_os_str().to_owned();
    s.push(".");
    s.push(extension);
    std::path::PathBuf::from(s)
}

/// Log file path for the host serving `socket_path`.
///
/// One live host per socket is guaranteed by the lockfile, so deriving
/// the log path from the socket keeps every log file single-writer.
/// The full path is flattened into the file name (not just the stem) so
/// distinct sockets with equal file names get distinct logs.
#[must_use]
pub fn derive_log_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    let flat = socket_path
        .with_extension("")
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => part.to_str(),
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir => None,
        })
        .collect::<Vec<_>>()
        .join("-");
    if flat.is_empty() {
        return std::path::PathBuf::from("/var/log/bmc/bmc-wasm-host.log");
    }
    std::path::PathBuf::from(format!("/var/log/bmc/{flat}.log"))
}

/// Derive the path to the device-wide image decode lock for the host
/// serving `socket_path`.
///
/// The file name is fixed within the socket's directory, so every host
/// there — including different SDK majors running through an upgrade —
/// serializes decodes on the same lock, while a custom socket directory
/// (tests, sandboxes) gets its own writable, hermetic lock.
#[must_use]
pub fn derive_image_decode_lock_path(socket_path: &std::path::Path) -> std::path::PathBuf {
    socket_path.with_file_name("image-decode.lock")
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub enum HelloMsg {
    Load {
        widget_key: String,
        wasm_path: String,
        asset_root: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, bincode::Encode, bincode::Decode)]
pub enum AckMsg {
    Ok,
    Err(String),
}

#[must_use]
fn bincode_config() -> impl bincode::config::Config {
    bincode::config::standard()
}

fn build_frame<E: bincode::Encode>(payload: &E) -> io::Result<Vec<u8>> {
    let body = bincode::encode_to_vec(payload, bincode_config())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let frame_len = u32::try_from(body.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "encoded payload exceeds u32"))?;
    if frame_len > MAX_FRAME_LEN {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "encoded payload exceeds MAX_FRAME_LEN",
        ));
    }
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + body.len());
    frame.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    frame.extend_from_slice(&frame_len.to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn sendmsg_with_fd(sock: &UnixStream, payload: &[u8], fd: BorrowedFd<'_>) -> io::Result<()> {
    use nix::sys::socket::{ControlMessage, MsgFlags, sendmsg};
    use std::io::IoSlice;

    let iov = [IoSlice::new(payload)];
    let fd_raw: std::os::fd::RawFd = fd.as_raw_fd();
    let cmsgs = [ControlMessage::ScmRights(std::slice::from_ref(&fd_raw))];

    let n = sendmsg::<()>(sock.as_raw_fd(), &iov, &cmsgs, MsgFlags::MSG_NOSIGNAL, None)
        .map_err(io::Error::from)?;

    if n != payload.len() {
        return Err(io::Error::new(io::ErrorKind::WriteZero, "partial sendmsg"));
    }
    Ok(())
}

pub fn send_hello_with_fd(
    sock: &UnixStream,
    msg: &HelloMsg,
    wayland_fd: BorrowedFd<'_>,
) -> io::Result<()> {
    let frame = build_frame(msg)?;
    sendmsg_with_fd(sock, &frame, wayland_fd)
}

#[derive(Debug)]
pub enum HelloReceiveStatus {
    Pending,
    Complete(HelloMsg, OwnedFd),
}

#[derive(Debug, Default)]
pub struct HelloReceiver {
    frame: Vec<u8>,
    expected: Option<usize>,
    wayland_fd: Option<OwnedFd>,
}

impl HelloReceiver {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_recv(&mut self, sock: &UnixStream) -> io::Result<HelloReceiveStatus> {
        self.recv_with_flags(sock, nix::sys::socket::MsgFlags::MSG_DONTWAIT)
    }

    fn recv_blocking(&mut self, sock: &UnixStream) -> io::Result<HelloReceiveStatus> {
        self.recv_with_flags(sock, nix::sys::socket::MsgFlags::empty())
    }

    fn recv_with_flags(
        &mut self,
        sock: &UnixStream,
        flags: nix::sys::socket::MsgFlags,
    ) -> io::Result<HelloReceiveStatus> {
        let result = self.recv_inner(sock, flags);
        if result.is_err() {
            self.reset();
        }
        result
    }

    fn recv_inner(
        &mut self,
        sock: &UnixStream,
        flags: nix::sys::socket::MsgFlags,
    ) -> io::Result<HelloReceiveStatus> {
        loop {
            if let Some(complete) = self.decode_complete()? {
                return Ok(complete);
            }

            let target = self.expected.unwrap_or(FRAME_HEADER_LEN);
            let remaining = target
                .checked_sub(self.frame.len())
                .expect("BUG: complete frames are decoded before receiving more bytes");
            if remaining == 0 {
                self.decode_header()?;
                continue;
            }

            match self.recv_chunk(sock, flags, remaining) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(HelloReceiveStatus::Pending);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn recv_chunk(
        &mut self,
        sock: &UnixStream,
        flags: nix::sys::socket::MsgFlags,
        remaining: usize,
    ) -> io::Result<()> {
        use nix::cmsg_space;
        use nix::sys::socket::{ControlMessageOwned, MsgFlags, recvmsg};
        use std::io::IoSliceMut;

        let mut bytes = vec![0_u8; remaining];
        let (received_bytes, received_flags, mut received) = {
            let mut iov = [IoSliceMut::new(&mut bytes)];
            let mut cmsg_buf = cmsg_space!([std::os::fd::RawFd; 1]);
            let msg = recvmsg::<()>(
                sock.as_raw_fd(),
                &mut iov,
                Some(&mut cmsg_buf),
                flags | MsgFlags::MSG_CMSG_CLOEXEC,
            )
            .map_err(io::Error::from)?;

            let mut received: Vec<OwnedFd> = Vec::new();
            for cmsg in msg.cmsgs().map_err(io::Error::from)? {
                #[expect(
                    clippy::wildcard_enum_match_arm,
                    reason = "any non-ScmRights cmsg on this socket is a protocol violation"
                )]
                match cmsg {
                    ControlMessageOwned::ScmRights(fds) => {
                        for raw in fds {
                            // SAFETY: `raw` is a fresh kernel-owned fd delivered via SCM_RIGHTS;
                            // nix has already validated the cmsg shape. Wrapping immediately
                            // means subsequent early-return paths close it via OwnedFd::drop.
                            let owned = unsafe { OwnedFd::from_raw_fd(raw) };
                            received.push(owned);
                        }
                    }
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected control message",
                        ));
                    }
                }
            }
            (msg.bytes, msg.flags, received)
        };

        if received_flags.contains(MsgFlags::MSG_CTRUNC) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SCM_RIGHTS control message truncated",
            ));
        }

        if received_bytes == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "peer closed"));
        }

        match (self.wayland_fd.is_some(), received.len()) {
            (false, 1) => {
                self.wayland_fd = received.pop();
            }
            (false, 0) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing or malformed SCM_RIGHTS",
                ));
            }
            (true, 0) => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "more than one fd in SCM_RIGHTS",
                ));
            }
        }

        self.frame.extend_from_slice(&bytes[..received_bytes]);
        Ok(())
    }

    fn decode_header(&mut self) -> io::Result<()> {
        let version = u16::from_le_bytes([self.frame[0], self.frame[1]]);
        if version != PROTOCOL_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("protocol version mismatch: expected {PROTOCOL_VERSION}, got {version}"),
            ));
        }
        let frame_len =
            u32::from_le_bytes([self.frame[2], self.frame[3], self.frame[4], self.frame[5]]);
        if frame_len > MAX_FRAME_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame_len exceeds MAX_FRAME_LEN",
            ));
        }
        self.expected = Some(FRAME_HEADER_LEN + frame_len as usize);
        Ok(())
    }

    fn decode_complete(&mut self) -> io::Result<Option<HelloReceiveStatus>> {
        let Some(expected) = self.expected else {
            return Ok(None);
        };
        if self.frame.len() < expected {
            return Ok(None);
        }

        let body = &self.frame[FRAME_HEADER_LEN..expected];
        let (msg, _consumed) = bincode::decode_from_slice::<HelloMsg, _>(body, bincode_config())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let fd = self
            .wayland_fd
            .take()
            .expect("BUG: a complete Hello frame retains its SCM_RIGHTS fd");
        self.frame.clear();
        self.expected = None;
        Ok(Some(HelloReceiveStatus::Complete(msg, fd)))
    }

    fn reset(&mut self) {
        self.frame.clear();
        self.expected = None;
        self.wayland_fd = None;
    }
}

pub fn recv_hello_with_fd(sock: &UnixStream) -> io::Result<(HelloMsg, OwnedFd)> {
    let mut receiver = HelloReceiver::new();
    match receiver.recv_blocking(sock)? {
        HelloReceiveStatus::Complete(msg, fd) => Ok((msg, fd)),
        HelloReceiveStatus::Pending => Err(io::Error::from(io::ErrorKind::WouldBlock)),
    }
}

pub fn write_ack(sock: &UnixStream, msg: &AckMsg) -> io::Result<()> {
    let frame = build_frame(msg)?;
    let mut sock_w: &UnixStream = sock;
    sock_w.write_all(&frame)
}

#[derive(Debug, Default)]
pub struct AckDecoder {
    buf: Vec<u8>,
    expected: Option<usize>,
}

impl AckDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> io::Result<Option<AckMsg>> {
        if self.buf.len() + bytes.len()
            > FRAME_HEADER_LEN + usize::try_from(MAX_FRAME_LEN).expect("BUG: fits usize")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AckDecoder buffer would exceed MAX_FRAME_LEN",
            ));
        }
        self.buf.extend_from_slice(bytes);

        if self.expected.is_none() && self.buf.len() >= FRAME_HEADER_LEN {
            let version = u16::from_le_bytes([self.buf[0], self.buf[1]]);
            if version != PROTOCOL_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "protocol version mismatch: expected {PROTOCOL_VERSION}, got {version}"
                    ),
                ));
            }
            let frame_len =
                u32::from_le_bytes([self.buf[2], self.buf[3], self.buf[4], self.buf[5]]);
            if frame_len > MAX_FRAME_LEN {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame_len exceeds MAX_FRAME_LEN",
                ));
            }
            self.expected = Some(FRAME_HEADER_LEN + frame_len as usize);
        }

        if let Some(total) = self.expected
            && self.buf.len() >= total
        {
            let body = &self.buf[FRAME_HEADER_LEN..total];
            let (msg, _) = bincode::decode_from_slice::<AckMsg, _>(body, bincode_config())
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            return Ok(Some(msg));
        }

        Ok(None)
    }
}

#[cfg(test)]
mod log_path_tests {
    use std::path::Path;

    use super::derive_log_path;

    #[test]
    fn derives_log_path_from_flattened_socket_path() {
        assert_eq!(
            derive_log_path(Path::new("/run/bmc/wasm-host-sdk-v1.sock")),
            Path::new("/var/log/bmc/run-bmc-wasm-host-sdk-v1.log")
        );
    }

    #[test]
    fn distinct_sockets_with_equal_file_names_get_distinct_logs() {
        assert_ne!(
            derive_log_path(Path::new("/tmp/a/host.sock")),
            derive_log_path(Path::new("/tmp/b/host.sock"))
        );
    }

    #[test]
    fn falls_back_when_socket_path_has_no_components() {
        assert_eq!(
            derive_log_path(Path::new("/")),
            Path::new("/var/log/bmc/bmc-wasm-host.log")
        );
    }
}

#[cfg(test)]
mod image_decode_lock_path_tests {
    use std::path::Path;

    use super::derive_image_decode_lock_path;

    #[test]
    fn hosts_sharing_a_socket_directory_share_the_lock() {
        assert_eq!(
            derive_image_decode_lock_path(Path::new("/run/bmc/wasm-host-sdk-v1.sock")),
            derive_image_decode_lock_path(Path::new("/run/bmc/wasm-host-sdk-v2.sock")),
        );
        assert_eq!(
            derive_image_decode_lock_path(Path::new("/run/bmc/wasm-host-sdk-v1.sock")),
            Path::new("/run/bmc/image-decode.lock")
        );
    }

    #[test]
    fn custom_socket_directories_get_a_hermetic_lock() {
        assert_eq!(
            derive_image_decode_lock_path(Path::new("/tmp/e2e/host.sock")),
            Path::new("/tmp/e2e/image-decode.lock")
        );
    }
}
