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

//! Convert a Winamp `.wsz` skin to a skin zip for `include_skin!()`.
//!
//! Usage:
//!     cargo run -- input.wsz -o output.zip
//!
//! Extracts the beveled button frame from `CBUTTONS.BMP`, clears the center
//! symbol, and produces generic `button_normal.9.png` / `button_pressed.9.png`
//! assets. The widget then overlays its own SVG icons on top.
//!
//! Also extracts slider assets from `POSBAR.BMP`:
//! - `slider_track.9.png` — seek bar track, horizontally stretchable
//! - `slider_thumb.png` / `slider_thumb_pressed.png` — thumb sprites

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::path::PathBuf;

use image::{ImageReader, Rgba, RgbaImage};

/// Bevel width in pixels on each side of a Winamp transport button.
const BEVEL: u32 = 2;

/// Extract the beveled button frame, clear the center, and produce a 9-patch.
///
/// The bevel corners/edges are preserved. The center is filled with the
/// sampled background color. The 9-patch stretch markers cover only the
/// center (non-bevel) region so corners stay crisp when resized.
fn make_button_nine_patch(sprite: &RgbaImage) -> RgbaImage {
    let (w, h) = (sprite.width(), sprite.height());

    let mut img = sprite.clone();

    // Sample background color from a pixel in the flat center area
    let bg = *sprite.get_pixel(w / 2, h / 2);

    // Clear the center (everything inside the bevel) with the background color
    for y in BEVEL..h - BEVEL {
        for x in BEVEL..w - BEVEL {
            img.put_pixel(x, y, bg);
        }
    }

    // Smooth bevel edges: ensure the stretchable zone of each bevel row/column
    // is uniform. Without this, transition pixels between the two bevel layers
    // fall inside the stretchable zone and become very visible when stretched
    // (e.g. a single non-highlight pixel becomes a ~15px block on a wide button).
    for y in 0..BEVEL {
        let sample = *img.get_pixel(w / 2, y);
        for x in BEVEL..w - BEVEL {
            img.put_pixel(x, y, sample);
        }
    }
    for y in h - BEVEL..h {
        let sample = *img.get_pixel(w / 2, y);
        for x in BEVEL..w - BEVEL {
            img.put_pixel(x, y, sample);
        }
    }
    for x in 0..BEVEL {
        let sample = *img.get_pixel(x, h / 2);
        for y in BEVEL..h - BEVEL {
            img.put_pixel(x, y, sample);
        }
    }
    for x in w - BEVEL..w {
        let sample = *img.get_pixel(x, h / 2);
        for y in BEVEL..h - BEVEL {
            img.put_pixel(x, y, sample);
        }
    }

    // Build the .9.png with 1px border
    let mut nine = RgbaImage::new(w + 2, h + 2);
    let black = Rgba([0, 0, 0, 255]);

    // Paste content at (1,1)
    for y in 0..h {
        for x in 0..w {
            nine.put_pixel(x + 1, y + 1, *img.get_pixel(x, y));
        }
    }

    // Stretch markers: only the center region (between bevels)
    // Top row: mark stretchable columns
    for x in (BEVEL + 1)..=(w - BEVEL) {
        nine.put_pixel(x, 0, black);
    }
    // Left column: mark stretchable rows
    for y in (BEVEL + 1)..=(h - BEVEL) {
        nine.put_pixel(0, y, black);
    }

    nine
}

/// Encode an RGBA image as PNG bytes.
fn encode_png(img: &RgbaImage) -> Vec<u8> {
    let mut buf = Vec::new();
    img.write_with_encoder(image::codecs::png::PngEncoder::new(Cursor::new(&mut buf)))
        .expect("BUG: PNG encoding failed");
    buf
}

/// Read a zip entry by name (case-insensitive) into a byte buffer.
fn read_entry(wsz: &mut zip::ZipArchive<Cursor<&[u8]>>, actual_name: &str) -> Vec<u8> {
    let mut entry = wsz
        .by_name(actual_name)
        .unwrap_or_else(|e| panic!("failed to read {actual_name}: {e}"));
    let mut buf = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut buf)
        .unwrap_or_else(|e| panic!("failed to read {actual_name}: {e}"));
    buf
}

/// Decode image bytes into RGBA.
fn decode_sheet(data: &[u8], name: &str) -> RgbaImage {
    ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .unwrap_or_else(|e| panic!("failed to guess format for {name}: {e}"))
        .decode()
        .unwrap_or_else(|e| panic!("failed to decode {name}: {e}"))
        .to_rgba8()
}

/// Format an RGBA pixel as a `#RRGGBB` hex string.
fn pixel_hex(px: Rgba<u8>) -> String {
    format!("#{:02x}{:02x}{:02x}", px.0[0], px.0[1], px.0[2])
}

/// Sample the foreground (icon/symbol) color from a button sprite.
///
/// Finds the darkest pixel in the center region (inside the bevel) — this is
/// the symbol/icon color. More robust than hardcoded offsets across skins.
fn sample_fg_color(sprite: &RgbaImage) -> Rgba<u8> {
    let (w, h) = (sprite.width(), sprite.height());
    let mut darkest = Rgba([255, 255, 255, 255]);
    let mut min_luma = u32::MAX;
    for y in BEVEL..h - BEVEL {
        for x in BEVEL..w - BEVEL {
            let px = *sprite.get_pixel(x, y);
            let luma = u32::from(px.0[0]) + u32::from(px.0[1]) + u32::from(px.0[2]);
            if luma < min_luma {
                min_luma = luma;
                darkest = px;
            }
        }
    }
    darkest
}

/// Extract generic button backgrounds from CBUTTONS.BMP.
///
/// Uses the "stop" button (69,0 for normal, 69,18 for pressed) as it has
/// the least intrusive center symbol — after clearing, the frame is clean.
/// Also samples fg colors and generates `skin.toml`.
fn extract_buttons(
    wsz: &mut zip::ZipArchive<Cursor<&[u8]>>,
    entries: &HashMap<String, String>,
) -> HashMap<String, Vec<u8>> {
    let Some(actual_name) = entries.get("cbuttons.bmp") else {
        eprintln!("warning: cbuttons.bmp not found in .wsz");
        return HashMap::new();
    };

    let sheet_data = read_entry(wsz, actual_name);
    let sheet = decode_sheet(&sheet_data, actual_name);

    let mut assets = HashMap::new();

    // Stop button: (69, 0) normal, (69, 18) pressed — 23×18 each
    let normal = image::imageops::crop_imm(&sheet, 69, 0, 23, 18).to_image();
    let pressed = image::imageops::crop_imm(&sheet, 69, 18, 23, 18).to_image();

    // Sample fg colors from the symbol pixels before clearing center
    let normal_fg = sample_fg_color(&normal);
    let pressed_fg = sample_fg_color(&pressed);

    assets.insert(
        "button_normal.9.png".to_string(),
        encode_png(&make_button_nine_patch(&normal)),
    );
    assets.insert(
        "button_pressed.9.png".to_string(),
        encode_png(&make_button_nine_patch(&pressed)),
    );

    // Generate skin.toml with sampled colors
    let toml = format!(
        "[button_normal]\ncolor = \"{}\"\n\n[button_pressed]\ncolor = \"{}\"\n",
        pixel_hex(normal_fg),
        pixel_hex(pressed_fg),
    );
    assets.insert("skin.toml".to_string(), toml.into_bytes());

    assets
}

/// Build a case-insensitive entry name map for a zip archive.
fn build_entry_map(wsz: &mut zip::ZipArchive<Cursor<&[u8]>>) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    for i in 0..wsz.len() {
        let name = wsz
            .by_index(i)
            .expect("BUG: zip index out of bounds")
            .name()
            .to_string();
        entries.insert(name.to_lowercase(), name);
    }
    entries
}

/// Make a 9-patch that stretches only horizontally (full height preserved).
///
/// Adds a 1px border with black stretch markers on the top row (horizontal)
/// covering the entire width, and on the left column (vertical) covering the
/// entire height — so the host stretches both axes uniformly.
fn make_hstretch_nine_patch(img: &RgbaImage) -> RgbaImage {
    let (w, h) = (img.width(), img.height());
    let mut nine = RgbaImage::new(w + 2, h + 2);
    let black = Rgba([0, 0, 0, 255]);

    // Paste content at (1,1)
    for y in 0..h {
        for x in 0..w {
            nine.put_pixel(x + 1, y + 1, *img.get_pixel(x, y));
        }
    }

    // Horizontal stretch: mark all columns as stretchable
    for x in 1..=w {
        nine.put_pixel(x, 0, black);
    }
    // Vertical stretch: mark all rows as stretchable (uniform scale)
    for y in 1..=h {
        nine.put_pixel(0, y, black);
    }

    nine
}

/// Extract slider assets from POSBAR.BMP.
///
/// Layout (Winamp spec): 248×10 track + 29×10 thumb normal + 29×10 thumb pressed.
/// The track becomes a horizontally-stretchable 9-patch. Thumbs are plain PNGs.
fn extract_slider(
    wsz: &mut zip::ZipArchive<Cursor<&[u8]>>,
    entries: &HashMap<String, String>,
) -> HashMap<String, Vec<u8>> {
    let mut assets = HashMap::new();

    let Some(actual_name) = entries.get("posbar.bmp") else {
        eprintln!("warning: posbar.bmp not found in .wsz");
        return assets;
    };

    let sheet_data = read_entry(wsz, actual_name);
    let sheet = decode_sheet(&sheet_data, actual_name);

    // Track: (0,0) 248×10
    let track = image::imageops::crop_imm(&sheet, 0, 0, 248, 10).to_image();
    assets.insert(
        "slider_track.9.png".to_string(),
        encode_png(&make_hstretch_nine_patch(&track)),
    );

    // Thumb normal: (248,0) 29×10
    let thumb = image::imageops::crop_imm(&sheet, 248, 0, 29, 10).to_image();
    assets.insert("slider_thumb.png".to_string(), encode_png(&thumb));

    // Thumb pressed: (278,0) 29×10  (248+29=277, next at 278)
    // Some skins may be exactly 307px wide (248+29+29+1 padding) or 306.
    // Only extract if there's enough width.
    let pressed_x = 278;
    let pressed_w = sheet.width().saturating_sub(pressed_x).min(29);
    if pressed_w >= 20 {
        let thumb_pressed =
            image::imageops::crop_imm(&sheet, pressed_x, 0, pressed_w, 10).to_image();
        assets.insert(
            "slider_thumb_pressed.png".to_string(),
            encode_png(&thumb_pressed),
        );
    }

    assets
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (input_path, output_path) = parse_args(&args);

    let input_data = std::fs::read(&input_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", input_path.display()));

    let cursor = Cursor::new(input_data.as_slice());
    let mut wsz = zip::ZipArchive::new(cursor)
        .unwrap_or_else(|e| panic!("failed to open {}: {e}", input_path.display()));

    let entries = build_entry_map(&mut wsz);
    let mut assets = extract_buttons(&mut wsz, &entries);
    let slider_assets = extract_slider(&mut wsz, &entries);
    assets.extend(slider_assets);

    if assets.is_empty() {
        eprintln!("error: no assets extracted");
        std::process::exit(1);
    }

    let out_file = std::fs::File::create(&output_path)
        .unwrap_or_else(|e| panic!("failed to create {}: {e}", output_path.display()));
    let mut out_zip = zip::ZipWriter::new(out_file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut names: Vec<&String> = assets.keys().collect();
    names.sort();

    for name in names {
        out_zip
            .start_file(name, options)
            .unwrap_or_else(|e| panic!("failed to write zip entry {name}: {e}"));
        out_zip
            .write_all(&assets[name])
            .unwrap_or_else(|e| panic!("failed to write zip entry {name}: {e}"));
    }

    out_zip
        .finish()
        .unwrap_or_else(|e| panic!("failed to finalize zip: {e}"));

    println!(
        "created {} with {} assets",
        output_path.display(),
        assets.len()
    );
}

fn parse_args(args: &[String]) -> (PathBuf, PathBuf) {
    if args.len() < 4 {
        eprintln!("usage: wsz-to-skin <input.wsz> -o <output.zip>");
        std::process::exit(1);
    }

    let input = PathBuf::from(&args[1]);

    let output_idx = args.iter().position(|a| a == "-o").unwrap_or_else(|| {
        eprintln!("usage: wsz-to-skin <input.wsz> -o <output.zip>");
        std::process::exit(1);
    });

    if output_idx + 1 >= args.len() {
        eprintln!("usage: wsz-to-skin <input.wsz> -o <output.zip>");
        std::process::exit(1);
    }

    let output = PathBuf::from(&args[output_idx + 1]);
    (input, output)
}
