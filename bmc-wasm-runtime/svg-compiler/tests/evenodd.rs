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
const FLAG_EVENODD: u8 = 0x04;

/// Extract the per-path flags byte from the first path in the compiled binary.
fn extract_flags(bin: &[u8]) -> u8 {
    // Header: viewbox_w(f32) + viewbox_h(f32) + path_count(u16) = 10 bytes
    // First byte after header is the flags byte of the first path.
    assert!(bin.len() > 10, "binary too short");
    bin[10]
}

#[test]
fn fillrule_on_svg_element_is_inherited() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="currentColor" fill-rule="evenodd">
    <path d="M8 16A8 8 0 1 1 8 0a8 8 0 0 1 0 16M3 5l8 8 2-2-8-8z"/>
</svg>"#;

    let bin = compile_svg(svg);
    let flags = extract_flags(&bin);

    assert_ne!(flags & FLAG_HAS_FILL, 0, "expected fill flag");
    assert_ne!(
        flags & FLAG_EVENODD,
        0,
        "expected EVENODD flag for fill-rule inherited from <svg>"
    );
}

#[test]
fn fillrule_on_path_element_works() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
    <path fill-rule="evenodd" d="M8 16A8 8 0 1 1 8 0a8 8 0 0 1 0 16M3 5l8 8 2-2-8-8z"/>
</svg>"#;

    let bin = compile_svg(svg);
    let flags = extract_flags(&bin);

    assert_ne!(flags & FLAG_HAS_FILL, 0, "expected fill flag");
    assert_ne!(
        flags & FLAG_EVENODD,
        0,
        "expected EVENODD flag for fill-rule on <path>"
    );
}

#[test]
fn default_nonzero_has_no_evenodd_flag() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
    <path d="M8 16A8 8 0 1 1 8 0a8 8 0 0 1 0 16M3 5l8 8 2-2-8-8z"/>
</svg>"#;

    let bin = compile_svg(svg);
    let flags = extract_flags(&bin);

    assert_ne!(flags & FLAG_HAS_FILL, 0, "expected fill flag");
    assert_eq!(
        flags & FLAG_EVENODD,
        0,
        "did NOT expect EVENODD flag for default nonzero fill rule"
    );
}
