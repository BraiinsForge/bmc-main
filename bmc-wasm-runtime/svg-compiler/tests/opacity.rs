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

use bmc_svg_compiler::compile_svg;

const FLAG_HAS_FILL: u8 = 0x01;

/// The alpha byte of the first path's fill color.
fn first_fill_alpha(bin: &[u8]) -> u8 {
    // Header: viewbox_w(f32) + viewbox_h(f32) + path_count(u16) = 10 bytes,
    // then the flags byte, then the RGBA fill stored little-endian.
    assert!(bin.len() > 14, "binary too short");
    assert_ne!(bin[10] & FLAG_HAS_FILL, 0, "expected fill flag");
    bin[11]
}

fn icon(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">{body}</svg>"#
    )
}

#[test]
fn element_opacity_scales_the_fill_alpha() {
    let bin = compile_svg(&icon(
        r#"<path opacity="0.4" d="M0 0h16v16H0z" fill="white"/>"#,
    ));
    assert_eq!(first_fill_alpha(&bin), 102, "0.4 of 255, truncated");
}

#[test]
fn nested_group_opacities_multiply() {
    let bin = compile_svg(&icon(
        r#"<g opacity="0.5"><g opacity="0.5"><path d="M0 0h16v16H0z" fill="white"/></g></g>"#,
    ));
    assert_eq!(first_fill_alpha(&bin), 63, "0.25 of 255, truncated");
}

#[test]
fn element_and_fill_opacity_multiply() {
    let bin = compile_svg(&icon(
        r#"<path opacity="0.5" fill-opacity="0.5" d="M0 0h16v16H0z" fill="white"/>"#,
    ));
    assert_eq!(first_fill_alpha(&bin), 63, "0.25 of 255, truncated");
}

#[test]
fn an_opaque_path_keeps_full_alpha() {
    let bin = compile_svg(&icon(r#"<path d="M0 0h16v16H0z" fill="white"/>"#));
    assert_eq!(first_fill_alpha(&bin), 255);
}
