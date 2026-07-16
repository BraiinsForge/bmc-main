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

//! Wayland frame capture via ext-image-copy-capture-v1.
//!
//! Connects to the compositor as a Wayland client, creates a capture session
//! on the first output, and provides a blocking `capture_frame()` that copies
//! the compositor's display into a shared-memory buffer.

use std::os::fd::AsFd;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;

use wayland_client::protocol::{wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_image_capture_source_v1, ext_output_image_capture_source_manager_v1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_frame_v1,
    ext_image_copy_capture_manager_v1::{self, Options},
    ext_image_copy_capture_session_v1,
};

/// Pixel format: 4 bytes per pixel (XRGB8888).
const BPP: u32 = 4;

pub struct WaylandCapture {
    queue: EventQueue<CaptureState>,
    state: CaptureState,
}

struct CaptureState {
    // Globals
    output: Option<wl_output::WlOutput>,
    shm: Option<wl_shm::WlShm>,
    capture_source_mgr:
        Option<ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1>,
    capture_mgr: Option<ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1>,

    // Session
    session: Option<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1>,
    source: Option<ext_image_capture_source_v1::ExtImageCaptureSourceV1>,

    // Buffer constraints (received from compositor)
    buffer_width: u32,
    buffer_height: u32,
    shm_format: wl_shm::Format,

    // Shared memory buffer
    shm_pool: Option<wl_shm_pool::WlShmPool>,
    shm_fd: Option<OwnedFd>,
    shm_ptr: *mut u8,
    shm_len: usize,
    buffer: Option<wayland_client::protocol::wl_buffer::WlBuffer>,

    // Frame state
    constraints_ready: bool,
    frame_ready: bool,
    frame_failed: bool,
}

// Safety: shm_ptr points to a private mmap; only accessed from the capture thread.
unsafe impl Send for CaptureState {}

impl WaylandCapture {
    /// Connect to the compositor and set up a capture session.
    /// Blocks until the session is established and buffer constraints are received.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "buffer pool size fits in i32 for any reasonable display"
    )]
    pub fn connect(socket_path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(socket_path)
            .map_err(|e| format!("wayland connect {}: {e}", socket_path.display()))?;
        let conn = Connection::from_socket(stream)
            .map_err(|e| format!("wayland connect {}: {e}", socket_path.display()))?;
        let display = conn.display();

        let mut queue = conn.new_event_queue();
        let qh = queue.handle();

        let mut state = CaptureState {
            output: None,
            shm: None,
            capture_source_mgr: None,
            capture_mgr: None,
            session: None,
            source: None,
            buffer_width: 0,
            buffer_height: 0,
            shm_format: wl_shm::Format::Xrgb8888,
            shm_pool: None,
            shm_fd: None,
            shm_ptr: std::ptr::null_mut(),
            shm_len: 0,
            buffer: None,
            constraints_ready: false,
            frame_ready: false,
            frame_failed: false,
        };

        // Trigger global enumeration
        display.get_registry(&qh, ());

        // Roundtrip to receive globals
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("roundtrip (globals): {e}"))?;

        let output = state.output.clone().ok_or("no wl_output found")?;
        let shm = state.shm.clone().ok_or("no wl_shm found")?;
        let source_mgr = state
            .capture_source_mgr
            .clone()
            .ok_or("no ext_output_image_capture_source_manager_v1 found")?;
        let capture_mgr = state
            .capture_mgr
            .clone()
            .ok_or("no ext_image_copy_capture_manager_v1 found")?;

        // Create capture source from output
        let source = source_mgr.create_source(&output, &qh, ());
        state.source = Some(source.clone());

        // Create capture session (no cursor compositing)
        let session = capture_mgr.create_session(&source, Options::empty(), &qh, ());
        state.session = Some(session);

        // Roundtrip to receive buffer constraints
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("roundtrip (constraints): {e}"))?;

        if !state.constraints_ready {
            return Err("did not receive buffer constraints from compositor".into());
        }

        eprintln!(
            "capture: session ready, {}x{} {:?}",
            state.buffer_width, state.buffer_height, state.shm_format
        );

        // Allocate shared memory buffer
        let stride = state.buffer_width * BPP;
        let size = (stride * state.buffer_height) as usize;

        let fd = create_shm_fd(size)?;
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_fd().as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(format!("mmap failed: {}", std::io::Error::last_os_error()));
        }

        let pool = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            state.buffer_width as i32,
            state.buffer_height as i32,
            stride as i32,
            state.shm_format,
            &qh,
            (),
        );

        state.shm_pool = Some(pool);
        state.shm_fd = Some(fd);
        state.shm_ptr = ptr.cast();
        state.shm_len = size;
        state.buffer = Some(buffer);

        Ok(Self { queue, state })
    }

    /// Capture a single frame. Blocks until the compositor delivers it.
    /// Returns a reference to the pixel data in the shared memory buffer.
    #[expect(clippy::cast_possible_wrap, reason = "buffer dimensions fit in i32")]
    pub fn capture_frame(&mut self) -> Result<&[u8], String> {
        let qh = self.queue.handle();
        let session = self
            .state
            .session
            .as_ref()
            .ok_or("no capture session")?
            .clone();
        let buffer = self
            .state
            .buffer
            .as_ref()
            .ok_or("no capture buffer")?
            .clone();

        self.state.frame_ready = false;
        self.state.frame_failed = false;

        // Create a frame, attach buffer, request capture
        let frame = session.create_frame(&qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(
            0,
            0,
            self.state.buffer_width as i32,
            self.state.buffer_height as i32,
        );
        frame.capture();

        // Block until frame is ready or failed
        while !self.state.frame_ready && !self.state.frame_failed {
            self.queue
                .blocking_dispatch(&mut self.state)
                .map_err(|e| format!("dispatch: {e}"))?;
        }

        if self.state.frame_failed {
            return Err("frame capture failed".into());
        }

        Ok(unsafe { std::slice::from_raw_parts(self.state.shm_ptr, self.state.shm_len) })
    }

    pub fn width(&self) -> u32 {
        self.state.buffer_width
    }

    pub fn height(&self) -> u32 {
        self.state.buffer_height
    }

    pub fn stride(&self) -> u32 {
        self.state.buffer_width * BPP
    }
}

impl Drop for CaptureState {
    fn drop(&mut self) {
        if !self.shm_ptr.is_null() {
            unsafe {
                libc::munmap(self.shm_ptr.cast(), self.shm_len);
            }
        }
    }
}

// ── Wayland dispatch implementations ────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for CaptureState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_output" => {
                    if state.output.is_none() {
                        state.output =
                            Some(registry.bind::<wl_output::WlOutput, _, _>(name, version, qh, ()));
                    }
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, version, qh, ()));
                }
                "ext_output_image_capture_source_manager_v1" => {
                    state.capture_source_mgr = Some(registry.bind(name, version, qh, ()));
                }
                "ext_image_copy_capture_manager_v1" => {
                    state.capture_mgr = Some(registry.bind(name, version, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1, ()>
    for CaptureState
{
    fn event(
        state: &mut Self,
        _proxy: &ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.buffer_width = width;
                state.buffer_height = height;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: wayland_client::WEnum::Value(fmt),
            } => {
                state.shm_format = fmt;
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                state.constraints_ready = true;
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                eprintln!("capture: session stopped by compositor");
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { .. }
            | ext_image_copy_capture_session_v1::Event::DmabufDevice { .. }
            | ext_image_copy_capture_session_v1::Event::DmabufFormat { .. }
            | _ => {}
        }
    }
}

impl Dispatch<ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => {
                state.frame_ready = true;
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                eprintln!("capture: frame failed: {reason:?}");
                state.frame_failed = true;
            }
            ext_image_copy_capture_frame_v1::Event::Transform { .. }
            | ext_image_copy_capture_frame_v1::Event::Damage { .. }
            | ext_image_copy_capture_frame_v1::Event::PresentationTime { .. }
            | _ => {}
        }
    }
}

// Globals whose events we don't need — `ignore` avoids panicking on incoming events.
delegate_noop!(CaptureState: ignore wl_output::WlOutput);
delegate_noop!(CaptureState: ignore wl_shm::WlShm);
delegate_noop!(CaptureState: ignore wl_shm_pool::WlShmPool);
delegate_noop!(CaptureState: ignore wayland_client::protocol::wl_buffer::WlBuffer);
delegate_noop!(CaptureState: ignore ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(CaptureState: ignore ext_image_capture_source_v1::ExtImageCaptureSourceV1);
delegate_noop!(CaptureState: ignore ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1);

// ── Helpers ─────────────────────────────────────────────────────────────

/// Create an anonymous shared memory fd for the wl_shm pool.
#[expect(clippy::cast_possible_wrap, reason = "shm size fits in off_t")]
fn create_shm_fd(size: usize) -> Result<OwnedFd, String> {
    let name = std::ffi::CString::new("/bmc-virt-capture").expect("BUG: CString");

    // Try to create; if it already exists from a crashed previous run, unlink and retry.
    let fd = unsafe {
        libc::shm_open(
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )
    };
    let fd = if fd < 0 {
        unsafe { libc::shm_unlink(name.as_ptr()) };
        let fd = unsafe {
            libc::shm_open(
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )
        };
        if fd < 0 {
            return Err(format!("shm_open: {}", std::io::Error::last_os_error()));
        }
        fd
    } else {
        fd
    };

    // Unlink immediately so the name is freed even if we crash
    unsafe { libc::shm_unlink(name.as_ptr()) };

    if unsafe { libc::ftruncate(fd, size as libc::off_t) } < 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(format!("ftruncate: {err}"));
    }

    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}
