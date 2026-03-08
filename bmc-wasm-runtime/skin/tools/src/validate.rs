// Copyright (C) 2026  Braiins Systems s.r.o.

//! Validate a skin directory or zip archive.
//!
//! Checks that `skin.toml` has required fields, `preview.png` exists,
//! and all `.9.png` files have valid stretch markers.

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "skin-validate", about = "Validate a WASM widget skin")]
struct Args {
    /// Path to a skin directory or zip file
    path: PathBuf,
}

fn main() {
    let args = Args::parse();
    let path = &args.path;

    if !path.exists() {
        eprintln!("error: path does not exist: {}", path.display());
        std::process::exit(1);
    }

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if path.is_dir() {
        validate_dir(path, &mut errors, &mut warnings);
    } else {
        validate_zip(path, &mut errors, &mut warnings);
    }

    for w in &warnings {
        eprintln!("warning: {w}");
    }
    for e in &errors {
        eprintln!("error: {e}");
    }

    if errors.is_empty() {
        let w = if warnings.is_empty() {
            String::new()
        } else {
            format!(" ({} warning(s))", warnings.len())
        };
        eprintln!("ok: skin is valid{w}");
    } else {
        eprintln!("\n{} error(s), {} warning(s)", errors.len(), warnings.len());
        std::process::exit(1);
    }
}

fn validate_dir(dir: &std::path::Path, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    // Check skin.toml
    let toml_path = dir.join("skin.toml");
    if !toml_path.exists() {
        errors.push("skin.toml not found".into());
        return;
    }
    let toml_str = std::fs::read_to_string(&toml_path).unwrap_or_else(|e| {
        errors.push(format!("failed to read skin.toml: {e}"));
        String::new()
    });

    // Collect asset names from .9.png files (strip the .9.png suffix)
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push(format!("failed to read directory: {e}"));
            return;
        }
    };
    let mut asset_names: Vec<String> = Vec::new();
    let mut nine_patches: Vec<(std::path::PathBuf, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".9.png") {
            asset_names.push(
                name.strip_suffix(".9.png")
                    .expect("BUG: checked above")
                    .to_string(),
            );
            nine_patches.push((entry.path(), name));
        }
    }

    if !toml_str.is_empty() {
        validate_toml(&toml_str, errors, warnings, &asset_names);
    }

    // Check preview.png
    if !dir.join("preview.png").exists() {
        warnings.push("preview.png not found — skin picker will show no thumbnail".into());
    }

    // Validate .9.png files
    for (path, name) in &nine_patches {
        validate_nine_patch(path, name, errors);
    }
}

fn validate_zip(path: &std::path::Path, errors: &mut Vec<String>, warnings: &mut Vec<String>) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            errors.push(format!("failed to read zip: {e}"));
            return;
        }
    };
    let mut archive = match zip::ZipArchive::new(std::io::Cursor::new(&data)) {
        Ok(a) => a,
        Err(e) => {
            errors.push(format!("failed to open zip: {e}"));
            return;
        }
    };

    // Collect asset names from .9.png entries
    let mut asset_names: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        if let Some(stem) = name.strip_suffix(".9.png") {
            asset_names.push(stem.to_string());
        }
    }

    // Check skin.toml
    match archive.by_name("skin.toml") {
        Ok(mut entry) => {
            let mut toml_str = String::new();
            if std::io::Read::read_to_string(&mut entry, &mut toml_str).is_ok() {
                validate_toml(&toml_str, errors, warnings, &asset_names);
            } else {
                errors.push("failed to read skin.toml from zip".into());
            }
        }
        Err(_) => errors.push("skin.toml not found in zip".into()),
    }

    // Check preview.png
    if archive.by_name("preview.png").is_err() {
        warnings.push("preview.png not found — skin picker will show no thumbnail".into());
    }

    // Validate .9.png files
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        if !name.ends_with(".9.png") {
            continue;
        }
        let mut data = Vec::new();
        if std::io::Read::read_to_end(&mut entry, &mut data).is_err() {
            errors.push(format!("{name}: failed to read"));
            continue;
        }
        validate_nine_patch_bytes(&data, &name, errors);
    }
}

/// Known top-level keys in skin.toml.
const KNOWN_TOP_LEVEL_KEYS: &[&str] = &["name", "description", "palette"];

/// Known keys inside the [palette] section.
const KNOWN_PALETTE_KEYS: &[&str] = &[
    "accent",
    "background",
    "layer1",
    "layer2",
    "text_primary",
    "text_secondary",
];

/// Known keys inside per-asset sections (e.g. [button_normal]).
const KNOWN_ASSET_KEYS: &[&str] = &["color"];

fn validate_toml(
    toml_str: &str,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
    known_assets: &[String],
) {
    let table: toml::Table = match toml_str.parse() {
        Ok(t) => t,
        Err(e) => {
            errors.push(format!("skin.toml parse error: {e}"));
            return;
        }
    };

    if !table.get("name").is_some_and(|v| v.is_str()) {
        errors.push("skin.toml: missing required `name` string field".into());
    }
    if !table.get("description").is_some_and(|v| v.is_str()) {
        errors.push("skin.toml: missing required `description` string field".into());
    }

    // Check for unknown top-level keys
    for key in table.keys() {
        if KNOWN_TOP_LEVEL_KEYS.contains(&key.as_str()) {
            continue;
        }
        // Must be an asset section (table with known asset keys)
        if let Some(section) = table.get(key).and_then(|v| v.as_table()) {
            if !known_assets.contains(key) {
                warnings.push(format!(
                    "skin.toml: unknown section `[{key}]` — not matching any asset file"
                ));
            }
            for sub_key in section.keys() {
                if !KNOWN_ASSET_KEYS.contains(&sub_key.as_str()) {
                    warnings.push(format!("skin.toml: unknown key `{sub_key}` in [{key}]"));
                }
            }
        } else {
            warnings.push(format!("skin.toml: unknown top-level key `{key}`"));
        }
    }

    // Validate [palette] keys
    if let Some(palette) = table.get("palette").and_then(|v| v.as_table()) {
        for key in palette.keys() {
            if !KNOWN_PALETTE_KEYS.contains(&key.as_str()) {
                warnings.push(format!("skin.toml: unknown palette key `{key}`"));
            }
        }
    }
}

fn validate_nine_patch(path: &std::path::Path, name: &str, errors: &mut Vec<String>) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            errors.push(format!("{name}: failed to read: {e}"));
            return;
        }
    };
    validate_nine_patch_bytes(&data, name, errors);
}

fn validate_nine_patch_bytes(data: &[u8], name: &str, errors: &mut Vec<String>) {
    let img = match image::load_from_memory(data) {
        Ok(i) => i,
        Err(e) => {
            errors.push(format!("{name}: failed to decode: {e}"));
            return;
        }
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    if let Err(e) = bmc_wasm_skin::try_parse_nine_patch_insets(w, h, |x, y| rgba.get_pixel(x, y).0)
    {
        errors.push(format!("{name}: {e}"));
    }
}
