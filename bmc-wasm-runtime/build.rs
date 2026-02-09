// Copyright (C) 2026  Braiins Systems s.r.o.

//! Build script — compiles built-in SVG icons into binary blobs.

use std::fmt::Write;
use std::fs;
use std::path::Path;

fn main() {
    let icons_dir = Path::new("icons");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);

    // Collect (name, id, binary) for each icon
    let mut entries = Vec::new();

    for entry in fs::read_dir(icons_dir).expect("failed to read icons/ directory") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "svg") {
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            let svg = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

            let compiled = bmc_icon_compiler::compile_svg(&svg);

            // Write binary blob
            let bin_name = format!("icon_{stem}.bin");
            fs::write(out_path.join(&bin_name), &compiled).unwrap();

            // Map name to builtin constant
            let const_id = match stem.as_str() {
                "close" => 0xFF01_u16,
                other => panic!("unknown built-in icon: {other} — add its ID to build.rs"),
            };

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
        .unwrap();
    }
    generated.push_str("];\n");

    fs::write(out_path.join("builtin_icons.rs"), generated).unwrap();
}
