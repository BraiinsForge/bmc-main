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

/// Read `path_count` out of the 10-byte header (format documented in `lib.rs`).
fn path_count(bin: &[u8]) -> u16 {
    assert!(bin.len() >= 10, "binary too short");
    u16::from_le_bytes([bin[8], bin[9]])
}

#[test]
fn gradient_painted_path_does_not_drop_its_siblings() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="30" height="10" viewBox="0 0 30 10" fill="none">
    <defs>
        <linearGradient id="g" x1="10" y1="5" x2="20" y2="5" gradientUnits="userSpaceOnUse">
            <stop stop-color="#000000"/>
            <stop offset="1" stop-color="#ffffff"/>
        </linearGradient>
    </defs>
    <path fill="#ff0000" d="M0 0h10v10H0z"/>
    <path stroke="url(#g)" d="M10 5H20"/>
    <path fill="#0000ff" d="M20 0h10v10H20z"/>
</svg>"##;

    let bin = compile_svg(svg);

    assert_eq!(
        path_count(&bin),
        2,
        "gradient paint is unsupported and skipped, but the paths around it must survive"
    );
}
