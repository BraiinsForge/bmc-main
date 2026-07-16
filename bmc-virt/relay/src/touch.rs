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

// Touch event injection via evdev write to the virtio-tablet device.
// Extracted from bmc-virt-vnc/src/vnc.rs.

use std::fs;
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use bmc_virt_ipc::{FB_HEIGHT, FB_WIDTH};

pub use bmc_platform::linux_input::discover_touch_node;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const EVIOC_ABS_BASE: libc::Ioctl = 0x40;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct InputAbsInfo {
    value: i32,
    minimum: i32,
    maximum: i32,
    fuzz: i32,
    flat: i32,
    resolution: i32,
}

#[repr(C)]
struct EvdevEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

#[derive(Clone, Copy, Debug)]
struct AxisRange {
    min: i32,
    max: i32,
}

pub struct Touch {
    fd: i32,
    pressed: bool,
    x_range: AxisRange,
    y_range: AxisRange,
    // Toggle the LSB on each touch-down so identical consecutive taps do not
    // get collapsed by the guest kernel's ABS deduplication.
    nudge_parity: bool,
}

const fn ev_ior(nr: libc::Ioctl, size: libc::Ioctl) -> libc::Ioctl {
    (2 << 30) | (size << 16) | ((b'E' as libc::Ioctl) << 8) | nr
}

const fn eviocgabs(axis: u16) -> libc::Ioctl {
    ev_ior(
        EVIOC_ABS_BASE + axis as libc::Ioctl,
        std::mem::size_of::<InputAbsInfo>() as libc::Ioctl,
    )
}

fn query_axis_range(fd: i32, axis: u16) -> io::Result<AxisRange> {
    let mut abs = InputAbsInfo {
        value: 0,
        minimum: 0,
        maximum: 0,
        fuzz: 0,
        flat: 0,
        resolution: 0,
    };

    let ret = unsafe { libc::ioctl(fd, eviocgabs(axis), &mut abs) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(AxisRange {
        min: abs.minimum,
        max: abs.maximum,
    })
}

#[expect(
    clippy::integer_division,
    reason = "evdev ABS coordinates are integer-valued and guest-pixel precision is sufficient"
)]
fn scale_to_raw(coord: u16, logical_extent: u32, range: AxisRange) -> i32 {
    let logical_span = i64::from(logical_extent.saturating_sub(1));
    if logical_span == 0 {
        return range.min;
    }

    let raw_span = i64::from(range.max) - i64::from(range.min);
    let scaled = i64::from(range.min) + (i64::from(coord) * raw_span) / logical_span;
    i32::try_from(scaled).expect("BUG: scaled evdev coordinate must fit i32")
}

impl Touch {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = fs::OpenOptions::new().write(true).open(path)?;
        let fd = file.as_raw_fd();
        std::mem::forget(file); // keep fd alive
        eprintln!("touch: opened {} for event injection", path.display());

        let fallback_range = AxisRange {
            min: 0,
            max: 32_767,
        };

        let x_range = query_axis_range(fd, ABS_X).unwrap_or_else(|err| {
            eprintln!("touch: failed to query ABS_X range: {err}; falling back to 0..32767");
            fallback_range
        });
        let y_range = query_axis_range(fd, ABS_Y).unwrap_or_else(|err| {
            eprintln!("touch: failed to query ABS_Y range: {err}; falling back to 0..32767");
            fallback_range
        });

        eprintln!(
            "touch: ABS ranges X=[{}, {}] Y=[{}, {}]",
            x_range.min, x_range.max, y_range.min, y_range.max
        );

        Ok(Self {
            fd,
            pressed: false,
            x_range,
            y_range,
            nudge_parity: false,
        })
    }

    fn emit(&self, type_: u16, code: u16, value: i32) {
        let ev = EvdevEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        let ret = unsafe {
            libc::write(
                self.fd,
                std::ptr::from_ref(&ev).cast(),
                size_of::<EvdevEvent>(),
            )
        };
        if ret < 0 {
            eprintln!(
                "evdev write failed: {} (type={type_} code={code} value={value})",
                io::Error::last_os_error()
            );
        }
    }

    /// Inject a touch-down event at the given (x, y) coordinates.
    pub fn down(&mut self, x: u16, y: u16) {
        let raw_x = scale_to_raw(x, FB_HEIGHT, self.x_range);
        let raw_y = scale_to_raw(y, FB_WIDTH, self.y_range);
        let nudge = i32::from(self.nudge_parity);
        self.nudge_parity = !self.nudge_parity;
        self.emit(EV_KEY, BTN_TOUCH, 1);
        self.emit(EV_ABS, ABS_X, raw_x ^ nudge);
        self.emit(EV_ABS, ABS_Y, raw_y ^ nudge);
        self.emit(EV_SYN, SYN_REPORT, 0);
        self.pressed = true;
    }

    /// Inject a touch-move (drag) event at the given (x, y) coordinates.
    pub fn move_to(&mut self, x: u16, y: u16) {
        if !self.pressed {
            return;
        }
        let raw_x = scale_to_raw(x, FB_HEIGHT, self.x_range);
        let raw_y = scale_to_raw(y, FB_WIDTH, self.y_range);
        self.emit(EV_ABS, ABS_X, raw_x);
        self.emit(EV_ABS, ABS_Y, raw_y);
        self.emit(EV_SYN, SYN_REPORT, 0);
    }

    /// Inject a touch-up event.
    pub fn up(&mut self) {
        if !self.pressed {
            return;
        }
        self.emit(EV_KEY, BTN_TOUCH, 0);
        self.emit(EV_SYN, SYN_REPORT, 0);
        self.pressed = false;
    }
}
