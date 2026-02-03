// Copyright (C) 2025  Braiins Systems s.r.o.

//! Node type and draw command constants for tree serialization.

// Node types
pub const NODE_COLUMN: u8 = 0;
pub const NODE_ROW: u8 = 1;
pub const NODE_CENTER: u8 = 2;
pub const NODE_PARAGRAPH: u8 = 3;
pub const NODE_BUTTON: u8 = 4;
pub const NODE_SPACER: u8 = 5;
pub const NODE_CANVAS: u8 = 6;

// Draw commands
pub const DRAW_RECT: u8 = 16;
pub const DRAW_CENTERED: u8 = 17;
pub const DRAW_ORBIT: u8 = 18;
pub const DRAW_ROTATED: u8 = 19;
