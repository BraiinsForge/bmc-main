// Copyright (C) 2026  Braiins Systems s.r.o.

//! GL mesh renderer — arbitrary 3D meshes with quaternion-based orientation.
//!
//! Renders a mesh to an offscreen FBO, shared zero-copy with femtovg as a
//! native texture (identical pattern to `SphereRenderer`).
//!
//! Shader: simple diffuse + hemisphere ambient with optional directional light.
//! Target: OpenGL ES 2.0 (GLSL ES 1.00) on Vivante GC400.

#![expect(clippy::cast_precision_loss, clippy::cast_possible_wrap)]

use anyhow::{Result, bail};
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, ImageFlags, ImageId, ImageInfo, PixelFormat};
use glow::HasContext;

use bmc_wasm_protocol::mesh::{
    FLAG_HAS_NORMAL_MAP, FLAG_HAS_TANGENTS, FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE, MESH_MAGIC,
};

// ── Shaders (GLSL ES 1.00 / #version 100) ──────────────────────────

const VERTEX_SHADER: &str = "\
#version 100
attribute vec3 a_pos;
attribute vec3 a_normal;
attribute vec2 a_uv;
attribute vec4 a_tangent;

uniform mat4 u_mvp;
uniform mat3 u_normal_mat;

varying vec3 v_normal;
varying vec2 v_uv;
varying vec3 v_tangent;
varying vec3 v_bitangent;

void main() {
    v_normal = u_normal_mat * a_normal;
    v_uv = a_uv;
    v_tangent = u_normal_mat * a_tangent.xyz;
    v_bitangent = cross(v_normal, v_tangent) * a_tangent.w;
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
";

const FRAGMENT_SHADER: &str = "\
#version 100
precision mediump float;

uniform sampler2D u_diffuse;
uniform sampler2D u_normal_map;
uniform vec3 u_light_dir;
uniform float u_has_texture;
uniform float u_has_normal_map;
uniform float u_ambient;
uniform float u_specular;
// UV-rect highlight: brightens pixels within the rect with the given color.
// u_highlight_rect = (u_min, v_min, u_max, v_max), all < 0 = disabled.
uniform vec4 u_highlight_rect;
uniform vec3 u_highlight_color;

varying vec3 v_normal;
varying vec2 v_uv;
varying vec3 v_tangent;
varying vec3 v_bitangent;

void main() {
    vec3 albedo;
    if (u_has_texture > 0.5) {
        albedo = texture2D(u_diffuse, v_uv).rgb;
    } else {
        // Default material: warm clay color derived from normal for visual interest
        vec3 n = normalize(v_normal);
        albedo = vec3(0.7, 0.55, 0.45) + n * 0.15;
    }

    // UV-rect highlight: tint pixels within the rect toward the highlight color
    if (u_highlight_rect.x >= 0.0) {
        if (v_uv.x >= u_highlight_rect.x && v_uv.x <= u_highlight_rect.z &&
            v_uv.y >= u_highlight_rect.y && v_uv.y <= u_highlight_rect.w) {
            albedo = mix(albedo, u_highlight_color, 0.4);
        }
    }

    // Compute shading normal — perturbed by normal map if present
    vec3 N = normalize(v_normal);
    if (u_has_normal_map > 0.5) {
        vec3 T = normalize(v_tangent);
        vec3 B = normalize(v_bitangent);
        vec3 map = texture2D(u_normal_map, v_uv).rgb * 2.0 - 1.0;
        N = normalize(T * map.x + B * map.y + N * map.z);
    }

    // View direction: camera at origin looking down -Z, object at -distance.
    // For small centered objects, view dir ≈ constant +Z in view space.
    vec3 V = vec3(0.0, 0.0, 1.0);

    float diffuse = 1.0;
    float spec = 0.0;
    if (dot(u_light_dir, u_light_dir) > 0.001) {
        // Diffuse (Lambert) with configurable ambient
        diffuse = max(dot(N, u_light_dir), 0.0) * (1.0 - u_ambient) + u_ambient;
        // Specular (Blinn-Phong) with configurable intensity
        vec3 H = normalize(u_light_dir + V);
        spec = pow(max(dot(N, H), 0.0), 32.0) * u_specular;
    }

    // Edge darkening: subtle rim falloff for depth
    float rim = 1.0 - max(dot(N, V), 0.0);
    float edge_darken = 1.0 - rim * rim * 0.3;

    gl_FragColor = vec4(albedo * diffuse * edge_darken + vec3(spec), 1.0);
}
";

/// Epsilon for dirty-checking mesh parameters.
const DIRTY_EPSILON: f32 = 0.001;

/// Atlas grid: 3 columns × 3 rows = 9 slots.
const ATLAS_COLS: u32 = 3;
const ATLAS_ROWS: u32 = 3;
/// Per-slot pixel size (atlas is `ATLAS_COLS * SLOT_SIZE` × `ATLAS_ROWS * SLOT_SIZE`).
const SLOT_SIZE: u32 = 320;
/// Total atlas dimensions.
const ATLAS_W: u32 = ATLAS_COLS * SLOT_SIZE;
const ATLAS_H: u32 = ATLAS_ROWS * SLOT_SIZE;
/// Maximum number of atlas slots.
const MAX_SLOTS: u32 = ATLAS_COLS * ATLAS_ROWS;

// ── Per-slot dirty state ────────────────────────────────────────────

/// Dirty-check state for a single atlas slot.
///
/// Stores the last-rendered parameter values. On each `render()` call, if any
/// parameter changed beyond `DIRTY_EPSILON`, the slot is re-rendered.
#[derive(Clone)]
#[expect(clippy::struct_field_names)] // "prev_" prefix is intentional for clarity
struct SlotState {
    prev_mesh_id: u16,
    prev_qx: f32,
    prev_qy: f32,
    prev_qz: f32,
    prev_qw: f32,
    prev_px: f32,
    prev_py: f32,
    prev_pz: f32,
    prev_scale: f32,
    prev_fov: f32,
    prev_distance: f32,
    prev_light_pitch: f32,
    prev_light_yaw: f32,
    prev_hl_u_min: f32,
}

impl SlotState {
    fn new() -> Self {
        Self {
            prev_mesh_id: u16::MAX,
            prev_qx: f32::NAN,
            prev_qy: f32::NAN,
            prev_qz: f32::NAN,
            prev_qw: f32::NAN,
            prev_px: f32::NAN,
            prev_py: f32::NAN,
            prev_pz: f32::NAN,
            prev_scale: f32::NAN,
            prev_fov: f32::NAN,
            prev_distance: f32::NAN,
            prev_light_pitch: f32::NAN,
            prev_light_yaw: f32::NAN,
            prev_hl_u_min: f32::NAN,
        }
    }

    /// Returns true if the slot needs re-rendering for the given parameters.
    #[expect(clippy::too_many_arguments)]
    fn check_and_update(
        &mut self,
        mesh_id: u16,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        px: f32,
        py: f32,
        pz: f32,
        scale: f32,
        fov: f32,
        distance: f32,
        light_pitch: f32,
        light_yaw: f32,
        hl_u_min: f32,
    ) -> bool {
        let dirty = self.prev_mesh_id != mesh_id
            || is_dirty(self.prev_qx, qx)
            || is_dirty(self.prev_qy, qy)
            || is_dirty(self.prev_qz, qz)
            || is_dirty(self.prev_qw, qw)
            || is_dirty(self.prev_px, px)
            || is_dirty(self.prev_py, py)
            || is_dirty(self.prev_pz, pz)
            || is_dirty(self.prev_scale, scale)
            || is_dirty(self.prev_fov, fov)
            || is_dirty(self.prev_distance, distance)
            || is_dirty(self.prev_light_pitch, light_pitch)
            || is_dirty(self.prev_light_yaw, light_yaw)
            || is_dirty(self.prev_hl_u_min, hl_u_min);
        if dirty {
            self.prev_mesh_id = mesh_id;
            self.prev_qx = qx;
            self.prev_qy = qy;
            self.prev_qz = qz;
            self.prev_qw = qw;
            self.prev_px = px;
            self.prev_py = py;
            self.prev_pz = pz;
            self.prev_scale = scale;
            self.prev_fov = fov;
            self.prev_distance = distance;
            self.prev_light_pitch = light_pitch;
            self.prev_light_yaw = light_yaw;
            self.prev_hl_u_min = hl_u_min;
        }
        dirty
    }
}

// ── Uploaded mesh data ──────────────────────────────────────────────

/// GPU resources for a single registered mesh.
#[expect(missing_debug_implementations)]
pub struct UploadedMesh {
    vbo: glow::Buffer,
    ibo: glow::Buffer,
    index_count: i32,
    texture: Option<glow::Texture>,
    normal_map: Option<glow::Texture>,
    has_uvs: bool,
    has_tangents: bool,
}

// ── MeshRenderer ────────────────────────────────────────────────────

/// Offscreen GL renderer for 3D meshes using a single atlas FBO.
///
/// The atlas is a `ATLAS_W × ATLAS_H` texture divided into a grid of
/// `ATLAS_COLS × ATLAS_ROWS` slots, each `SLOT_SIZE × SLOT_SIZE` pixels.
/// Each `draw_mesh()` call targets a specific slot via `glViewport` + `glScissor`,
/// allowing multiple independent meshes to render into one FBO.
///
/// MSAA support:
/// - Desktop: MSAA renderbuffers + glBlitFramebuffer resolve to the femtovg texture.
/// - GC400 (future): GL_EXT_multisampled_render_to_texture for zero-cost resolve.
///   When available, `draw_fbo == resolve_fbo` (single FBO, no blit).
#[expect(missing_debug_implementations)]
pub struct MeshRenderer {
    program: glow::Program,
    vao: Option<glow::VertexArray>,
    /// FBO to draw into. When MSAA is enabled, this is the MSAA FBO;
    /// otherwise it's the same as `resolve_fbo`.
    draw_fbo: glow::Framebuffer,
    /// FBO with the texture registered in femtovg (always non-MSAA).
    resolve_fbo: glow::Framebuffer,
    /// Color texture attached to resolve_fbo, shared with femtovg.
    resolve_color: glow::Texture,
    /// Depth renderbuffer attached to resolve_fbo.
    resolve_depth: glow::Renderbuffer,
    /// MSAA renderbuffers (only allocated when MSAA is enabled).
    msaa_color_rb: Option<glow::Renderbuffer>,
    msaa_depth_rb: Option<glow::Renderbuffer>,
    image_id: ImageId,
    // Uniform locations
    u_mvp: glow::UniformLocation,
    u_normal_mat: glow::UniformLocation,
    u_light_dir: glow::UniformLocation,
    u_diffuse: glow::UniformLocation,
    u_normal_map: glow::UniformLocation,
    u_has_texture: glow::UniformLocation,
    u_has_normal_map: glow::UniformLocation,
    u_ambient: glow::UniformLocation,
    u_specular: glow::UniformLocation,
    u_highlight_rect: glow::UniformLocation,
    u_highlight_color: glow::UniformLocation,
    // Registered meshes
    meshes: Vec<Option<UploadedMesh>>,
    // Per-slot dirty tracking
    slots: Vec<SlotState>,
}

impl MeshRenderer {
    /// Compile shaders, create atlas FBO with depth buffer, register with femtovg.
    ///
    /// `msaa_samples`: 0 = disabled, 4 = 4× MSAA. Only effective on desktop GL.
    pub fn new(gl: &glow::Context, canvas: &mut Canvas<OpenGl>, msaa_samples: u32) -> Result<Self> {
        unsafe {
            let program = compile_program(gl)?;

            // Bind attributes before linking (ES 2.0 compatible)
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.bind_attrib_location(program, 1, "a_normal");
            gl.bind_attrib_location(program, 2, "a_uv");
            gl.bind_attrib_location(program, 3, "a_tangent");
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                bail!("mesh shader link failed: {log}");
            }

            let get_uniform = |name: &str| -> Result<glow::UniformLocation> {
                gl.get_uniform_location(program, name)
                    .ok_or_else(|| anyhow::anyhow!("missing uniform: {name}"))
            };
            let u_mvp = get_uniform("u_mvp")?;
            let u_normal_mat = get_uniform("u_normal_mat")?;
            let u_light_dir = get_uniform("u_light_dir")?;
            let u_diffuse = get_uniform("u_diffuse")?;
            let u_normal_map = get_uniform("u_normal_map")?;
            let u_has_texture = get_uniform("u_has_texture")?;
            let u_has_normal_map = get_uniform("u_has_normal_map")?;
            let u_ambient = get_uniform("u_ambient")?;
            let u_specular = get_uniform("u_specular")?;
            let u_highlight_rect = get_uniform("u_highlight_rect")?;
            let u_highlight_color = get_uniform("u_highlight_color")?;

            // VAO required on desktop GL core profile and ES 3.0+.
            // Optional on ES 2.0 (extension), so we try and skip if unavailable.
            let vao = gl.create_vertex_array().ok();

            // Resolve FBO — always non-MSAA, texture registered with femtovg
            let (resolve_fbo, resolve_color, resolve_depth) =
                create_offscreen_fbo_with_depth(gl, ATLAS_W, ATLAS_H)?;

            // MSAA FBO — optional draw target with multisampled renderbuffers.
            // After drawing, blit to resolve FBO for femtovg to sample.
            let (draw_fbo, msaa_color_rb, msaa_depth_rb) = if msaa_samples > 0 {
                match create_msaa_fbo(gl, ATLAS_W, ATLAS_H, msaa_samples) {
                    Ok((fbo, color_rb, depth_rb)) => {
                        tracing::info!("mesh MSAA enabled: {msaa_samples}× samples");
                        (fbo, Some(color_rb), Some(depth_rb))
                    }
                    Err(e) => {
                        tracing::warn!("mesh MSAA init failed, falling back: {e}");
                        (resolve_fbo, None, None)
                    }
                }
            } else {
                (resolve_fbo, None, None)
            };

            // Register FBO texture with femtovg (zero-copy).
            // PREMULTIPLIED: FBO background is (0,0,0,0) and mesh pixels are (r,g,b,1),
            // both valid premultiplied alpha.
            let image_id = canvas.create_image_from_native_texture(
                resolve_color,
                ImageInfo::new(
                    ImageFlags::PREMULTIPLIED,
                    ATLAS_W as usize,
                    ATLAS_H as usize,
                    PixelFormat::Rgba8,
                ),
            )?;

            tracing::info!(
                "mesh atlas renderer initialized ({ATLAS_W}x{ATLAS_H}, \
                 {ATLAS_COLS}x{ATLAS_ROWS} grid, slot={SLOT_SIZE}px), \
                 MSAA={msaa_samples}×, resolve_color={resolve_color:?}",
            );

            let slots = (0..MAX_SLOTS).map(|_| SlotState::new()).collect();

            Ok(Self {
                program,
                vao,
                draw_fbo,
                resolve_fbo,
                resolve_color,
                resolve_depth,
                msaa_color_rb,
                msaa_depth_rb,
                image_id,
                u_mvp,
                u_normal_mat,
                u_light_dir,
                u_diffuse,
                u_normal_map,
                u_has_texture,
                u_has_normal_map,
                u_ambient,
                u_specular,
                u_highlight_rect,
                u_highlight_color,
                meshes: Vec::new(),
                slots,
            })
        }
    }

    /// Upload mesh binary data to GPU (VBO + IBO + optional texture).
    /// Returns the mesh ID (index into the meshes vec).
    pub fn register_mesh(&mut self, gl: &glow::Context, data: &[u8]) -> u16 {
        let mesh = match parse_and_upload(gl, data) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("mesh upload failed: {e}");
                return 0;
            }
        };

        let id = self.meshes.len();
        self.meshes.push(Some(mesh));
        // Force re-render on all slots
        for slot in &mut self.slots {
            slot.prev_qx = f32::NAN;
        }
        #[expect(clippy::cast_possible_truncation)]
        {
            id as u16
        }
    }

    /// Render a mesh into an atlas slot. Returns the atlas image ID and sub-rect
    /// `(src_x, src_y, src_w, src_h)` for sampling with `draw_bitmap_subrect`.
    ///
    /// Skips GL work if the slot's parameters haven't changed (dirty check).
    #[expect(clippy::too_many_arguments, clippy::too_many_lines)]
    pub fn render(
        &mut self,
        gl: &glow::Context,
        slot_index: u8,
        mesh_id: u16,
        fov: f32,
        distance: f32,
        qx: f32,
        qy: f32,
        qz: f32,
        qw: f32,
        px: f32,
        py: f32,
        pz: f32,
        scale: f32,
        light_pitch: f32,
        light_yaw: f32,
        ambient: f32,
        specular: f32,
        hl_u_min: f32,
        hl_v_min: f32,
        hl_u_max: f32,
        hl_v_max: f32,
        hl_r: f32,
        hl_g: f32,
        hl_b: f32,
    ) -> (ImageId, f32, f32, f32, f32) {
        let si = u32::from(slot_index).min(MAX_SLOTS - 1);
        let col = si % ATLAS_COLS;
        #[expect(clippy::integer_division)]
        let row = si / ATLAS_COLS;
        let vp_x = col * SLOT_SIZE;
        let vp_y = row * SLOT_SIZE;
        let subrect = (
            self.image_id,
            vp_x as f32,
            vp_y as f32,
            SLOT_SIZE as f32,
            SLOT_SIZE as f32,
        );

        let mesh_idx = mesh_id as usize;
        if mesh_idx >= self.meshes.len() || self.meshes[mesh_idx].is_none() {
            return subrect;
        }

        // Per-slot dirty check
        if !self.slots[si as usize].check_and_update(
            mesh_id,
            qx,
            qy,
            qz,
            qw,
            px,
            py,
            pz,
            scale,
            fov,
            distance,
            light_pitch,
            light_yaw,
            hl_u_min,
        ) {
            return subrect;
        }

        let mesh = self.meshes[mesh_idx]
            .as_ref()
            .expect("BUG: mesh was None after check");

        // Compute MVP matrix from quaternion (aspect ratio is 1:1 for square slots)
        let rotation = quat_to_mat3([qx, qy, qz, qw]);
        let mvp = compute_mvp(&rotation, [px, py, pz], scale, fov, 1.0, distance);
        let normal_mat = rotation; // For uniform scale, normal matrix = rotation matrix

        // Compute light direction
        let (lx, ly, lz) = if light_pitch.is_nan() {
            (0.0, 0.0, 0.0)
        } else {
            let lp = light_pitch.to_radians();
            let ly_rad = light_yaw.to_radians();
            (lp.cos() * ly_rad.sin(), lp.sin(), lp.cos() * ly_rad.cos())
        };

        let sw = SLOT_SIZE as i32;
        let sh = SLOT_SIZE as i32;
        let vx = vp_x as i32;
        let vy = vp_y as i32;

        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.draw_fbo));
            gl.viewport(vx, vy, sw, sh);

            // Scissor limits clear to this slot only
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(vx, vy, sw, sh);

            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LESS);
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
            // Y-negate in projection flips triangle winding
            gl.front_face(glow::CW);
            gl.disable(glow::STENCIL_TEST);
            gl.color_mask(true, true, true, true);

            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            gl.use_program(Some(self.program));

            // Set uniforms
            gl.uniform_matrix_4_f32_slice(Some(&self.u_mvp), false, &mvp);
            gl.uniform_matrix_3_f32_slice(
                Some(&self.u_normal_mat),
                false,
                &flatten_mat3(&normal_mat),
            );
            gl.uniform_3_f32(Some(&self.u_light_dir), lx, ly, lz);
            gl.uniform_1_f32(Some(&self.u_ambient), ambient);
            gl.uniform_1_f32(Some(&self.u_specular), specular);

            // Bind diffuse texture (unit 0)
            gl.active_texture(glow::TEXTURE0);
            if let Some(tex) = mesh.texture {
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.uniform_1_f32(Some(&self.u_has_texture), 1.0);
            } else {
                gl.uniform_1_f32(Some(&self.u_has_texture), 0.0);
            }
            gl.uniform_1_i32(Some(&self.u_diffuse), 0);

            // Bind normal map (unit 1, optional)
            gl.active_texture(glow::TEXTURE1);
            if let Some(nmap) = mesh.normal_map {
                gl.bind_texture(glow::TEXTURE_2D, Some(nmap));
                gl.uniform_1_f32(Some(&self.u_has_normal_map), 1.0);
            } else {
                gl.uniform_1_f32(Some(&self.u_has_normal_map), 0.0);
            }
            gl.uniform_1_i32(Some(&self.u_normal_map), 1);

            // Highlight rect: driven by host-side fields
            gl.uniform_4_f32(
                Some(&self.u_highlight_rect),
                hl_u_min,
                hl_v_min,
                hl_u_max,
                hl_v_max,
            );
            gl.uniform_3_f32(Some(&self.u_highlight_color), hl_r, hl_g, hl_b);

            // Bind VAO / VBO / IBO
            if let Some(vao) = self.vao {
                gl.bind_vertex_array(Some(vao));
            }

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(mesh.vbo));

            // Stride in bytes for the dequantized float VBO:
            //   pos(3) + normal(3) = 24 bytes (no UVs)
            //   + uv(2) = 32 bytes (with UVs)
            //   + tangent(4) = 48 bytes (with UVs + tangents)
            let stride = match (mesh.has_uvs, mesh.has_tangents) {
                (false, _) => 24,
                (true, false) => 32,
                (true, true) => 48,
            };

            // a_pos: 3x float at offset 0
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);

            // a_normal: 3x float at offset 12
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 12);

            // a_uv: 2x float at offset 24 (optional)
            if mesh.has_uvs {
                gl.enable_vertex_attrib_array(2);
                gl.vertex_attrib_pointer_f32(2, 2, glow::FLOAT, false, stride, 24);
            }

            // a_tangent: 4x float at offset 32 (optional, requires UVs)
            if mesh.has_tangents {
                gl.enable_vertex_attrib_array(3);
                gl.vertex_attrib_pointer_f32(3, 4, glow::FLOAT, false, stride, 32);
            }

            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(mesh.ibo));
            gl.draw_elements(glow::TRIANGLES, mesh.index_count, glow::UNSIGNED_SHORT, 0);

            let err = gl.get_error();
            if err != glow::NO_ERROR {
                tracing::error!("GL error after mesh draw (slot {si}): 0x{err:04X}");
            }

            gl.disable_vertex_attrib_array(0);
            gl.disable_vertex_attrib_array(1);
            if mesh.has_uvs {
                gl.disable_vertex_attrib_array(2);
            }
            if mesh.has_tangents {
                gl.disable_vertex_attrib_array(3);
            }
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::SCISSOR_TEST);
            gl.front_face(glow::CCW);
            if self.vao.is_some() {
                gl.bind_vertex_array(None);
            }

            // MSAA resolve: blit only this slot's region
            if self.msaa_color_rb.is_some() {
                let x0 = vx;
                let y0 = vy;
                let x1 = vx + sw;
                let y1 = vy + sh;
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(self.draw_fbo));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(self.resolve_fbo));
                gl.blit_framebuffer(
                    x0,
                    y0,
                    x1,
                    y1,
                    x0,
                    y0,
                    x1,
                    y1,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }

        subrect
    }

    /// The femtovg image backed by the atlas FBO texture.
    #[must_use]
    pub fn image_id(&self) -> ImageId {
        self.image_id
    }

    /// Atlas texture dimensions (for `draw_bitmap_subrect`).
    #[must_use]
    pub fn atlas_size(&self) -> (f32, f32) {
        (ATLAS_W as f32, ATLAS_H as f32)
    }

    /// Release GL resources. Call before dropping the GL context.
    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.resolve_fbo);
            gl.delete_texture(self.resolve_color);
            gl.delete_renderbuffer(self.resolve_depth);
            if let Some(color_rb) = self.msaa_color_rb {
                gl.delete_framebuffer(self.draw_fbo);
                gl.delete_renderbuffer(color_rb);
            }
            if let Some(depth_rb) = self.msaa_depth_rb {
                gl.delete_renderbuffer(depth_rb);
            }
            gl.delete_program(self.program);
            for m in self.meshes.iter().flatten() {
                gl.delete_buffer(m.vbo);
                gl.delete_buffer(m.ibo);
                if let Some(tex) = m.texture {
                    gl.delete_texture(tex);
                }
                if let Some(nmap) = m.normal_map {
                    gl.delete_texture(nmap);
                }
            }
        }
    }
}

// ── Mesh parsing and GPU upload ─────────────────────────────────────

/// Parse the optimized binary format and upload VBO/IBO/texture to GL.
#[expect(clippy::too_many_lines)]
fn parse_and_upload(gl: &glow::Context, data: &[u8]) -> Result<UploadedMesh> {
    if data.len() < HEADER_SIZE {
        bail!("mesh data too small: {} bytes", data.len());
    }

    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != MESH_MAGIC {
        bail!("invalid mesh magic: 0x{magic:08X}");
    }

    let vertex_count = read_u32(data, 4) as usize;
    let index_count = read_u32(data, 8) as usize;
    let vertex_offset = read_u32(data, 12) as usize;
    let index_offset = read_u32(data, 16) as usize;
    let texture_offset = read_u32(data, 20) as usize;
    let tex_width = u32::from(read_u16(data, 24));
    let tex_height = u32::from(read_u16(data, 26));
    let tex_format = data[28];
    let flags = data[29];

    // Normal map info (offsets 30-37 in header)
    let nmap_offset = read_u32(data, 30) as usize;
    let nmap_width = u32::from(read_u16(data, 34));
    let nmap_height = u32::from(read_u16(data, 36));

    let has_texture = flags & FLAG_HAS_TEXTURE != 0;
    let has_uvs = flags & FLAG_HAS_UVS != 0;
    let has_tangents = flags & FLAG_HAS_TANGENTS != 0;
    let has_normal_map = flags & FLAG_HAS_NORMAL_MAP != 0;

    // Read AABB (6 floats after header)
    let aabb_offset = HEADER_SIZE;
    let aabb_min = [
        read_f32(data, aabb_offset),
        read_f32(data, aabb_offset + 4),
        read_f32(data, aabb_offset + 8),
    ];
    let aabb_max = [
        read_f32(data, aabb_offset + 12),
        read_f32(data, aabb_offset + 16),
        read_f32(data, aabb_offset + 20),
    ];

    // Dequantize vertices into float VBO
    // Layout: [pos(3), normal(3), uv(2)?, tangent(4)?]
    let floats_per_vertex = match (has_uvs, has_tangents) {
        (false, _) => 6,
        (true, false) => 8,
        (true, true) => 12,
    };
    let quantized_vertex_size = match (has_uvs, has_tangents) {
        (false, _) => 10,
        (true, false) => 14,
        (true, true) => 22,
    };
    let mut vertex_floats = Vec::with_capacity(vertex_count * floats_per_vertex);

    for i in 0..vertex_count {
        let base = vertex_offset + i * quantized_vertex_size;

        // Dequantize position from i16
        let qx = read_i16(data, base);
        let qy = read_i16(data, base + 2);
        let qz = read_i16(data, base + 4);
        vertex_floats.push(dequantize_position(qx, aabb_min[0], aabb_max[0]));
        vertex_floats.push(dequantize_position(qy, aabb_min[1], aabb_max[1]));
        vertex_floats.push(dequantize_position(qz, aabb_min[2], aabb_max[2]));

        // Unpack normal from 10/10/10/2
        let packed_normal = read_u32(data, base + 6);
        let (nx, ny, nz) = unpack_normal_10_10_10_2(packed_normal);
        vertex_floats.push(nx);
        vertex_floats.push(ny);
        vertex_floats.push(nz);

        if has_uvs {
            let qu = read_u16(data, base + 10);
            let qv = read_u16(data, base + 12);
            vertex_floats.push(f32::from(qu) / 65_535.0);
            vertex_floats.push(f32::from(qv) / 65_535.0);

            if has_tangents {
                // Dequantize tangent xyzw from i16 (range -32767..32767 → -1..1)
                let dq = |off: usize| f32::from(read_i16(data, off)) / 32_767.0;
                vertex_floats.push(dq(base + 14));
                vertex_floats.push(dq(base + 16));
                vertex_floats.push(dq(base + 18));
                vertex_floats.push(dq(base + 20));
            }
        }
    }

    // Upload VBO
    let vbo = unsafe {
        let vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let bytes: &[u8] = std::slice::from_raw_parts(
            vertex_floats.as_ptr().cast::<u8>(),
            vertex_floats.len() * 4,
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        vbo
    };

    // Upload IBO (indices are u16, already in the right format)
    let ibo = unsafe {
        let ibo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
        let index_bytes = &data[index_offset..index_offset + index_count * 2];
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
        ibo
    };

    let is_etc1 = tex_format == bmc_wasm_protocol::mesh::TextureFormat::Etc1 as u8;

    // Upload texture (optional)
    let texture = if has_texture && tex_width > 0 && tex_height > 0 {
        let tex_size = if is_etc1 {
            etc1_data_size(tex_width, tex_height)
        } else {
            (tex_width * tex_height * 4) as usize
        };
        if texture_offset + tex_size <= data.len() {
            let tex_data = &data[texture_offset..texture_offset + tex_size];
            Some(upload_texture(
                gl, tex_width, tex_height, tex_data, is_etc1,
            )?)
        } else {
            tracing::warn!("mesh texture data truncated");
            None
        }
    } else {
        None
    };

    // Upload normal map (optional, same format as albedo)
    let normal_map = if has_normal_map && nmap_width > 0 && nmap_height > 0 {
        let nmap_size = if is_etc1 {
            etc1_data_size(nmap_width, nmap_height)
        } else {
            (nmap_width * nmap_height * 4) as usize
        };
        if nmap_offset + nmap_size <= data.len() {
            let nmap_data = &data[nmap_offset..nmap_offset + nmap_size];
            Some(upload_texture(
                gl,
                nmap_width,
                nmap_height,
                nmap_data,
                is_etc1,
            )?)
        } else {
            tracing::warn!("mesh normal map data truncated");
            None
        }
    } else {
        None
    };

    #[expect(clippy::integer_division)]
    let triangle_count = index_count / 3;
    tracing::info!(
        "mesh uploaded: {vertex_count} vertices, {triangle_count} triangles, \
         texture={}, normal_map={}",
        texture.is_some(),
        normal_map.is_some()
    );

    Ok(UploadedMesh {
        vbo,
        ibo,
        #[expect(clippy::cast_possible_truncation)]
        index_count: index_count as i32,
        texture,
        normal_map,
        has_uvs,
        has_tangents,
    })
}

/// GL_ETC1_RGB8_OES (from GL_OES_compressed_ETC1_RGB8_texture).
const GL_ETC1_RGB8_OES: u32 = 0x8D64;

/// Compute ETC1 compressed data size: 8 bytes per 4×4 block.
fn etc1_data_size(width: u32, height: u32) -> usize {
    (width.div_ceil(4) * height.div_ceil(4) * 8) as usize
}

fn upload_texture(
    gl: &glow::Context,
    width: u32,
    height: u32,
    data: &[u8],
    etc1: bool,
) -> Result<glow::Texture> {
    unsafe {
        let texture = gl.create_texture().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        if etc1 {
            gl.compressed_tex_image_2d(
                glow::TEXTURE_2D,
                0,
                GL_ETC1_RGB8_OES as i32,
                width as i32,
                height as i32,
                0,
                #[expect(clippy::cast_possible_truncation)] // texture data ≤ 1MB
                {
                    data.len() as i32
                },
                data,
            );
        } else {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(data)),
            );
        }
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);
        Ok(texture)
    }
}

// ── Matrix math ─────────────────────────────────────────────────────

fn quat_to_mat3(q: [f32; 4]) -> [[f32; 3]; 3] {
    let m = glam::Mat3::from_quat(glam::Quat::from_xyzw(q[0], q[1], q[2], q[3]));
    // Each element is a glam column vector: r[col][row].
    [m.x_axis.into(), m.y_axis.into(), m.z_axis.into()]
}

fn compute_mvp(
    rotation: &[[f32; 3]; 3],
    position: [f32; 3],
    scale: f32,
    fov_deg: f32,
    aspect: f32,
    distance: f32,
) -> [f32; 16] {
    // Model matrix: translate(position) * rotate(quat) * scale(s)
    // View matrix: translate(0, 0, -distance)
    // Projection: perspective

    let near = 0.1_f32;
    let far = 100.0_f32;
    let fov_rad = fov_deg.to_radians();
    let f = 1.0 / (fov_rad / 2.0).tan();

    // Perspective projection (column-major)
    let proj = [
        f / aspect,
        0.0,
        0.0,
        0.0,
        0.0,
        -f,
        0.0,
        0.0, // Negate Y to flip for FBO→femtovg (same as sphere)
        0.0,
        0.0,
        (far + near) / (near - far),
        -1.0,
        0.0,
        0.0,
        (2.0 * far * near) / (near - far),
        0.0,
    ];

    // Model-view matrix (column-major): view * translate * rotate * scale
    // r[col][row] from quat_to_mat3 — columns of R go into columns of MV.
    let r = rotation;
    let mv = [
        r[0][0] * scale,
        r[0][1] * scale,
        r[0][2] * scale,
        0.0,
        r[1][0] * scale,
        r[1][1] * scale,
        r[1][2] * scale,
        0.0,
        r[2][0] * scale,
        r[2][1] * scale,
        r[2][2] * scale,
        0.0,
        position[0],
        position[1],
        position[2] - distance,
        1.0,
    ];

    // MVP = proj * mv (4x4 column-major multiply)
    mat4_mul(&proj, &mv)
}

fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let ma = glam::Mat4::from_cols_array(a);
    let mb = glam::Mat4::from_cols_array(b);
    (ma * mb).to_cols_array()
}

fn flatten_mat3(m: &[[f32; 3]; 3]) -> [f32; 9] {
    // m[col][row] → column-major flat array for GL uniform
    [
        m[0][0], m[0][1], m[0][2], m[1][0], m[1][1], m[1][2], m[2][0], m[2][1], m[2][2],
    ]
}

// ── FBO creation ────────────────────────────────────────────────────

unsafe fn create_offscreen_fbo_with_depth(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<(glow::Framebuffer, glow::Texture, glow::Renderbuffer)> {
    unsafe {
        // Color texture
        let texture = gl.create_texture().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.bind_texture(glow::TEXTURE_2D, None);

        // Depth renderbuffer
        let depth_rb = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rb));
        gl.renderbuffer_storage(
            glow::RENDERBUFFER,
            glow::DEPTH_COMPONENT16,
            width as i32,
            height as i32,
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);

        // FBO
        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth_rb),
        );

        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);

        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(texture);
            gl.delete_renderbuffer(depth_rb);
            bail!("mesh FBO incomplete: status 0x{status:04X}");
        }

        Ok((fbo, texture, depth_rb))
    }
}

/// Create an MSAA FBO with multisampled renderbuffers for color + depth.
///
/// This is the desktop GL path — uses `glRenderbufferStorageMultisample`.
/// The returned FBO is the draw target; after drawing, blit to the resolve FBO.
unsafe fn create_msaa_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
    samples: u32,
) -> Result<(glow::Framebuffer, glow::Renderbuffer, glow::Renderbuffer)> {
    unsafe {
        // MSAA color renderbuffer
        let color_rb = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color_rb));
        gl.renderbuffer_storage_multisample(
            glow::RENDERBUFFER,
            samples as i32,
            glow::RGBA8,
            width as i32,
            height as i32,
        );

        // MSAA depth renderbuffer
        let depth_rb = gl
            .create_renderbuffer()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rb));
        gl.renderbuffer_storage_multisample(
            glow::RENDERBUFFER,
            samples as i32,
            glow::DEPTH_COMPONENT16,
            width as i32,
            height as i32,
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);

        // FBO
        let fbo = gl
            .create_framebuffer()
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(color_rb),
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth_rb),
        );

        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);

        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_renderbuffer(color_rb);
            gl.delete_renderbuffer(depth_rb);
            bail!("mesh MSAA FBO incomplete: status 0x{status:04X}");
        }

        Ok((fbo, color_rb, depth_rb))
    }
}

// ── Shader compilation ──────────────────────────────────────────────

unsafe fn compile_program(gl: &glow::Context) -> Result<glow::Program> {
    let vs = unsafe { compile_shader(gl, glow::VERTEX_SHADER, VERTEX_SHADER) }?;
    let fs = match unsafe { compile_shader(gl, glow::FRAGMENT_SHADER, FRAGMENT_SHADER) } {
        Ok(s) => s,
        Err(e) => {
            unsafe { gl.delete_shader(vs) };
            return Err(e);
        }
    };

    let program = unsafe { gl.create_program() }.map_err(|e| anyhow::anyhow!("{e}"))?;
    unsafe {
        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
    }

    Ok(program)
}

unsafe fn compile_shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader> {
    let shader = unsafe { gl.create_shader(kind) }.map_err(|e| anyhow::anyhow!("{e}"))?;
    unsafe {
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
    }
    if !unsafe { gl.get_shader_compile_status(shader) } {
        let log = unsafe { gl.get_shader_info_log(shader) };
        unsafe { gl.delete_shader(shader) };
        let kind_name = if kind == glow::VERTEX_SHADER {
            "vertex"
        } else {
            "fragment"
        };
        bail!("mesh {kind_name} shader compile failed: {log}");
    }
    Ok(shader)
}

// ── Helpers ─────────────────────────────────────────────────────────

fn is_dirty(old: f32, new: f32) -> bool {
    old.is_nan() || (old - new).abs() > DIRTY_EPSILON
}

fn dequantize_position(q: i16, min: f32, max: f32) -> f32 {
    let range = max - min;
    if range < 1e-8 {
        return min;
    }
    let t = (f32::from(q) + 32_767.0) / 65_534.0; // 0..1
    min + t * range
}

fn unpack_normal_10_10_10_2(packed: u32) -> (f32, f32, f32) {
    let from_10bit = |bits: u32| -> f32 {
        // Sign-extend 10-bit to i32
        let signed = if bits & 0x200 != 0 {
            (bits | 0xFFFF_FC00) as i32
        } else {
            bits as i32
        };
        signed as f32 / 511.0
    };
    let x = from_10bit(packed & 0x3FF);
    let y = from_10bit((packed >> 10) & 0x3FF);
    let z = from_10bit((packed >> 20) & 0x3FF);
    (x, y, z)
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_i16(data: &[u8], offset: usize) -> i16 {
    i16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_f32(data: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}
