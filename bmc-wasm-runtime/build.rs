// Copyright (C) 2026  Braiins Systems s.r.o.

//! Build script — compiles built-in SVG icons into binary blobs.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use bmc_wasm_protocol::{ICON_CLOSE, ICON_ERROR, ICON_INFO, ICON_SUCCESS, ICON_WARNING};

/// Maps SVG file stem → builtin icon ID from the protocol crate.
const BUILTIN_ICON_MAP: &[(&str, u16)] = &[
    ("close", ICON_CLOSE),
    ("error--solid", ICON_ERROR),
    ("warning--solid", ICON_WARNING),
    ("checkmark--solid", ICON_SUCCESS),
    ("info--solid", ICON_INFO),
];

fn main() {
    let icons_dir = Path::new("icons");
    let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| panic!("OUT_DIR not set"));
    let out_path = Path::new(&out_dir);

    // Collect (name, id) for each icon
    let mut entries = Vec::new();

    for entry in fs::read_dir(icons_dir).unwrap_or_else(|e| panic!("failed to read icons/: {e}")) {
        let entry = entry.unwrap_or_else(|e| panic!("failed to iterate icon entries: {e}"));
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "svg") {
            let stem = path
                .file_stem()
                .unwrap_or_else(|| panic!("no file stem for {}", path.display()))
                .to_str()
                .unwrap_or_else(|| panic!("non-UTF8 file stem for {}", path.display()))
                .to_owned();
            let svg = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

            let compiled = bmc_icon_compiler::compile_svg(&svg);

            // Write binary blob
            let bin_name = format!("icon_{stem}.bin");
            fs::write(out_path.join(&bin_name), &compiled)
                .unwrap_or_else(|e| panic!("failed to write {bin_name}: {e}"));

            // Look up ID from the protocol-defined mapping
            let const_id = BUILTIN_ICON_MAP
                .iter()
                .find(|(name, _)| *name == stem)
                .unwrap_or_else(|| {
                    panic!("unknown built-in icon: {stem} — add its ID to protocol/src/icon.rs and build.rs BUILTIN_ICON_MAP")
                })
                .1;

            entries.push((stem, const_id));

            // Tell Cargo to rerun if this SVG changes
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    // Generate Rust source with the icon data table
    let mut generated = String::from(
        "/// Built-in icon data compiled from SVGs at build time.\n\
         pub const BUILTIN_ICON_DATA: &[(u16, &[u8])] = &[\n",
    );
    for (stem, id) in &entries {
        writeln!(
            generated,
            "    (0x{id:04X}, include_bytes!(concat!(env!(\"OUT_DIR\"), \"/icon_{stem}.bin\"))),"
        )
        .unwrap_or_else(|e| panic!("failed to write generated source: {e}"));
    }
    generated.push_str("];\n");

    fs::write(out_path.join("builtin_icons.rs"), generated)
        .unwrap_or_else(|e| panic!("failed to write builtin_icons.rs: {e}"));
}
