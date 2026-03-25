// Copyright (C) 2026  Braiins Systems s.r.o.

//! Build script — compiles built-in SVG icons into binary blobs.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use bmc_wasm_protocol::{
    ICON_CLOSE, ICON_DEV_CAMERA, ICON_DEV_CURSOR, ICON_DEV_DOWNLOAD, ICON_DEV_SCROLL,
    ICON_DEV_UNLINK, ICON_DEV_UPLOAD, ICON_ERROR, ICON_INFO, ICON_METER, ICON_MINUS, ICON_PLUS,
    ICON_SUCCESS, ICON_WARN_ALT, ICON_WARN_FILLED, ICON_WARNING,
};

/// Maps SVG file stem → builtin icon ID from the protocol crate.
const BUILTIN_ICON_MAP: &[(&str, bmc_wasm_protocol::IconId)] = &[
    ("close", ICON_CLOSE),
    ("error--solid", ICON_ERROR),
    ("warning--solid", ICON_WARNING),
    ("checkmark--solid", ICON_SUCCESS),
    ("info--solid", ICON_INFO),
    ("meter", ICON_METER),
    ("minus", ICON_MINUS),
    ("plus", ICON_PLUS),
    ("warn--alt-filled", ICON_WARN_ALT),
    ("warn--filled", ICON_WARN_FILLED),
    // Dev / testbed icons
    ("camera", ICON_DEV_CAMERA),
    ("cursor", ICON_DEV_CURSOR),
    ("scroll", ICON_DEV_SCROLL),
    ("download", ICON_DEV_DOWNLOAD),
    ("upload", ICON_DEV_UPLOAD),
    ("unlink", ICON_DEV_UNLINK),
];

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap_or_else(|_| panic!("OUT_DIR not set"));
    let out_path = Path::new(&out_dir);

    // Scan both icons/ and icons/dev/ for SVG files
    let icon_dirs: &[&Path] = &[Path::new("assets/icons"), Path::new("assets/icons/dev")];

    // Collect (name, id) for each icon
    let mut entries = Vec::new();

    for icons_dir in icon_dirs {
        let Ok(dir) = fs::read_dir(icons_dir) else {
            continue;
        };
        for entry in dir {
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
    }

    // Generate Rust source with the icon data table
    let mut generated = String::from(
        "use bmc_wasm_protocol::IconId;\n\
         /// Built-in icon data compiled from SVGs at build time.\n\
         pub const BUILTIN_ICON_DATA: &[(IconId, &[u8])] = &[\n",
    );
    for (stem, id) in &entries {
        let raw = id.raw();
        writeln!(
            generated,
            "    (IconId::from_raw(0x{raw:04X}), include_bytes!(concat!(env!(\"OUT_DIR\"), \"/icon_{stem}.bin\"))),"
        )
        .unwrap_or_else(|e| panic!("failed to write generated source: {e}"));
    }
    generated.push_str("];\n");

    fs::write(out_path.join("builtin_icons.rs"), generated)
        .unwrap_or_else(|e| panic!("failed to write builtin_icons.rs: {e}"));
}
