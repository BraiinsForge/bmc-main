// Copyright (C) 2025  Braiins Systems s.r.o.

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
