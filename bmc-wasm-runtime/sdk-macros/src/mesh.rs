// Copyright (C) 2026  Braiins Systems s.r.o.

//! `include_mesh!` proc macro implementation.
//!
//! Parses a glTF 2.0 binary (.glb) at compile time, validates it against
//! hardware constraints, quantizes vertices, and packs everything into
//! the optimized binary mesh format consumed by the host `MeshRenderer`.

#![expect(
    clippy::too_many_lines,
    clippy::manual_assert,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_let_else,
    reason = "compile-time mesh packing is validation-heavy and uses intentional quantization casts"
)]

use std::path::Path;

use bmc_wasm_protocol::mesh::{
    FLAG_HAS_NORMAL_MAP, FLAG_HAS_TANGENTS, FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE,
    MAX_TEXTURE_SIZE, MAX_TRIANGLES, MAX_VERTICES, MESH_MAGIC,
};

/// Pack a mesh from a .glb file into the optimized binary format.
///
/// Returns `(packed_binary, face_normals)` where `face_normals` are extracted
/// from glTF node extras (empty if not present).
///
/// # Panics
/// Panics with a descriptive compile error if the mesh violates constraints.
pub fn pack_mesh(glb_path: &Path) -> (Vec<u8>, Vec<[f32; 3]>) {
    let glb_data = std::fs::read(glb_path)
        .unwrap_or_else(|e| panic!("failed to read mesh `{}`: {e}", glb_path.display()));

    let (document, buffers, images) = gltf::import_slice(&glb_data)
        .unwrap_or_else(|e| panic!("failed to parse glTF `{}`: {e}", glb_path.display()));

    // Extract first mesh, first primitive
    let mesh = document
        .meshes()
        .next()
        .unwrap_or_else(|| panic!("mesh `{}` has no meshes", glb_path.display()));

    let primitive = mesh
        .primitives()
        .next()
        .unwrap_or_else(|| panic!("mesh `{}` has no primitives", glb_path.display()));

    // Validate: must be triangles
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        panic!(
            "mesh `{}` uses {:?} mode, must be Triangles — run scripts/prepare_model.py to triangulate",
            glb_path.display(),
            primitive.mode()
        );
    }

    // Extract positions (required)
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .unwrap_or_else(|| panic!("mesh `{}` has no position data", glb_path.display()))
        .collect();

    let vertex_count = positions.len();
    if vertex_count > MAX_VERTICES as usize {
        panic!(
            "mesh `{}` has {vertex_count} vertices (max {MAX_VERTICES})",
            glb_path.display()
        );
    }

    // Extract normals (required)
    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .unwrap_or_else(|| {
            panic!(
                "mesh `{}` has no normal data — run scripts/prepare_model.py to recalculate normals",
                glb_path.display()
            )
        })
        .collect();

    // Extract UVs (optional)
    let uvs: Option<Vec<[f32; 2]>> = reader.read_tex_coords(0).map(|tc| tc.into_f32().collect());

    // Extract tangents (optional — needed for normal mapping)
    let tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(|t| t.collect());

    // Extract indices (required)
    let indices: Vec<u16> = reader
        .read_indices()
        .unwrap_or_else(|| panic!("mesh `{}` has no index data", glb_path.display()))
        .into_u32()
        .map(|i| {
            u16::try_from(i).unwrap_or_else(|_| {
                panic!(
                    "mesh `{}` has index {i} > 65535 — too many vertices",
                    glb_path.display()
                )
            })
        })
        .collect();

    let tri_count = indices.len() / 3;
    if tri_count > MAX_TRIANGLES as usize {
        panic!(
            "mesh `{}` has {tri_count} triangles (max {MAX_TRIANGLES}) — run scripts/prepare_model.py to decimate",
            glb_path.display()
        );
    }

    // Extract albedo texture (optional — from material's base color)
    let texture_data = extract_texture(&primitive, &images, glb_path);

    // Extract normal map texture (optional — from material's normalTexture)
    let normal_map_data = extract_normal_map(&primitive, &images, glb_path);

    // Compute AABB for position quantization
    let (aabb_min, aabb_max) = compute_aabb(&positions);

    // Quantize and pack
    let has_uvs = uvs.is_some();
    let has_tangents = tangents.is_some();
    let has_texture = texture_data.is_some();
    let has_normal_map = normal_map_data.is_some();
    // Vertex sizes (quantized):
    //   base: 3×i16 pos + u32 normal = 10 bytes
    //   + 2×u16 uv = 14 bytes
    //   + 4×i16 tangent = 22 bytes (tangent xyzw quantized to i16)
    let vertex_size = match (has_uvs, has_tangents) {
        (false, _) => 10,
        (true, false) => 14,
        (true, true) => 22,
    };
    let vertex_data_size = vertex_count * vertex_size;
    let index_data_size = indices.len() * 2;

    // Compress textures to ETC1 at compile time (8:1 vs RGBA8, no alpha needed).
    let compressed_texture = texture_data.as_ref().map(|t| compress_to_etc1(t));
    let compressed_normal_map = normal_map_data.as_ref().map(|t| compress_to_etc1(t));

    let texture_size = compressed_texture.as_ref().map_or(0, Vec::len);
    let normal_map_size = compressed_normal_map.as_ref().map_or(0, Vec::len);

    // Layout: [header 40B][AABB 24B][vertices][indices][albedo texture][normal map]
    let aabb_offset = HEADER_SIZE;
    let vertex_offset = aabb_offset + 24;
    let index_offset = vertex_offset + vertex_data_size;
    let texture_offset = index_offset + index_data_size;
    let normal_map_offset = texture_offset + texture_size;
    let total_size = normal_map_offset + normal_map_size;

    let mut flags = 0u8;
    if has_texture {
        flags |= FLAG_HAS_TEXTURE;
    }
    if has_uvs {
        flags |= FLAG_HAS_UVS;
    }
    if has_tangents {
        flags |= FLAG_HAS_TANGENTS;
    }
    if has_normal_map {
        flags |= FLAG_HAS_NORMAL_MAP;
    }

    let mut buf = vec![0u8; total_size];

    // Write header (40 bytes)
    write_u32(&mut buf, 0, MESH_MAGIC);
    write_u32(&mut buf, 4, vertex_count as u32);
    write_u32(&mut buf, 8, indices.len() as u32);
    write_u32(&mut buf, 12, vertex_offset as u32);
    write_u32(&mut buf, 16, index_offset as u32);
    write_u32(&mut buf, 20, texture_offset as u32);
    if let Some(ref tex) = texture_data {
        write_u16(&mut buf, 24, tex.width as u16);
        write_u16(&mut buf, 26, tex.height as u16);
    }
    buf[28] = bmc_wasm_protocol::mesh::TextureFormat::Etc1 as u8;
    buf[29] = flags;
    // Normal map info (offsets 30-37)
    write_u32(&mut buf, 30, normal_map_offset as u32);
    if let Some(ref nmap) = normal_map_data {
        write_u16(&mut buf, 34, nmap.width as u16);
        write_u16(&mut buf, 36, nmap.height as u16);
    }

    // Write AABB (6 floats)
    write_f32(&mut buf, aabb_offset, aabb_min[0]);
    write_f32(&mut buf, aabb_offset + 4, aabb_min[1]);
    write_f32(&mut buf, aabb_offset + 8, aabb_min[2]);
    write_f32(&mut buf, aabb_offset + 12, aabb_max[0]);
    write_f32(&mut buf, aabb_offset + 16, aabb_max[1]);
    write_f32(&mut buf, aabb_offset + 20, aabb_max[2]);

    // Write quantized vertices
    let mut offset = vertex_offset;
    for i in 0..vertex_count {
        let pos = positions[i];
        let norm = normals[i];

        // Quantize position to i16 within AABB
        let qx = quantize_position(pos[0], aabb_min[0], aabb_max[0]);
        let qy = quantize_position(pos[1], aabb_min[1], aabb_max[1]);
        let qz = quantize_position(pos[2], aabb_min[2], aabb_max[2]);

        write_i16(&mut buf, offset, qx);
        write_i16(&mut buf, offset + 2, qy);
        write_i16(&mut buf, offset + 4, qz);

        // Pack normal into 10/10/10/2
        let packed_normal = pack_normal_10_10_10_2(norm);
        write_u32(&mut buf, offset + 6, packed_normal);

        if has_uvs {
            let uv = uvs.as_ref().expect("BUG: has_uvs set but no UV data")[i];
            let qu = (uv[0].clamp(0.0, 1.0) * 65_535.0) as u16;
            let qv = (uv[1].clamp(0.0, 1.0) * 65_535.0) as u16;
            write_u16(&mut buf, offset + 10, qu);
            write_u16(&mut buf, offset + 12, qv);

            if has_tangents {
                // Tangent xyzw quantized to i16 (range -1..1 → -32767..32767)
                let tan = tangents
                    .as_ref()
                    .expect("BUG: has_tangents set but no data")[i];
                let qt = |v: f32| -> i16 { (v.clamp(-1.0, 1.0) * 32_767.0).round() as i16 };
                write_i16(&mut buf, offset + 14, qt(tan[0]));
                write_i16(&mut buf, offset + 16, qt(tan[1]));
                write_i16(&mut buf, offset + 18, qt(tan[2]));
                write_i16(&mut buf, offset + 20, qt(tan[3]));
                offset += 22;
            } else {
                offset += 14;
            }
        } else {
            offset += 10;
        }
    }

    // Write indices
    for (i, &idx) in indices.iter().enumerate() {
        write_u16(&mut buf, index_offset + i * 2, idx);
    }

    // Write ETC1-compressed albedo texture
    if let Some(ref data) = compressed_texture {
        buf[texture_offset..texture_offset + data.len()].copy_from_slice(data);
    }

    // Write ETC1-compressed normal map
    if let Some(ref data) = compressed_normal_map {
        buf[normal_map_offset..normal_map_offset + data.len()].copy_from_slice(data);
    }

    // Extract face normals from glTF node extras (if present).
    // Blender exports custom properties on objects as node extras.
    // Format: flat f64 array [x1,y1,z1, x2,y2,z2, ...] with face_count.
    let face_normals = extract_face_normals(&document);

    (buf, face_normals)
}

struct TextureData {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

fn extract_texture(
    primitive: &gltf::Primitive<'_>,
    images: &[gltf::image::Data],
    glb_path: &Path,
) -> Option<TextureData> {
    let material = primitive.material();
    let pbr = material.pbr_metallic_roughness();
    let info = pbr.base_color_texture()?;
    let texture = info.texture();
    let image_index = texture.source().index();

    if image_index >= images.len() {
        return None;
    }

    let image = &images[image_index];
    let width = image.width;
    let height = image.height;

    if width > MAX_TEXTURE_SIZE || height > MAX_TEXTURE_SIZE {
        panic!(
            "mesh `{}` texture is {}x{} (max {MAX_TEXTURE_SIZE}x{MAX_TEXTURE_SIZE})",
            glb_path.display(),
            width,
            height
        );
    }

    // Ensure power-of-2 dimensions for ES 2.0 NPOT limitations
    if !width.is_power_of_two() || !height.is_power_of_two() {
        panic!(
            "mesh `{}` texture dimensions {}x{} are not power-of-2 — resize in Blender",
            glb_path.display(),
            width,
            height
        );
    }

    // Convert to RGBA8 regardless of source format
    let rgba_data = match image.format {
        gltf::image::Format::R8G8B8A8 => image.pixels.clone(),
        gltf::image::Format::R8G8B8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() / 3 * 4);
            for chunk in image.pixels.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        gltf::image::Format::R8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
            for &v in &image.pixels {
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
            rgba
        }
        gltf::image::Format::R8G8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() / 2 * 4);
            for chunk in image.pixels.chunks(2) {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            rgba
        }
        other => panic!(
            "mesh `{}` texture uses unsupported format {:?}",
            glb_path.display(),
            other
        ),
    };

    Some(TextureData {
        data: rgba_data,
        width,
        height,
    })
}

fn extract_normal_map(
    primitive: &gltf::Primitive<'_>,
    images: &[gltf::image::Data],
    glb_path: &Path,
) -> Option<TextureData> {
    let material = primitive.material();
    let info = material.normal_texture()?;
    let texture = info.texture();
    let image_index = texture.source().index();

    if image_index >= images.len() {
        return None;
    }

    let image = &images[image_index];
    let width = image.width;
    let height = image.height;

    if width > MAX_TEXTURE_SIZE || height > MAX_TEXTURE_SIZE {
        panic!(
            "mesh `{}` normal map is {}x{} (max {MAX_TEXTURE_SIZE}x{MAX_TEXTURE_SIZE})",
            glb_path.display(),
            width,
            height
        );
    }

    if !width.is_power_of_two() || !height.is_power_of_two() {
        panic!(
            "mesh `{}` normal map dimensions {}x{} are not power-of-2",
            glb_path.display(),
            width,
            height
        );
    }

    // Normal maps are typically RGB
    let rgba_data = match image.format {
        gltf::image::Format::R8G8B8A8 => image.pixels.clone(),
        gltf::image::Format::R8G8B8 => {
            let mut rgba = Vec::with_capacity(image.pixels.len() / 3 * 4);
            for chunk in image.pixels.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        other => panic!(
            "mesh `{}` normal map uses unsupported format {:?}",
            glb_path.display(),
            other
        ),
    };

    Some(TextureData {
        data: rgba_data,
        width,
        height,
    })
}

/// Extract face normals from glTF node extras.
///
/// Looks for `face_normals` (flat f64 array) and `face_count` on the first node.
/// Returns empty vec if not present.
fn extract_face_normals(document: &gltf::Document) -> Vec<[f32; 3]> {
    let node = match document.nodes().next() {
        Some(n) => n,
        None => return Vec::new(),
    };
    let extras = match node.extras() {
        Some(e) => e,
        None => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(extras.get()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let face_count = match json.get("face_count").and_then(|v| v.as_u64()) {
        Some(c) => c as usize,
        None => return Vec::new(),
    };
    let flat = match json.get("face_normals").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    if flat.len() != face_count * 3 {
        return Vec::new();
    }
    flat.chunks(3)
        .map(|c| {
            [
                c[0].as_f64().unwrap_or(0.0) as f32,
                c[1].as_f64().unwrap_or(0.0) as f32,
                c[2].as_f64().unwrap_or(0.0) as f32,
            ]
        })
        .collect()
}

/// Compress RGBA8 texture data to ETC1 using Intel ISPC encoder.
fn compress_to_etc1(tex: &TextureData) -> Vec<u8> {
    let surface = intel_tex_2::RgbaSurface {
        data: &tex.data,
        width: tex.width,
        height: tex.height,
        stride: tex.width * 4,
    };
    intel_tex_2::etc1::compress_blocks(&intel_tex_2::etc1::slow_settings(), &surface)
}

fn compute_aabb(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in positions {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    (min, max)
}

fn quantize_position(val: f32, min: f32, max: f32) -> i16 {
    let range = max - min;
    if range < 1e-8 {
        return 0;
    }
    let t = (val - min) / range; // 0..1
    let q = t * 65_534.0 - 32_767.0; // -32767..32767
    q.round() as i16
}

fn pack_normal_10_10_10_2(n: [f32; 3]) -> u32 {
    let to_10bit = |v: f32| -> u32 {
        let clamped = v.clamp(-1.0, 1.0);
        let scaled = (clamped * 511.0).round() as i32;
        (scaled & 0x3FF) as u32
    };
    let x = to_10bit(n[0]);
    let y = to_10bit(n[1]);
    let z = to_10bit(n[2]);
    x | (y << 10) | (z << 20)
}

fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}

fn write_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
}

fn write_i16(buf: &mut [u8], offset: usize, val: i16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
}

fn write_f32(buf: &mut [u8], offset: usize, val: f32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}
