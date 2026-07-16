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

// Wire protocol constants.
//
// Frame format:
//   [tag: u8] [length: u32 LE] [payload: length bytes]
//
// The tag identifies the message type. Length is the payload size
// (not including the 5-byte header).

/// Default TCP port the relay listens on inside the guest.
pub const DEFAULT_PORT: u16 = 5_910;

/// Maximum payload size (16 MB — fits any framebuffer).
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

/// Wire header size: 1 (tag) + 4 (length LE).
pub const HEADER_SIZE: usize = 5;

// Guest → host tags
pub const TAG_FRAME: u8 = 0x01;
pub const TAG_LEDS: u8 = 0x02;
pub const TAG_LOG: u8 = 0x03;
pub const TAG_ACTIVE_EFFECT: u8 = 0x04;
pub const TAG_CAPTURE_STATUS: u8 = 0x05;
pub const TAG_VOLUME_LEVEL: u8 = 0x06;
pub const TAG_PONG: u8 = 0x07;
pub const TAG_NOTIFY: u8 = 0x08;
pub const TAG_CONTROLS_STATUS: u8 = 0x09;

// Host → guest tags
pub const TAG_INPUT: u8 = 0x80;
pub const TAG_RUN_COMMAND: u8 = 0x81;
pub const TAG_GPIO_BUTTON: u8 = 0x82;
pub const TAG_PING: u8 = 0x83;
