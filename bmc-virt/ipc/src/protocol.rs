// Copyright (C) 2026  Braiins Systems s.r.o.

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
