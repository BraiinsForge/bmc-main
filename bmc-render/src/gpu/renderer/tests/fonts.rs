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

use std::path::Path;
use std::sync::Arc;

use cosmic_text::fontdb;

use crate::gpu::renderer::fonts::FONTS_DIR;
use crate::gpu::renderer::{FACES, Fonts, build_font_system};

/// fontdb must read each face straight from its mapping,
/// and femtovg must hold that same mapping rather than a copy of its bytes.
#[test]
fn fontdb_and_femtovg_share_one_mapping_per_face() {
    let fonts = Fonts::load().expect("BUG: the font directory carries every face");
    let font_system = build_font_system(&fonts);

    let faces: Vec<_> = font_system.db().faces().collect();
    assert_eq!(faces.len(), fonts.maps.len());
    for (face, map) in faces.iter().zip(&fonts.maps) {
        let (source, _) = font_system
            .db()
            .face_source(face.id)
            .expect("BUG: the face was just loaded");
        let fontdb::Source::SharedFile(_, data) = source else {
            panic!("{} is not read from its mapping", face.post_script_name);
        };
        assert_eq!(
            (*data).as_ref().as_ptr(),
            map.as_ptr(),
            "{} reads other bytes than its mapping",
            face.post_script_name,
        );
    }

    let before: Vec<_> = fonts.maps.iter().map(Arc::strong_count).collect();
    let text_context = femtovg::TextContext::default();
    let _ids = fonts
        .register_femtovg(&text_context)
        .expect("BUG: font registration failed");
    let after: Vec<_> = fonts.maps.iter().map(Arc::strong_count).collect();
    let expected: Vec<_> = before.iter().map(|count| count + 1).collect();
    assert_eq!(after, expected, "femtovg must hold each mapping once");
}

#[test]
fn a_missing_face_names_its_file() {
    let dir = tempfile::tempdir().expect("BUG: the test needs a scratch directory");

    let Err(err) = Fonts::load_from(dir.path()) else {
        panic!("an empty directory loaded as a font set");
    };

    let missing = dir.path().join(FACES[0].file);
    let message = format!("{err:#}");
    assert!(
        message.contains(&missing.display().to_string()),
        "{message}"
    );
}

/// A file that maps fine but holds another face
/// would pair every glyph with the wrong femtovg font.
#[test]
fn a_file_holding_another_face_names_itself() {
    let dir = tempfile::tempdir().expect("BUG: the test needs a scratch directory");
    let fonts_dir = Path::new(FONTS_DIR);
    let [first, second, ..] = &FACES;
    std::fs::copy(fonts_dir.join(second.file), dir.path().join(first.file))
        .expect("BUG: the font directory carries every face");

    let Err(err) = Fonts::load_from(dir.path()) else {
        panic!("a file holding another face loaded as the face it is named for");
    };

    let swapped = dir.path().join(first.file);
    let message = format!("{err:#}");
    assert!(
        message.contains(&swapped.display().to_string()),
        "{message}"
    );
}
