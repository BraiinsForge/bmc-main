// Copyright (C) 2026  Braiins Systems s.r.o.

// Touch event injection via evdev write to the virtio-tablet device.
// Extracted from bmc-virt-vnc/src/vnc.rs.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::io::AsRawFd;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0x00;
const BTN_TOUCH: u16 = 0x14a;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;

#[repr(C)]
struct EvdevEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

pub struct Touch {
    fd: i32,
    pressed: bool,
}

impl Touch {
    pub fn open(path: &str) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).open(path)?;
        let fd = file.as_raw_fd();
        std::mem::forget(file); // keep fd alive
        eprintln!("touch: opened {path} for event injection");
        Ok(Self { fd, pressed: false })
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
        self.emit(EV_KEY, BTN_TOUCH, 1);
        self.emit(EV_ABS, ABS_X, i32::from(x));
        self.emit(EV_ABS, ABS_Y, i32::from(y));
        self.emit(EV_SYN, SYN_REPORT, 0);
        self.pressed = true;
    }

    /// Inject a touch-move (drag) event at the given (x, y) coordinates.
    pub fn move_to(&mut self, x: u16, y: u16) {
        if self.pressed {
            self.emit(EV_ABS, ABS_X, i32::from(x));
            self.emit(EV_ABS, ABS_Y, i32::from(y));
            self.emit(EV_SYN, SYN_REPORT, 0);
        }
    }

    /// Inject a touch-up event.
    pub fn up(&mut self) {
        if self.pressed {
            self.emit(EV_KEY, BTN_TOUCH, 0);
            self.emit(EV_SYN, SYN_REPORT, 0);
            self.pressed = false;
        }
    }
}
