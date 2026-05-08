// Copyright (C) 2026  Braiins Systems s.r.o.

#![expect(
    clippy::too_many_lines,
    reason = "asset-embedding macros expand a lot of token-tree assembly inline"
)]

//! Proc macros for compiling and embedding assets at build time.
//!
//! Provides `include_icon!`, `include_bitmap!`, `include_mesh!`,
//! `include_nine_patch!`, `include_skin!`, and `include_audio!`.

mod mesh;

use std::path::{Path, PathBuf};

use proc_macro::TokenStream;
use quote::quote;
use syn::{LitStr, parse_macro_input};

/// Resolve an asset path relative to `CARGO_MANIFEST_DIR`.
///
/// When a `.stories.rs` file is compiled via `#[path = "..."]` from a
/// *different* crate, `CARGO_MANIFEST_DIR` points to the compiling crate,
/// not the source file's crate.  To handle this, we walk up parent
/// directories of `CARGO_MANIFEST_DIR` until we find the file.
fn resolve_asset(rel_path: &str) -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| panic!("CARGO_MANIFEST_DIR not set"));
    let base = Path::new(&manifest_dir);

    // Try the direct path first (normal case)
    let direct = base.join(rel_path);
    if direct.exists() {
        return direct;
    }

    // Walk up parent directories (handles cross-crate #[path] includes)
    let mut dir = base.to_path_buf();
    while dir.pop() {
        let candidate = dir.join(rel_path);
        if candidate.exists() {
            return candidate;
        }
    }

    panic!("asset not found: `{rel_path}` (searched from `{manifest_dir}`)");
}

/// Build a host-unique asset tag of the form `"<crate>::<file_stem>"`.
///
/// Two crates that ship an asset with the same filename register under
/// different tags, so they don't alias in the host's shared registry.
fn asset_tag(full_path: &Path) -> String {
    let crate_name = std::env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "unknown".to_owned());
    let stem = full_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    format!("{crate_name}::{stem}")
}

/// Embed a PNG (or other raster image) file as a `Bitmap` at compile time.
///
/// The raw file bytes are included directly; the host decodes on first registration.
/// Cargo tracks the file for recompilation when it changes.
///
/// # Usage
///
/// ```ignore
/// const FALCON_9: Bitmap = include_bitmap!("assets/falcon-9.png");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// with fallback to parent directories for cross-crate story files.
#[proc_macro]
pub fn include_bitmap(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    let rel_path = path_lit.value();

    let full_path = resolve_asset(&rel_path);
    let abs_path = full_path.display().to_string();
    let name = asset_tag(&full_path);

    let expanded = quote! {
        bmc_wasm_sdk::Bitmap {
            data: include_bytes!(#abs_path),
            name: #name,
        }
    };

    expanded.into()
}

/// Embed an audio file (WAV, OGG, MP3) as an `Audio` asset at compile time.
///
/// The raw file bytes are included directly; the host decodes on first registration.
/// Cargo tracks the file for recompilation when it changes.
///
/// # Usage
///
/// ```ignore
/// const TICK: Audio = include_audio!("assets/sounds/tick.wav");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// with fallback to parent directories for cross-crate story files.
#[proc_macro]
pub fn include_audio(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    let rel_path = path_lit.value();

    let full_path = resolve_asset(&rel_path);
    let abs_path = full_path.display().to_string();
    let name = asset_tag(&full_path);

    let expanded = quote! {
        bmc_wasm_sdk::Audio {
            data: include_bytes!(#abs_path),
            name: #name,
        }
    };

    expanded.into()
}

/// Embed a glTF 2.0 binary (.glb) mesh at compile time.
///
/// Parses the mesh, validates it against hardware constraints (triangle count,
/// vertex count, texture size), quantizes vertices, and packs everything into
/// an optimized binary format. The host uploads VBO/IBO/texture to GPU on
/// first registration.
///
/// # Compile-time validation
///
/// Produces clear `compile_error!` if:
/// - Triangle count > 5,000
/// - Vertex count > 65,535
/// - Missing normals
/// - Texture dimensions not power-of-2
/// - Texture > 1024x1024
/// - Non-triangulated faces
///
/// # Usage
///
/// ```ignore
/// static SUZANNE: Mesh = include_mesh!("assets/suzanne.glb");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// with fallback to parent directories for cross-crate story files.
#[proc_macro]
pub fn include_mesh(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    match include_mesh_impl(&path_lit) {
        Ok(stream) => stream.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn include_mesh_impl(path_lit: &LitStr) -> syn::Result<proc_macro2::TokenStream> {
    let span = path_lit.span();
    let rel_path = path_lit.value();
    let full_path = resolve_asset(&rel_path);
    let abs_path = full_path.display().to_string();

    if !full_path.exists() {
        return Err(syn::Error::new(
            span,
            format!("mesh file not found: {abs_path}"),
        ));
    }

    let (packed, face_normals, extra_tracked_paths) = mesh::pack_mesh(&full_path, span)?;
    let name = asset_tag(&full_path);

    // Generate face_normals as &[[f32; 3]] literal
    let normal_arrays = face_normals.iter().map(|[x, y, z]| {
        quote! { [#x, #y, #z] }
    });

    // Emit one `include_bytes!` per sidecar so cargo's dep-info tracks
    // them — without this the proc macro is a black box and edits to
    // `<stem>.msdf.{png,json}` don't trigger recompilation.
    let extra_tracks = extra_tracked_paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let path_str = path.to_str().ok_or_else(|| {
                syn::Error::new(span, format!("non-utf8 sidecar path: {}", path.display()))
            })?;
            let ident = quote::format_ident!("_TRACK_{}", i);
            Ok(quote! { const #ident: &[u8] = include_bytes!(#path_str); })
        })
        .collect::<syn::Result<Vec<_>>>()?;

    Ok(quote! {
        {
            const _TRACK: &[u8] = include_bytes!(#abs_path);
            #(#extra_tracks)*
            bmc_wasm_sdk::Mesh {
                data: &[#(#packed),*],
                face_normals: &[#(#normal_arrays),*],
                name: #name,
            }
        }
    })
}

/// Compile an SVG file into compact binary path data at build time.
///
/// The SVG is parsed by usvg which simplifies all elements (rects, circles,
/// transforms, CSS, etc.) into absolute bezier paths. The result is a compact
/// binary format that the host runtime converts into FemtoVG `Path` objects.
///
/// # Usage
///
/// ```ignore
/// const STAR: Icon = include_icon!("assets/star.svg");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// with fallback to parent directories for cross-crate story files.
/// Cargo tracks the SVG file for recompilation when it changes.
#[proc_macro]
pub fn include_icon(input: TokenStream) -> TokenStream {
    let path_lit = parse_macro_input!(input as LitStr);
    let rel_path = path_lit.value();

    let full_path = resolve_asset(&rel_path);
    let abs_path = full_path.display().to_string();

    let svg_data = std::fs::read_to_string(&full_path)
        .unwrap_or_else(|e| panic!("failed to read SVG `{}`: {e}", full_path.display()));

    let compiled = bmc_icon_compiler::compile_svg(&svg_data);
    let name = asset_tag(&full_path);

    // Emit const-compatible expression.
    // The include_bytes! ensures Cargo recompiles when the SVG file changes.
    let expanded = quote! {
        {
            const _TRACK: &[u8] = include_bytes!(#abs_path);
            bmc_wasm_sdk::Icon { data: &[#(#compiled),*], name: #name }
        }
    };

    expanded.into()
}

/// Embed a `.9.png` (Android 9-patch) file at compile time.
///
/// The macro decodes the 1px black-pixel border that encodes stretchable regions,
/// strips it, re-encodes the inner bitmap as PNG, and embeds both the image bytes
/// and the computed insets as a `NinePatchAsset`.
///
/// # Usage
///
/// ```ignore
/// const BUTTON_BG: NinePatchAsset = include_nine_patch!("assets/button.9.png");
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// with fallback to parent directories for cross-crate story files.
/// At runtime, call `ensure_nine_patch_registered(&BUTTON_BG)` to get a `NinePatch`
/// with a host-registered bitmap ID.
#[proc_macro]
pub fn include_nine_patch(input: TokenStream) -> TokenStream {
    include_nine_patch_impl(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn include_nine_patch_impl(input: TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    let path_lit: LitStr = syn::parse(input)?;
    let rel_path = path_lit.value();
    let span = path_lit.span();

    let full_path = resolve_asset(&rel_path);

    let file_data = std::fs::read(&full_path).map_err(|e| {
        syn::Error::new(
            span,
            format!("failed to read .9.png `{}`: {e}", full_path.display()),
        )
    })?;

    let img = image::load_from_memory(&file_data).map_err(|e| {
        syn::Error::new(
            span,
            format!("failed to decode .9.png `{}`: {e}", full_path.display()),
        )
    })?;
    let rgba = img.to_rgba8();
    let (full_w, full_h) = (rgba.width(), rgba.height());

    let insets =
        bmc_render_skin::try_parse_nine_patch_insets(full_w, full_h, |x, y| rgba.get_pixel(x, y).0)
            .map_err(|e| syn::Error::new(span, format!(".9.png `{}`: {e}", full_path.display())))?;
    let left = insets.left;
    let right = insets.right;
    let top = insets.top;
    let bottom = insets.bottom;

    // try_parse_nine_patch_insets enforces full_w >= 3 && full_h >= 3,
    // so these subtractions cannot underflow.
    let inner_w = full_w - 2;
    let inner_h = full_h - 2;

    // Extract inner bitmap and re-encode as PNG
    let inner = image::imageops::crop_imm(&rgba, 1, 1, inner_w, inner_h).to_image();
    let mut png_bytes: Vec<u8> = Vec::new();
    inner
        .write_with_encoder(image::codecs::png::PngEncoder::new(std::io::Cursor::new(
            &mut png_bytes,
        )))
        .map_err(|e| syn::Error::new(span, format!("failed to re-encode inner bitmap: {e}")))?;

    let png_data = png_bytes.as_slice();

    let abs_path = full_path.display().to_string();
    let name = asset_tag(&full_path);

    Ok(quote! {
        {
            // Track the source file for recompilation
            const _TRACK: &[u8] = include_bytes!(#abs_path);
            bmc_wasm_sdk::NinePatchAsset {
                data: &[#(#png_data),*],
                name: #name,
                left: #left,
                top: #top,
                right: #right,
                bottom: #bottom,
            }
        }
    })
}

/// Embed a skin at compile time as a `Skin` with named assets.
///
/// Accepts either a **zip file** or an **uncompressed directory** containing
/// `.png` / `.9.png` files and an optional `skin.toml`. Nine-patch files have
/// their 1px border parsed and stripped; plain PNGs get zero insets.
/// Asset names are filenames without extension (and without `.9` suffix).
///
/// # Usage
///
/// ```ignore
/// const MY_SKIN: Skin = include_skin!("assets/my-skin.zip");
/// // — or for easier development —
/// const MY_SKIN: Skin = include_skin!("assets/my-skin/");
/// // later: MY_SKIN.get("button_normal") → NinePatch
/// ```
///
/// The path is resolved relative to the crate's `CARGO_MANIFEST_DIR`,
/// with fallback to parent directories for cross-crate story files.
#[proc_macro]
pub fn include_skin(input: TokenStream) -> TokenStream {
    include_skin_impl(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn include_skin_impl(input: TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    let path_lit: LitStr = syn::parse(input)?;
    let rel_path = path_lit.value();
    let span = path_lit.span();

    let full_path = resolve_asset(&rel_path);

    let (files, meta) = if full_path.is_dir() {
        load_skin_from_dir(&full_path)
    } else {
        load_skin_from_zip(&full_path)
    };

    let skin_name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            syn::Error::new(
                span,
                format!(
                    "skin.toml missing required top-level `name` field in `{}`",
                    full_path.display()
                ),
            )
        })?
        .to_string();
    let skin_description = meta
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            syn::Error::new(
                span,
                format!(
                    "skin.toml missing required top-level `description` field in `{}`",
                    full_path.display()
                ),
            )
        })?
        .to_string();

    // Parse [palette] section — freeform string→color map.
    let palette_entries: Vec<(String, u32)> = meta
        .get("palette")
        .and_then(|v| v.as_table())
        .map(|table| {
            table
                .iter()
                .filter_map(|(key, value)| {
                    let hex = value.as_str()?;
                    let color = bmc_render_skin::parse_hex_color(
                        hex,
                        &format!("skin.toml [palette].{key}"),
                    )
                    .to_u32();
                    Some((key.clone(), color))
                })
                .collect()
        })
        .unwrap_or_default();

    let palette_tokens: Vec<_> = palette_entries
        .iter()
        .map(|(key, color)| {
            quote! {
                (#key, bmc_wasm_sdk::colors::Color::from_raw(#color))
            }
        })
        .collect();

    let assets = process_skin_files(&files, &meta, span)?;

    let asset_tokens: Vec<_> = assets
        .iter()
        .map(
            |(name, data, left, top, right, bottom, width, height, color)| {
                let data_slice = data.as_slice();
                quote! {
                    bmc_wasm_sdk::SkinAsset {
                        name: #name,
                        data: &[#(#data_slice),*],
                        left: #left,
                        top: #top,
                        right: #right,
                        bottom: #bottom,
                        width: #width,
                        height: #height,
                        color: bmc_wasm_sdk::colors::Color::from_raw(#color),
                    }
                }
            },
        )
        .collect();

    let abs_path = full_path.display().to_string();

    let expanded: proc_macro2::TokenStream = if full_path.is_dir() {
        // For directories, track skin.toml and all PNG files for recompilation.
        let mut track_tokens = Vec::new();
        for (name, _) in &files {
            let file_path = full_path.join(name);
            let file_path_str = file_path.to_str().expect("BUG: non-UTF-8 path");
            track_tokens.push(quote! {
                const _: &[u8] = include_bytes!(#file_path_str);
            });
        }
        let toml_path = full_path.join("skin.toml");
        if toml_path.exists() {
            let toml_path_str = toml_path.to_str().expect("BUG: non-UTF-8 path");
            track_tokens.push(quote! {
                const _: &[u8] = include_bytes!(#toml_path_str);
            });
        }
        quote! {
            {
                #(#track_tokens)*
                bmc_wasm_sdk::Skin {
                    name: #skin_name,
                    description: #skin_description,
                    palette: &[#(#palette_tokens),*],
                    assets: &[#(#asset_tokens),*]
                }
            }
        }
    } else {
        quote! {
            {
                const _TRACK: &[u8] = include_bytes!(#abs_path);
                bmc_wasm_sdk::Skin {
                    name: #skin_name,
                    description: #skin_description,
                    palette: &[#(#palette_tokens),*],
                    assets: &[#(#asset_tokens),*]
                }
            }
        }
    };

    Ok(expanded)
}

/// Raw skin file: `(filename, bytes)`.
type SkinFile = (String, Vec<u8>);
/// Parsed asset: `(name, png_bytes, left, top, right, bottom, width, height, color)`.
type SkinAssetData = (String, Vec<u8>, u16, u16, u16, u16, u16, u16, u32);

fn load_skin_from_zip(
    path: &std::path::Path,
) -> (Vec<SkinFile>, toml::map::Map<String, toml::Value>) {
    let file_data = std::fs::read(path)
        .unwrap_or_else(|e| panic!("failed to read skin zip `{}`: {e}", path.display()));

    let cursor = std::io::Cursor::new(&file_data);
    let mut archive = zip::ZipArchive::new(cursor)
        .unwrap_or_else(|e| panic!("failed to open skin zip `{}`: {e}", path.display()));

    // Parse skin.toml if present — per-asset metadata (color, etc.)
    let meta: toml::map::Map<String, toml::Value> = (|| {
        let mut toml_entry = archive.by_name("skin.toml").ok()?;
        let mut toml_str = String::new();
        std::io::Read::read_to_string(&mut toml_entry, &mut toml_str).ok()?;
        toml_str.parse::<toml::Table>().ok()
    })()
    .unwrap_or_default();

    // Only collect image files
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .unwrap_or_else(|e| panic!("failed to read zip entry {i}: {e}"));
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !std::path::Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        {
            continue;
        }
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data)
            .unwrap_or_else(|e| panic!("failed to read zip entry `{name}`: {e}"));
        files.push((name, data));
    }
    (files, meta)
}

fn load_skin_from_dir(
    dir: &std::path::Path,
) -> (Vec<SkinFile>, toml::map::Map<String, toml::Value>) {
    // Parse skin.toml if present — per-asset metadata (color, etc.)
    let meta: toml::map::Map<String, toml::Value> = (|| {
        let toml_str = std::fs::read_to_string(dir.join("skin.toml")).ok()?;
        toml_str.parse::<toml::Table>().ok()
    })()
    .unwrap_or_default();

    let mut files = Vec::new();
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("failed to read skin dir `{}`: {e}", dir.display()));
    let mut names: Vec<_> = entries
        .filter_map(std::result::Result::ok)
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
        })
        .collect();
    names.sort_by_key(std::fs::DirEntry::file_name);

    for entry in names {
        let name = entry.file_name().to_string_lossy().into_owned();
        let data = std::fs::read(entry.path())
            .unwrap_or_else(|e| panic!("failed to read `{}`: {e}", entry.path().display()));
        files.push((name, data));
    }
    (files, meta)
}

fn process_skin_files(
    files: &[SkinFile],
    meta: &toml::map::Map<String, toml::Value>,
    span: proc_macro2::Span,
) -> syn::Result<Vec<SkinAssetData>> {
    let mut assets = Vec::new();

    for (entry_name, entry_bytes) in files {
        // Only process image files
        let is_nine_patch = entry_name.ends_with(".9.png");

        // Derive asset name: strip path prefix, remove extension
        let file_stem = std::path::Path::new(entry_name)
            .file_name()
            .expect("BUG: entry has no filename")
            .to_str()
            .expect("BUG: entry filename is not UTF-8");
        let asset_name = if is_nine_patch {
            file_stem
                .strip_suffix(".9.png")
                .expect("BUG: suffix mismatch")
        } else {
            file_stem
                .strip_suffix(".png")
                .expect("BUG: suffix mismatch")
        };

        // Look up per-asset metadata from skin.toml [assets.<name>]
        let assets_table = meta.get("assets").and_then(|v| v.as_table());
        let color = assets_table
            .and_then(|t| t.get(asset_name))
            .and_then(|v| v.as_table())
            .and_then(|t| t.get("color"))
            .and_then(|v| v.as_str())
            .map_or(0, |hex| {
                bmc_render_skin::parse_hex_color(hex, &format!("skin.toml [{asset_name}]")).to_u32()
            });

        if is_nine_patch {
            let img = image::load_from_memory(entry_bytes).map_err(|e| {
                syn::Error::new(span, format!("failed to decode `{entry_name}`: {e}"))
            })?;
            let rgba = img.to_rgba8();
            let (full_w, full_h) = (rgba.width(), rgba.height());

            let insets = bmc_render_skin::try_parse_nine_patch_insets(full_w, full_h, |x, y| {
                rgba.get_pixel(x, y).0
            })
            .map_err(|e| syn::Error::new(span, format!("`{entry_name}`: {e}")))?;

            // try_parse_nine_patch_insets enforces full_w >= 3 && full_h >= 3,
            // so these subtractions cannot underflow.
            let inner_w = full_w - 2;
            let inner_h = full_h - 2;
            let inner = image::imageops::crop_imm(&rgba, 1, 1, inner_w, inner_h).to_image();
            let mut png_bytes: Vec<u8> = Vec::new();
            inner
                .write_with_encoder(image::codecs::png::PngEncoder::new(std::io::Cursor::new(
                    &mut png_bytes,
                )))
                .map_err(|e| {
                    syn::Error::new(span, format!("failed to re-encode `{entry_name}`: {e}"))
                })?;

            let inner_w_u16 = u16::try_from(inner_w).map_err(|_| {
                syn::Error::new(
                    span,
                    format!("`{entry_name}` inner width {inner_w} exceeds u16::MAX"),
                )
            })?;
            let inner_h_u16 = u16::try_from(inner_h).map_err(|_| {
                syn::Error::new(
                    span,
                    format!("`{entry_name}` inner height {inner_h} exceeds u16::MAX"),
                )
            })?;

            assets.push((
                asset_name.to_string(),
                png_bytes,
                insets.left,
                insets.top,
                insets.right,
                insets.bottom,
                inner_w_u16,
                inner_h_u16,
                color,
            ));
        } else {
            let img = image::load_from_memory(entry_bytes).map_err(|e| {
                syn::Error::new(span, format!("failed to decode `{entry_name}`: {e}"))
            })?;
            let img_w = img.width();
            let img_h = img.height();
            let img_w_u16 = u16::try_from(img_w).map_err(|_| {
                syn::Error::new(
                    span,
                    format!("`{entry_name}` width {img_w} exceeds u16::MAX"),
                )
            })?;
            let img_h_u16 = u16::try_from(img_h).map_err(|_| {
                syn::Error::new(
                    span,
                    format!("`{entry_name}` height {img_h} exceeds u16::MAX"),
                )
            })?;
            assets.push((
                asset_name.to_string(),
                entry_bytes.clone(),
                0,
                0,
                0,
                0,
                img_w_u16,
                img_h_u16,
                color,
            ));
        }
    }
    Ok(assets)
}
