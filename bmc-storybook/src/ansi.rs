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

// ANSI SGR escape sequence parser → egui RichText spans.
// Uses `anstyle-parse` for the state machine, extracts color/bold attributes.

use anstyle_parse::{Params, Parser, Perform};

/// A styled text segment extracted from an ANSI-encoded string.
pub struct Span {
    pub text: String,
    pub color: Option<egui::Color32>,
    pub bold: bool,
}

/// Parse an ANSI-encoded string into a sequence of styled spans.
pub fn parse(input: &str) -> Vec<Span> {
    let mut handler = AnsiHandler::new();
    let mut parser = Parser::<anstyle_parse::Utf8Parser>::new();
    for byte in input.bytes() {
        parser.advance(&mut handler, byte);
    }
    handler.finish()
}

struct AnsiHandler {
    spans: Vec<Span>,
    current_text: String,
    current_color: Option<egui::Color32>,
    current_bold: bool,
}

impl AnsiHandler {
    fn new() -> Self {
        Self {
            spans: Vec::new(),
            current_text: String::new(),
            current_color: None,
            current_bold: false,
        }
    }

    fn flush_text(&mut self) {
        if !self.current_text.is_empty() {
            self.spans.push(Span {
                text: std::mem::take(&mut self.current_text),
                color: self.current_color,
                bold: self.current_bold,
            });
        }
    }

    fn finish(mut self) -> Vec<Span> {
        self.flush_text();
        self.spans
    }

    fn apply_sgr(&mut self, params: &Params) {
        self.flush_text();

        let mut iter = params.iter();
        while let Some(param) = iter.next() {
            let code = param.first().copied().unwrap_or(0);
            match code {
                0 => {
                    // Reset
                    self.current_color = None;
                    self.current_bold = false;
                }
                1 => self.current_bold = true,
                22 => self.current_bold = false,
                30 => self.current_color = Some(egui::Color32::from_gray(40)),
                31 => self.current_color = Some(egui::Color32::from_rgb(255, 85, 85)),
                32 => self.current_color = Some(egui::Color32::from_rgb(85, 255, 85)),
                33 => self.current_color = Some(egui::Color32::from_rgb(255, 255, 85)),
                34 => self.current_color = Some(egui::Color32::from_rgb(85, 85, 255)),
                35 => self.current_color = Some(egui::Color32::from_rgb(255, 85, 255)),
                36 => self.current_color = Some(egui::Color32::from_rgb(85, 255, 255)),
                37 => self.current_color = Some(egui::Color32::from_gray(220)),
                38 => {
                    // Extended color: 38;5;N (256-color) or 38;2;R;G;B (truecolor)
                    if let Some(mode) = iter.next() {
                        match mode.first().copied().unwrap_or(0) {
                            5 => {
                                // 256-color — map to approximate RGB
                                if let Some(idx) = iter.next() {
                                    self.current_color =
                                        Some(color_256(idx.first().copied().unwrap_or(0)));
                                }
                            }
                            2 => {
                                // Truecolor RGB
                                let r_p = iter.next();
                                let g_p = iter.next();
                                let b_p = iter.next();
                                if let (Some(r_p), Some(g_p), Some(b_p)) = (r_p, g_p, b_p) {
                                    #[expect(
                                        clippy::cast_possible_truncation,
                                        reason = "RGB values are 0-255"
                                    )]
                                    {
                                        self.current_color = Some(egui::Color32::from_rgb(
                                            r_p.first().copied().unwrap_or(0) as u8,
                                            g_p.first().copied().unwrap_or(0) as u8,
                                            b_p.first().copied().unwrap_or(0) as u8,
                                        ));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                39 => self.current_color = None, // default fg
                90..=97 => {
                    // Bright colors
                    self.current_color = Some(match code {
                        90 => egui::Color32::from_gray(100),
                        91 => egui::Color32::from_rgb(255, 120, 120),
                        92 => egui::Color32::from_rgb(120, 255, 120),
                        93 => egui::Color32::from_rgb(255, 255, 120),
                        94 => egui::Color32::from_rgb(120, 120, 255),
                        95 => egui::Color32::from_rgb(255, 120, 255),
                        96 => egui::Color32::from_rgb(120, 255, 255),
                        97 => egui::Color32::WHITE,
                        _ => unreachable!(),
                    });
                }
                _ => {} // ignore background, underline, etc.
            }
        }
    }
}

impl Perform for AnsiHandler {
    fn print(&mut self, c: char) {
        self.current_text.push(c);
    }

    fn execute(&mut self, _byte: u8) {
        // Control chars (newlines, etc.) — ignore, lines are already split
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: u8) {
        if action == b'm' {
            self.apply_sgr(params);
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: u8) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {}
}

/// Map a 256-color index to an approximate RGB color.
#[expect(clippy::cast_possible_truncation, reason = "color math on 0-255 range")]
fn color_256(idx: u16) -> egui::Color32 {
    match idx {
        0..=7 => {
            // Standard colors
            let colors: [egui::Color32; 8] = [
                egui::Color32::from_gray(0),
                egui::Color32::from_rgb(170, 0, 0),
                egui::Color32::from_rgb(0, 170, 0),
                egui::Color32::from_rgb(170, 170, 0),
                egui::Color32::from_rgb(0, 0, 170),
                egui::Color32::from_rgb(170, 0, 170),
                egui::Color32::from_rgb(0, 170, 170),
                egui::Color32::from_gray(170),
            ];
            colors[idx as usize]
        }
        8..=15 => {
            let colors: [egui::Color32; 8] = [
                egui::Color32::from_gray(85),
                egui::Color32::from_rgb(255, 85, 85),
                egui::Color32::from_rgb(85, 255, 85),
                egui::Color32::from_rgb(255, 255, 85),
                egui::Color32::from_rgb(85, 85, 255),
                egui::Color32::from_rgb(255, 85, 255),
                egui::Color32::from_rgb(85, 255, 255),
                egui::Color32::WHITE,
            ];
            colors[(idx - 8) as usize]
        }
        16..=231 => {
            // 6x6x6 color cube — integer division is the correct decomposition here
            #[expect(clippy::integer_division, reason = "color cube index decomposition")]
            let (ri, gi, bi) = {
                let idx = idx - 16;
                (idx / 36, (idx % 36) / 6, idx % 6)
            };
            let to_byte = |v: u16| if v == 0 { 0_u8 } else { (55 + 40 * v) as u8 };
            egui::Color32::from_rgb(to_byte(ri), to_byte(gi), to_byte(bi))
        }
        232..=255 => {
            // Grayscale ramp
            let gray = (8 + 10 * (idx - 232)) as u8;
            egui::Color32::from_gray(gray)
        }
        _ => egui::Color32::from_gray(180),
    }
}
