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

// APA102 frame decoder.
// Accumulates bytes from multiple write() syscalls and emits decoded LED frames.

pub const LED_COUNT: usize = 10;

#[derive(Debug, Clone, Copy, Default)]
pub struct Led {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub brightness: u8,
}

impl Led {
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.brightness == 0 || (self.r == 0 && self.g == 0 && self.b == 0)
    }
}

#[derive(Debug)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: Vec::with_capacity(64),
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> Option<[Led; LED_COUNT]> {
        self.buf.extend_from_slice(data);

        let frame_size = 4 + LED_COUNT * 4 + 4 + 1;
        if let Some(start) = self.find_start_frame()
            && self.buf.len() >= start + frame_size
        {
            let frame = self.decode_frame(start);
            self.buf.drain(..start + frame_size);
            return frame;
        }

        if self.buf.len() > frame_size * 4 {
            let keep = self.buf.len() - frame_size * 2;
            self.buf.drain(..keep);
        }

        None
    }

    fn find_start_frame(&self) -> Option<usize> {
        (0..self.buf.len().saturating_sub(4)).find(|&i| {
            self.buf[i] == 0
                && self.buf[i + 1] == 0
                && self.buf[i + 2] == 0
                && self.buf[i + 3] == 0
                && self.buf.get(i + 4).is_some_and(|b| b & 0xE0 == 0xE0)
        })
    }

    fn decode_frame(&self, start: usize) -> Option<[Led; LED_COUNT]> {
        let mut leds = [Led::default(); LED_COUNT];
        let led_data_start = start + 4;

        for (i, led) in leds.iter_mut().enumerate() {
            let offset = led_data_start + i * 4;
            if offset + 3 >= self.buf.len() {
                return None;
            }

            let header = self.buf[offset];
            if header & 0xE0 != 0xE0 {
                return None;
            }

            *led = Led {
                brightness: header & 0x1F,
                b: self.buf[offset + 1],
                g: self.buf[offset + 2],
                r: self.buf[offset + 3],
            };
        }

        Some(leds)
    }
}
