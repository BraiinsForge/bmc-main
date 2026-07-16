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

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub fn create_activation_entrypoint(store_path: &Path) {
    let activation_dir = store_path.join("core/activation");
    std::fs::create_dir_all(&activation_dir).expect("BUG: create activation dir");
    let entrypoint = activation_dir.join("entrypoint");
    std::fs::write(
        &entrypoint,
        r#"#!/bin/sh
set -e
profile_dir="$(dirname "$PROFILE_NEW_GENERATION")"
current_link="$profile_dir/current"
gen_dir_name="$(basename "$PROFILE_NEW_GENERATION")"
tmp_link="$profile_dir/current.tmp"
rm -f "$tmp_link"
ln -s "$gen_dir_name" "$tmp_link"
mv -Tf "$tmp_link" "$current_link"
"#,
    )
    .expect("BUG: write entrypoint");
    std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755))
        .expect("BUG: set permissions");
}

pub fn create_fake_store(base: &Path, files: &[&str]) {
    for file_path in files {
        let full_path = base.join(file_path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("BUG: should create parent dirs");
        }
        std::fs::write(&full_path, format!("content of {file_path}"))
            .expect("BUG: should write fake file");
    }
}
