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

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::io::AsRawFd;

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_seat, wl_shm, wl_shm_pool, wl_surface, wl_touch,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

struct State {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    seat: Option<wl_seat::WlSeat>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    touch: Option<wl_touch::WlTouch>,
    configured: bool,
    should_exit: bool,
}

impl State {
    fn new() -> Self {
        Self {
            compositor: None,
            shm: None,
            layer_shell: None,
            seat: None,
            surface: None,
            layer_surface: None,
            touch: None,
            configured: false,
            should_exit: false,
        }
    }
}

fn create_shm_buffer(
    shm: &wl_shm::WlShm,
    width: u32,
    height: u32,
    qh: &QueueHandle<State>,
) -> (wl_buffer::WlBuffer, OwnedFd) {
    let stride = width * 4;
    let size = (stride * height) as usize;

    let fd = unsafe {
        let name = c"layer-shell-test-client";
        let raw = libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC);
        assert!(raw >= 0, "BUG: memfd_create failed");
        OwnedFd::from_raw_fd(raw)
    };

    let ftrunc_size = libc::off_t::try_from(size).expect("BUG: buffer size exceeds off_t range");
    let ret = unsafe { libc::ftruncate(fd.as_raw_fd(), ftrunc_size) };
    assert!(ret == 0, "BUG: ftruncate failed");

    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    assert!(
        !ptr.is_null() && ptr != libc::MAP_FAILED,
        "BUG: mmap failed"
    );

    let pixels =
        unsafe { std::slice::from_raw_parts_mut(ptr.cast::<u32>(), (width * height) as usize) };
    for p in pixels.iter_mut() {
        *p = 0x8000_8000_u32;
    }

    let ret = unsafe { libc::munmap(ptr, size) };
    assert!(ret == 0, "BUG: munmap failed");

    let pool_size = i32::try_from(size).expect("BUG: pool size exceeds i32 range");
    let pool = shm.create_pool(fd.as_fd(), pool_size, qh, ());
    let buffer = pool.create_buffer(
        0,
        width.cast_signed(),
        height.cast_signed(),
        stride.cast_signed(),
        wl_shm::Format::Argb8888,
        qh,
        (),
    );
    pool.destroy();

    (buffer, fd)
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_seat" => {
                    let seat: wl_seat::WlSeat = registry.bind(name, version.min(7), qh, ());
                    state.seat = Some(seat);
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm::WlShm,
        _event: wl_shm::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_shm_pool::WlShmPool,
        _event: wl_shm_pool::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_buffer::WlBuffer,
        _event: wl_buffer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _event: zwlr_layer_shell_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                if state.configured {
                    layer_surface.ack_configure(serial);
                    return;
                }

                let (w, h) = (
                    if width == 0 { 480 } else { width },
                    if height == 0 { 480 } else { height },
                );

                eprintln!("layer-shell-test-client: configure {w}x{h}");
                layer_surface.ack_configure(serial);

                let shm = state.shm.as_ref().expect("BUG: wl_shm not bound");
                let surface = state.surface.as_ref().expect("BUG: wl_surface not created");

                let (buffer, _shm_fd) = create_shm_buffer(shm, w, h, qh);
                surface.attach(Some(&buffer), 0, 0);
                surface.damage(0, 0, w.cast_signed(), h.cast_signed());
                surface.commit();

                state.configured = true;
                eprintln!("layer-shell-test-client: painted {w}x{h} 50%-alpha green overlay");
            }
            zwlr_layer_surface_v1::Event::Closed => {
                eprintln!("layer-shell-test-client: closed");
                state.should_exit = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        _seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let caps = match capabilities {
                WEnum::Value(c) => c,
                WEnum::Unknown(_) => return,
            };
            let has_touch = caps.contains(wl_seat::Capability::Touch);
            if has_touch && state.touch.is_none() {
                let seat = state.seat.as_ref().expect("BUG: seat not stored");
                state.touch = Some(seat.get_touch(qh, ()));
                eprintln!("layer-shell-test-client: wl_touch acquired");
            }
        }
    }
}

impl Dispatch<wl_touch::WlTouch, ()> for State {
    fn event(
        _state: &mut Self,
        _touch: &wl_touch::WlTouch,
        event: wl_touch::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_touch::Event::Down { x, y, id, .. } = event {
            eprintln!("layer-shell-test-client: touch down id={id} x={x} y={y}");
        }
    }
}

fn main() {
    let conn = Connection::connect_to_env().expect("BUG: failed to connect to Wayland display");
    let display = conn.display();

    let mut event_queue = conn.new_event_queue::<State>();
    let qh = event_queue.handle();

    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue
        .roundtrip(&mut state)
        .expect("BUG: initial roundtrip failed");

    let Some(compositor) = state.compositor.as_ref() else {
        eprintln!("layer-shell-test-client: wl_compositor global not available");
        std::process::exit(1);
    };
    let Some(layer_shell) = state.layer_shell.as_ref() else {
        eprintln!("layer-shell-test-client: zwlr_layer_shell_v1 global not available");
        std::process::exit(1);
    };

    let surface = compositor.create_surface(&qh, ());

    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        None,
        zwlr_layer_shell_v1::Layer::Overlay,
        "layer-shell-test".to_owned(),
        &qh,
        (),
    );

    let anchor = zwlr_layer_surface_v1::Anchor::Top
        | zwlr_layer_surface_v1::Anchor::Bottom
        | zwlr_layer_surface_v1::Anchor::Left
        | zwlr_layer_surface_v1::Anchor::Right;
    layer_surface.set_anchor(anchor);
    layer_surface.set_size(0, 0);
    layer_surface.set_exclusive_zone(-1);

    state.surface = Some(surface.clone());
    state.layer_surface = Some(layer_surface);

    surface.commit();
    conn.flush().expect("BUG: flush failed");

    eprintln!("layer-shell-test-client: waiting for configure…");

    loop {
        event_queue
            .blocking_dispatch(&mut state)
            .expect("BUG: dispatch failed");
        if state.should_exit {
            break;
        }
    }
}
