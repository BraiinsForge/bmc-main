// Copyright (C) 2026  Braiins Systems s.r.o.

//! Node type and draw command constants for tree serialization.

// Node types (0x00–0x3F)
pub const NODE_COLUMN: u8 = 0x00;
pub const NODE_ROW: u8 = 0x01;
pub const NODE_CENTER: u8 = 0x02;
pub const NODE_PARAGRAPH: u8 = 0x03;
pub const NODE_BUTTON: u8 = 0x04;
pub const NODE_SPACER: u8 = 0x05;
pub const NODE_CANVAS: u8 = 0x06;
pub const NODE_MODAL: u8 = 0x07;
pub const NODE_NOTIFICATION: u8 = 0x08;
pub const NODE_SCROLL: u8 = 0x09;
pub const NODE_PROGRESS_BAR: u8 = 0x0A;

// Button size variants (wire value for NODE_BUTTON)
pub const BUTTON_SIZE_SMALL: u8 = 0;
pub const BUTTON_SIZE_NORMAL: u8 = 1;
pub const BUTTON_SIZE_LARGE: u8 = 2;

// Draw commands — shapes (0x40–0x5F)
pub const DRAW_RECT: u8 = 0x40;
pub const DRAW_CIRCLE: u8 = 0x41;
pub const DRAW_ICON: u8 = 0x42;
pub const DRAW_BITMAP: u8 = 0x43;
pub const DRAW_PATH: u8 = 0x44;
pub const DRAW_SPHERE: u8 = 0x45;
pub const DRAW_TEXT: u8 = 0x46;
pub const DRAW_MESH: u8 = 0x47;
pub const DRAW_NINE_PATCH: u8 = 0x48;

// Draw commands — transforms (0x60–0x7F)
pub const DRAW_CENTERED: u8 = 0x60;
pub const DRAW_ORBIT: u8 = 0x61;
pub const DRAW_ROTATED: u8 = 0x62;

// Draw commands — modifiers (0x80–0x9F)
pub const DRAW_MODIFIED: u8 = 0x80;
