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
    FLAG_HAS_NORMAL_MAP, FLAG_HAS_TANGENTS, FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE,
    MAX_TEXTURE_SIZE, MAX_TRIANGLES, MAX_VERTICES, MESH_MAGIC, TextureFormat,
};

/// AABB record (6 × `f32`) immediately follows the binary header.
const AABB_SIZE: usize = 24;

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
// MSDF mode: when > 0.5, sample u_diffuse as a multi-channel SDF, take the
// median of RGB, smoothstep to coverage, and lerp body→label.
uniform float u_is_msdf;
uniform vec3 u_body_color;
uniform vec3 u_label_color;
// UV-rect highlight: brightens pixels within the rect with the given color.
// u_highlight_rect = (u_min, v_min, u_max, v_max), all < 0 = disabled.
uniform vec4 u_highlight_rect;
uniform vec3 u_highlight_color;

varying vec3 v_normal;
varying vec2 v_uv;
varying vec3 v_tangent;
varying vec3 v_bitangent;

float msdf_median(vec3 sdf) {
    return max(min(sdf.r, sdf.g), min(max(sdf.r, sdf.g), sdf.b));
}

void main() {
    vec3 albedo;
    if (u_is_msdf > 0.5) {
        // Multi-channel SDF: median across RGB, threshold at 0.5 with a
        // narrow smoothstep window for sub-texel antialiasing. fwidth would
        // be more correct but is unavailable on GLES 2 without an extension.
        vec3 sdf = texture2D(u_diffuse, v_uv).rgb;
        float coverage = smoothstep(0.5 - 0.04, 0.5 + 0.04, msdf_median(sdf));
        albedo = mix(u_body_color, u_label_color, coverage);
    } else if (u_has_texture > 0.5) {
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

/// The atlas dimensions are cast unchecked to `i32` at every `glViewport`
/// call site. Lock the invariant in: anything that bumps `ATLAS_COLS *
/// SLOT_SIZE` (or rows) past `i32::MAX / 2` would silently produce negative
/// viewport arguments. The 2× margin keeps a comfortable safety buffer.
const _: () = assert!(ATLAS_W.saturating_mul(2) < i32::MAX as u32);
const _: () = assert!(ATLAS_H.saturating_mul(2) < i32::MAX as u32);
/// Sentinel returned when mesh registration fails.
const INVALID_MESH_ID: u16 = 0;

// ── Mesh draw arguments ─────────────────────────────────────────────

/// Camera + 3D transform parameters for a mesh draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshTransform {
    pub fov: f32,
    pub distance: f32,
    /// Orientation quaternion `(x, y, z, w)`.
    pub quat: [f32; 4],
    /// Offset relative to the draw rect centre in mesh-local units.
    pub position: [f32; 3],
    pub scale: f32,
}

/// Directional lighting. `pitch == NaN` is the sentinel for "lighting
/// disabled" (matches `MeshView::light: None` in the SDK).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshLighting {
    pub pitch: f32,
    pub yaw: f32,
    pub ambient: f32,
    pub specular: f32,
}

/// Optional UV-rect highlight tint. `u_min == NaN` is the sentinel for
/// "no highlight".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshHighlight {
    pub u_min: f32,
    pub v_min: f32,
    pub u_max: f32,
    pub v_max: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

/// Bundle of every per-draw mesh parameter passed through the renderer
/// trait, the `DrawCommand::Mesh` variant, and `MeshRenderer::render`.
/// Grouping them in one struct avoids the wide-tuple drift that would
/// otherwise force every signature change to be edited at six sites, and
/// lets `SlotState` dirty-check every field by definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshDrawArgs {
    pub transform: MeshTransform,
    pub lighting: MeshLighting,
    pub highlight: MeshHighlight,
}

impl MeshDrawArgs {
    /// Field-by-field dirty check. Treats `NaN` either-side as "changed" so
    /// `Some(...) ↔ None` lighting / highlight transitions invalidate the
    /// cached slot.
    fn dirty_against(&self, prev: &Self) -> bool {
        let t = &self.transform;
        let pt = &prev.transform;
        if is_dirty(pt.fov, t.fov)
            || is_dirty(pt.distance, t.distance)
            || is_dirty(pt.scale, t.scale)
        {
            return true;
        }
        for i in 0..4 {
            if is_dirty(pt.quat[i], t.quat[i]) {
                return true;
            }
        }
        for i in 0..3 {
            if is_dirty(pt.position[i], t.position[i]) {
                return true;
            }
        }
        let l = &self.lighting;
        let pl = &prev.lighting;
        if is_dirty(pl.pitch, l.pitch)
            || is_dirty(pl.yaw, l.yaw)
            || is_dirty(pl.ambient, l.ambient)
            || is_dirty(pl.specular, l.specular)
        {
            return true;
        }
        let h = &self.highlight;
        let ph = &prev.highlight;
        is_dirty(ph.u_min, h.u_min)
            || is_dirty(ph.v_min, h.v_min)
            || is_dirty(ph.u_max, h.u_max)
            || is_dirty(ph.v_max, h.v_max)
            || is_dirty(ph.r, h.r)
            || is_dirty(ph.g, h.g)
            || is_dirty(ph.b, h.b)
    }
}

// ── Per-slot dirty state ────────────────────────────────────────────

/// Dirty-check state for a single atlas slot. Stores the last-rendered
/// `(mesh_id, args)` pair; a `None` `prev_args` represents "never rendered
/// yet" and forces a first-frame draw.
#[derive(Clone)]
struct SlotState {
    prev_mesh_id: u16,
    prev_args: Option<MeshDrawArgs>,
}

impl SlotState {
    fn new() -> Self {
        Self {
            prev_mesh_id: u16::MAX,
            prev_args: None,
        }
    }

    /// Returns true if the slot needs re-rendering for the given parameters.
    fn check_and_update(&mut self, mesh_id: u16, args: &MeshDrawArgs) -> bool {
        let dirty = self.prev_mesh_id != mesh_id
            || self
                .prev_args
                .as_ref()
                .is_none_or(|prev| args.dirty_against(prev));
        if dirty {
            self.prev_mesh_id = mesh_id;
            self.prev_args = Some(*args);
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
    /// `true` when the texture is an MSDF (multi-channel signed distance
    /// field). The fragment shader takes the median of RGB, smoothsteps to
    /// coverage, and lerps `body_color` ↔ `label_color`. ETC1 path stays
    /// fully supported alongside this.
    is_msdf: bool,
    /// Body color used by the MSDF shader path. Ignored for ETC1 meshes.
    body_color: [f32; 4],
    /// Label color used by the MSDF shader path. Ignored for ETC1 meshes.
    label_color: [f32; 4],
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
    u_is_msdf: glow::UniformLocation,
    u_body_color: glow::UniformLocation,
    u_label_color: glow::UniformLocation,
    u_ambient: glow::UniformLocation,
    u_specular: glow::UniformLocation,
    u_highlight_rect: glow::UniformLocation,
    u_highlight_color: glow::UniformLocation,
    // Registered meshes
    meshes: Vec<Option<UploadedMesh>>,
    // Per-slot dirty tracking
    slots: Vec<SlotState>,
    #[cfg(feature = "profiling")]
    setup_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    draw_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    blit_w: ii_stopwatch::StopWatch,
    #[cfg(feature = "profiling")]
    render_every: ii_stopwatch::Every,
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
            let u_is_msdf = get_uniform("u_is_msdf")?;
            let u_body_color = get_uniform("u_body_color")?;
            let u_label_color = get_uniform("u_label_color")?;
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
                u_is_msdf,
                u_body_color,
                u_label_color,
                u_ambient,
                u_specular,
                u_highlight_rect,
                u_highlight_color,
                meshes: Vec::new(),
                slots,
                #[cfg(feature = "profiling")]
                setup_w: ii_stopwatch::StopWatch::default(),
                #[cfg(feature = "profiling")]
                draw_w: ii_stopwatch::StopWatch::default(),
                #[cfg(feature = "profiling")]
                blit_w: ii_stopwatch::StopWatch::default(),
                #[cfg(feature = "profiling")]
                render_every: ii_stopwatch::Every::new(std::time::Duration::from_secs(5)),
            })
        }
    }

    /// Upload mesh binary data to GPU (VBO + IBO + optional texture).
    /// Returns an opaque non-zero mesh ID.
    pub fn register_mesh(&mut self, gl: &glow::Context, data: &[u8]) -> u16 {
        #[cfg(feature = "profiling")]
        let profile_before = profile::RegisterMeshProbe::start();

        let mesh = match parse_and_upload(gl, data) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("mesh upload failed: {e}");
                return INVALID_MESH_ID;
            }
        };

        let Some(id) = mesh_id_from_storage_index(self.meshes.len()) else {
            tracing::error!("mesh registration failed: id space exhausted");
            return INVALID_MESH_ID;
        };
        self.meshes.push(Some(mesh));
        // No slot invalidation needed: `mesh_id` is fresh (one-based index of
        // the just-pushed entry), so no existing slot's `prev_mesh_id` can
        // collide. `SlotState::check_and_update` already triggers a redraw
        // when a slot is first bound to this mesh via the `prev_mesh_id !=
        // mesh_id` check.

        #[cfg(feature = "profiling")]
        profile_before.finish(id, data.len());

        id
    }

    /// Render a mesh into an atlas slot. Returns the atlas image ID and sub-rect
    /// `(src_x, src_y, src_w, src_h)` for sampling with `draw_bitmap_subrect`.
    ///
    /// Skips GL work if the slot's parameters haven't changed (dirty check).
    #[expect(clippy::too_many_lines)]
    pub fn render(
        &mut self,
        gl: &glow::Context,
        slot_index: u8,
        mesh_id: u16,
        args: &MeshDrawArgs,
    ) -> (ImageId, f32, f32, f32, f32) {
        let si = u32::from(slot_index);
        if si >= MAX_SLOTS {
            warn_slot_overflow_once(si);
            // Return a degenerate sub-rect so the caller's
            // `draw_bitmap_subrect` no-ops (sw == sh == 0). The image_id is
            // still valid; only the geometry is suppressed.
            return (self.image_id, 0.0, 0.0, 0.0, 0.0);
        }
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

        let Some(mesh_idx) = mesh_id_to_storage_index(mesh_id) else {
            return subrect;
        };
        if mesh_idx >= self.meshes.len() || self.meshes[mesh_idx].is_none() {
            return subrect;
        }

        // Per-slot dirty check
        if !self.slots[si as usize].check_and_update(mesh_id, args) {
            return subrect;
        }

        let mesh = self.meshes[mesh_idx]
            .as_ref()
            .expect("BUG: mesh was None after check");

        let MeshTransform {
            fov,
            distance,
            quat,
            position,
            scale,
        } = args.transform;
        let MeshLighting {
            pitch: light_pitch,
            yaw: light_yaw,
            ambient,
            specular,
        } = args.lighting;
        let MeshHighlight {
            u_min: hl_u_min,
            v_min: hl_v_min,
            u_max: hl_u_max,
            v_max: hl_v_max,
            r: hl_r,
            g: hl_g,
            b: hl_b,
        } = args.highlight;

        // Compute MVP matrix from quaternion (aspect ratio is 1:1 for square slots)
        let rotation = quat_to_mat3(quat);
        let mvp = compute_mvp(&rotation, position, scale, fov, 1.0, distance);
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
            ii_stopwatch::stopwatch_start!(self.setup_w);
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

            // MSDF mode + body/label colors (used only when is_msdf == true)
            gl.uniform_1_f32(Some(&self.u_is_msdf), if mesh.is_msdf { 1.0 } else { 0.0 });
            gl.uniform_3_f32(
                Some(&self.u_body_color),
                mesh.body_color[0],
                mesh.body_color[1],
                mesh.body_color[2],
            );
            gl.uniform_3_f32(
                Some(&self.u_label_color),
                mesh.label_color[0],
                mesh.label_color[1],
                mesh.label_color[2],
            );

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
            ii_stopwatch::stopwatch_stop!(self.setup_w);

            ii_stopwatch::stopwatch_start!(self.draw_w);
            gl.draw_elements(glow::TRIANGLES, mesh.index_count, glow::UNSIGNED_SHORT, 0);
            ii_stopwatch::stopwatch_stop!(self.draw_w);

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
                ii_stopwatch::stopwatch_start!(self.blit_w);
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
                ii_stopwatch::stopwatch_stop!(self.blit_w);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }

        #[cfg(feature = "profiling")]
        if ii_stopwatch::every_expired!(self.render_every) {
            tracing::info!(
                target: crate::profile::TARGET,
                "mesh_render setup={setup} draw={draw} blit={blit}",
                setup = self.setup_w,
                draw = self.draw_w,
                blit = self.blit_w,
            );
            ii_stopwatch::stopwatch_reset!(self.setup_w);
            ii_stopwatch::stopwatch_reset!(self.draw_w);
            ii_stopwatch::stopwatch_reset!(self.blit_w);
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

    /// Release every FemtoVG and GL resource owned by the renderer. Consumes
    /// `self` so callers cannot accidentally double-delete; mirrors the
    /// pattern used by `SphereRenderer::destroy`.
    pub fn destroy(self, gl: &glow::Context, canvas: &mut Canvas<OpenGl>) {
        canvas.delete_image(self.image_id);
        unsafe {
            if let Some(vao) = self.vao {
                gl.delete_vertex_array(vao);
            }
            gl.delete_framebuffer(self.resolve_fbo);
            // `draw_fbo == resolve_fbo` when MSAA is off, so only delete it
            // separately when the two are distinct framebuffers. Tying this
            // to the FBO identity rather than to `msaa_color_rb.is_some()`
            // avoids a leak if the two MSAA fields ever drift out of sync.
            if self.draw_fbo != self.resolve_fbo {
                gl.delete_framebuffer(self.draw_fbo);
            }
            gl.delete_texture(self.resolve_color);
            gl.delete_renderbuffer(self.resolve_depth);
            if let Some(color_rb) = self.msaa_color_rb {
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

/// RAII guard that deletes a freshly-created GL buffer on drop unless
/// `defuse` is called first. Used to avoid leaking already-allocated buffers
/// when a later `?` aborts `parse_and_upload`.
struct BufferGuard<'a> {
    gl: &'a glow::Context,
    buf: Option<glow::Buffer>,
}

impl<'a> BufferGuard<'a> {
    fn new(gl: &'a glow::Context, buf: glow::Buffer) -> Self {
        Self { gl, buf: Some(buf) }
    }
    fn defuse(mut self) -> glow::Buffer {
        self.buf.take().expect("BUG: BufferGuard already defused")
    }
}

impl Drop for BufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            unsafe {
                self.gl.delete_buffer(buf);
            }
        }
    }
}

/// RAII counterpart of `BufferGuard` for textures.
struct TextureGuard<'a> {
    gl: &'a glow::Context,
    tex: Option<glow::Texture>,
}

impl<'a> TextureGuard<'a> {
    fn new(gl: &'a glow::Context, tex: glow::Texture) -> Self {
        Self { gl, tex: Some(tex) }
    }
    fn defuse(mut self) -> glow::Texture {
        self.tex.take().expect("BUG: TextureGuard already defused")
    }
}

impl Drop for TextureGuard<'_> {
    fn drop(&mut self) {
        if let Some(tex) = self.tex.take() {
            unsafe {
                self.gl.delete_texture(tex);
            }
        }
    }
}

/// RAII counterpart for renderbuffers.
struct RenderbufferGuard<'a> {
    gl: &'a glow::Context,
    rb: Option<glow::Renderbuffer>,
}

impl<'a> RenderbufferGuard<'a> {
    fn new(gl: &'a glow::Context, rb: glow::Renderbuffer) -> Self {
        Self { gl, rb: Some(rb) }
    }
    fn defuse(mut self) -> glow::Renderbuffer {
        self.rb
            .take()
            .expect("BUG: RenderbufferGuard already defused")
    }
}

impl Drop for RenderbufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(rb) = self.rb.take() {
            unsafe {
                self.gl.delete_renderbuffer(rb);
            }
        }
    }
}

/// RAII counterpart for framebuffers.
struct FramebufferGuard<'a> {
    gl: &'a glow::Context,
    fbo: Option<glow::Framebuffer>,
}

impl<'a> FramebufferGuard<'a> {
    fn new(gl: &'a glow::Context, fbo: glow::Framebuffer) -> Self {
        Self { gl, fbo: Some(fbo) }
    }
    fn defuse(mut self) -> glow::Framebuffer {
        self.fbo
            .take()
            .expect("BUG: FramebufferGuard already defused")
    }
}

impl Drop for FramebufferGuard<'_> {
    fn drop(&mut self) {
        if let Some(fbo) = self.fbo.take() {
            unsafe {
                self.gl.delete_framebuffer(fbo);
            }
        }
    }
}

/// Header fields that have been bounds-checked against `data.len()` and the
/// protocol limits. After construction every offset/length combination is
/// known to fit inside `data`, so subsequent reads can use unchecked indexing
/// without risk of panicking on attacker-controlled input.
#[derive(Debug)]
struct ValidatedHeader {
    vertex_count: usize,
    index_count: usize,
    vertex_offset: usize,
    index_offset: usize,
    texture_offset: usize,
    tex_width: u32,
    tex_height: u32,
    tex_format: u8,
    nmap_offset: usize,
    nmap_width: u32,
    nmap_height: u32,
    flags: u8,
    quantized_vertex_size: usize,
    floats_per_vertex: usize,
}

impl ValidatedHeader {
    fn has_texture(&self) -> bool {
        self.flags & FLAG_HAS_TEXTURE != 0
    }
    fn has_uvs(&self) -> bool {
        self.flags & FLAG_HAS_UVS != 0
    }
    fn has_tangents(&self) -> bool {
        self.flags & FLAG_HAS_TANGENTS != 0
    }
    fn has_normal_map(&self) -> bool {
        self.flags & FLAG_HAS_NORMAL_MAP != 0
    }
}

/// Verify the dimension caps and flag-derived vertex layout fields. Returns
/// `(floats_per_vertex, quantized_vertex_size)`.
fn validate_dimensions(
    vertex_count: u32,
    index_count: u32,
    tex_width: u32,
    tex_height: u32,
    nmap_width: u32,
    nmap_height: u32,
    flags: u8,
) -> Result<(usize, usize)> {
    if vertex_count > MAX_VERTICES {
        bail!("vertex_count {vertex_count} exceeds MAX_VERTICES {MAX_VERTICES}");
    }
    let max_indices = MAX_TRIANGLES.saturating_mul(3);
    if index_count > max_indices {
        bail!("index_count {index_count} exceeds 3 * MAX_TRIANGLES ({max_indices})");
    }
    if !index_count.is_multiple_of(3) {
        bail!("index_count {index_count} is not a multiple of 3");
    }
    if tex_width > MAX_TEXTURE_SIZE || tex_height > MAX_TEXTURE_SIZE {
        bail!("texture {tex_width}x{tex_height} exceeds MAX_TEXTURE_SIZE {MAX_TEXTURE_SIZE}");
    }
    if nmap_width > MAX_TEXTURE_SIZE || nmap_height > MAX_TEXTURE_SIZE {
        bail!("normal map {nmap_width}x{nmap_height} exceeds MAX_TEXTURE_SIZE {MAX_TEXTURE_SIZE}");
    }
    let has_uvs = flags & FLAG_HAS_UVS != 0;
    let has_tangents = flags & FLAG_HAS_TANGENTS != 0;
    Ok(match (has_uvs, has_tangents) {
        (false, _) => (6, 10),
        (true, false) => (8, 14),
        (true, true) => (12, 22),
    })
}

/// Texture or normal-map region descriptor used during header validation.
#[derive(Clone, Copy)]
struct ImageRegion {
    offset: usize,
    width: u32,
    height: u32,
}

/// Validate that the (optional) texture and normal-map regions fit in `data`.
fn check_image_regions(
    data_len: usize,
    flags: u8,
    tex_format: u8,
    texture: ImageRegion,
    normal_map: ImageRegion,
) -> Result<()> {
    let is_etc1 = tex_format == TextureFormat::Etc1 as u8;
    let image_size = |w: u32, h: u32| -> Result<usize> {
        if is_etc1 {
            Ok(etc1_data_size(w, h))
        } else {
            rgba8_byte_len(w, h)
        }
    };
    if flags & FLAG_HAS_TEXTURE != 0 && texture.width > 0 && texture.height > 0 {
        let size = image_size(texture.width, texture.height)?;
        check_region(data_len, texture.offset, Some(size), "texture region")?;
    }
    if flags & FLAG_HAS_NORMAL_MAP != 0 && normal_map.width > 0 && normal_map.height > 0 {
        let size = image_size(normal_map.width, normal_map.height)?;
        check_region(data_len, normal_map.offset, Some(size), "normal map region")?;
    }
    Ok(())
}

/// Parse and bounds-check the mesh header against `data` and the protocol
/// limits. The `host_register_mesh` import is reachable from untrusted WASM
/// guests, so every offset/size combination must be validated before any
/// indexing happens — the renderer must not panic on malformed input.
fn validate_mesh_header(data: &[u8]) -> Result<ValidatedHeader> {
    if data.len() < HEADER_SIZE + AABB_SIZE {
        bail!(
            "mesh data too small: {} bytes (need at least {})",
            data.len(),
            HEADER_SIZE + AABB_SIZE,
        );
    }
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != MESH_MAGIC {
        bail!("invalid mesh magic: 0x{magic:08X}");
    }

    let vertex_count = read_u32(data, 4);
    let index_count = read_u32(data, 8);
    let vertex_offset = read_u32(data, 12) as usize;
    let index_offset = read_u32(data, 16) as usize;
    let texture_offset = read_u32(data, 20) as usize;
    let tex_width = u32::from(read_u16(data, 24));
    let tex_height = u32::from(read_u16(data, 26));
    let tex_format = data[28];
    let flags = data[29];
    let nmap_offset = read_u32(data, 30) as usize;
    let nmap_width = u32::from(read_u16(data, 34));
    let nmap_height = u32::from(read_u16(data, 36));

    let (floats_per_vertex, quantized_vertex_size) = validate_dimensions(
        vertex_count,
        index_count,
        tex_width,
        tex_height,
        nmap_width,
        nmap_height,
        flags,
    )?;

    let vertex_count = vertex_count as usize;
    let index_count = index_count as usize;

    check_region(
        data.len(),
        vertex_offset,
        vertex_count.checked_mul(quantized_vertex_size),
        "vertex region",
    )?;
    check_region(
        data.len(),
        index_offset,
        index_count.checked_mul(2),
        "index region",
    )?;
    check_image_regions(
        data.len(),
        flags,
        tex_format,
        ImageRegion {
            offset: texture_offset,
            width: tex_width,
            height: tex_height,
        },
        ImageRegion {
            offset: nmap_offset,
            width: nmap_width,
            height: nmap_height,
        },
    )?;

    Ok(ValidatedHeader {
        vertex_count,
        index_count,
        vertex_offset,
        index_offset,
        texture_offset,
        tex_width,
        tex_height,
        tex_format,
        nmap_offset,
        nmap_width,
        nmap_height,
        flags,
        quantized_vertex_size,
        floats_per_vertex,
    })
}

/// Verify that `[offset .. offset + size]` fits inside `data_len`. Both the
/// `size` computation upstream and the `offset + size` sum are checked for
/// overflow; either failure yields an `Err` rather than a wrap or panic.
fn check_region(data_len: usize, offset: usize, size: Option<usize>, label: &str) -> Result<()> {
    let Some(size) = size else {
        bail!("{label} size overflow");
    };
    let Some(end) = offset.checked_add(size) else {
        bail!("{label} end overflow (offset={offset} + size={size})");
    };
    if end > data_len {
        bail!("{label} extends past data ({offset}..{end} > {data_len})",);
    }
    Ok(())
}

/// RGBA8 byte length with overflow-checked multiplication. `width` and
/// `height` are already capped at `MAX_TEXTURE_SIZE` by the caller, but the
/// arithmetic remains overflow-safe to keep the helper reusable.
fn rgba8_byte_len(width: u32, height: u32) -> Result<usize> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|p| p.checked_mul(4))
        .ok_or_else(|| anyhow::anyhow!("RGBA8 byte length overflow"))?;
    usize::try_from(bytes).map_err(Into::into)
}

/// Parse the optimized binary format and upload VBO/IBO/texture to GL.
#[expect(clippy::too_many_lines)]
fn parse_and_upload(gl: &glow::Context, data: &[u8]) -> Result<UploadedMesh> {
    let header = validate_mesh_header(data)?;
    let has_texture = header.has_texture();
    let has_uvs = header.has_uvs();
    let has_tangents = header.has_tangents();
    let has_normal_map = header.has_normal_map();
    let ValidatedHeader {
        vertex_count,
        index_count,
        vertex_offset,
        index_offset,
        texture_offset,
        tex_width,
        tex_height,
        tex_format,
        nmap_offset,
        nmap_width,
        nmap_height,
        quantized_vertex_size,
        floats_per_vertex,
        ..
    } = header;

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

    // Dequantize vertices into float VBO. `vertex_count` is bounded by
    // `MAX_VERTICES` and `floats_per_vertex` ≤ 12, so the capacity below
    // cannot overflow `usize` on any supported target.
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

    // Upload VBO. The guard deletes the buffer if any later step bails so
    // partial-failure paths don't leak GL handles.
    let vbo_guard = unsafe {
        let vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        let guard = BufferGuard::new(gl, vbo);
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let bytes: &[u8] = std::slice::from_raw_parts(
            vertex_floats.as_ptr().cast::<u8>(),
            vertex_floats.len() * 4,
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        guard
    };

    // Upload IBO (indices are u16, already in the right format)
    let ibo_guard = unsafe {
        let ibo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
        let guard = BufferGuard::new(gl, ibo);
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
        let index_bytes = &data[index_offset..index_offset + index_count * 2];
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
        guard
    };

    let is_etc1 = tex_format == TextureFormat::Etc1 as u8;
    let is_msdf = tex_format == TextureFormat::Msdf as u8;

    // Body and label colors (MSDF-only; left zero for ETC1 meshes).
    let body_color = read_rgba_u8(data, 40);
    let label_color = read_rgba_u8(data, 44);

    // Upload texture (optional). MSDF and Rgba8 share the uncompressed
    // RGBA8 upload path; only ETC1 takes the compressed path. Bounds were
    // already checked in `validate_mesh_header`, so the slice cannot panic.
    let texture_guard = if has_texture && tex_width > 0 && tex_height > 0 {
        let tex_size = if is_etc1 {
            etc1_data_size(tex_width, tex_height)
        } else {
            rgba8_byte_len(tex_width, tex_height)
                .expect("BUG: rgba8 size validated in validate_mesh_header")
        };
        let tex_data = &data[texture_offset..texture_offset + tex_size];
        Some(TextureGuard::new(
            gl,
            upload_texture(gl, tex_width, tex_height, tex_data, is_etc1)?,
        ))
    } else {
        None
    };

    // Upload normal map (optional, same format as albedo)
    let normal_map_guard = if has_normal_map && nmap_width > 0 && nmap_height > 0 {
        let nmap_size = if is_etc1 {
            etc1_data_size(nmap_width, nmap_height)
        } else {
            rgba8_byte_len(nmap_width, nmap_height)
                .expect("BUG: rgba8 size validated in validate_mesh_header")
        };
        let nmap_data = &data[nmap_offset..nmap_offset + nmap_size];
        Some(TextureGuard::new(
            gl,
            upload_texture(gl, nmap_width, nmap_height, nmap_data, is_etc1)?,
        ))
    } else {
        None
    };

    // All steps succeeded — defuse every guard so the handles survive into
    // the returned `UploadedMesh`.
    let vbo = vbo_guard.defuse();
    let ibo = ibo_guard.defuse();
    let texture = texture_guard.map(TextureGuard::defuse);
    let normal_map = normal_map_guard.map(TextureGuard::defuse);

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
        is_msdf,
        body_color,
        label_color,
    })
}

/// Read four consecutive u8 channels from the header and convert to a
/// linear `[f32; 4]` in [0..1] suitable for direct shader-uniform use.
fn read_rgba_u8(data: &[u8], offset: usize) -> [f32; 4] {
    [
        f32::from(data[offset]) / 255.0,
        f32::from(data[offset + 1]) / 255.0,
        f32::from(data[offset + 2]) / 255.0,
        f32::from(data[offset + 3]) / 255.0,
    ]
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
        let texture_guard = TextureGuard::new(gl, texture);
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
        let depth_guard = RenderbufferGuard::new(gl, depth_rb);
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
        let fbo_guard = FramebufferGuard::new(gl, fbo);
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
            // Guards drop at scope end and free the GL handles.
            bail!("mesh FBO incomplete: status 0x{status:04X}");
        }

        Ok((
            fbo_guard.defuse(),
            texture_guard.defuse(),
            depth_guard.defuse(),
        ))
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
        let color_guard = RenderbufferGuard::new(gl, color_rb);
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
        let depth_guard = RenderbufferGuard::new(gl, depth_rb);
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
        let fbo_guard = FramebufferGuard::new(gl, fbo);
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
            // Guards drop at scope end and free the GL handles.
            bail!("mesh MSAA FBO incomplete: status 0x{status:04X}");
        }

        Ok((
            fbo_guard.defuse(),
            color_guard.defuse(),
            depth_guard.defuse(),
        ))
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

/// `NaN` is the sentinel for "lighting disabled" (see `MeshView::light: None`
/// in the SDK). A naive `(old - new).abs() > eps` returns `false` for any
/// `NaN` operand, so a `Some(...)` → `None` transition would never dirty the
/// slot and the cached lit frame would stick. Either operand being `NaN`
/// therefore forces a re-render.
fn is_dirty(old: f32, new: f32) -> bool {
    old.is_nan() || new.is_nan() || (old - new).abs() > DIRTY_EPSILON
}

/// Once-per-process warning when a widget asks for more atlas slots than the
/// renderer offers. Logged at `warn` level so devs catch it during testing
/// without the stream being drowned in per-frame repeats once 9 dice roll
/// over to 10+.
fn warn_slot_overflow_once(slot_index: u32) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "mesh: slot_index {slot_index} exceeds MAX_SLOTS ({MAX_SLOTS}); \
             excess draws are suppressed. Reduce concurrent meshes per frame."
        );
    }
}

fn mesh_id_from_storage_index(index: usize) -> Option<u16> {
    let one_based = index.checked_add(1)?;
    u16::try_from(one_based).ok()
}

fn mesh_id_to_storage_index(mesh_id: u16) -> Option<usize> {
    mesh_id.checked_sub(1).map(usize::from)
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

#[cfg(test)]
mod tests {
    use super::{
        AABB_SIZE, DIRTY_EPSILON, INVALID_MESH_ID, is_dirty, mesh_id_from_storage_index,
        mesh_id_to_storage_index, validate_mesh_header,
    };
    use bmc_wasm_protocol::mesh::{
        FLAG_HAS_TEXTURE, FLAG_HAS_UVS, HEADER_SIZE, MAX_TEXTURE_SIZE, MAX_TRIANGLES, MAX_VERTICES,
        MESH_MAGIC,
    };

    /// First byte after the 48-byte header + 24-byte AABB record. Used as the
    /// default region offset in fixtures so vertex/index regions sit
    /// immediately after the AABB.
    const BODY_OFFSET: usize = HEADER_SIZE + AABB_SIZE;

    #[test]
    fn mesh_ids_are_one_based() {
        assert_eq!(mesh_id_from_storage_index(0), Some(1));
        assert_eq!(mesh_id_to_storage_index(1), Some(0));
    }

    #[test]
    fn invalid_mesh_id_does_not_map_to_storage() {
        assert_eq!(mesh_id_to_storage_index(INVALID_MESH_ID), None);
    }

    fn write_u32(buf: &mut [u8], range: std::ops::Range<usize>, value: usize) {
        let v = u32::try_from(value).expect("BUG: test fixture exceeds u32 range");
        buf[range].copy_from_slice(&v.to_le_bytes());
    }

    /// Build a minimal valid header with no texture / no normal map and the
    /// requested vertex/index counts/offsets. Caller must ensure `total_len`
    /// is large enough for the requested regions.
    fn header_buffer(
        vertex_count: usize,
        index_count: usize,
        vertex_offset: usize,
        index_offset: usize,
        flags: u8,
        total_len: usize,
    ) -> Vec<u8> {
        let mut buf = vec![0_u8; total_len.max(BODY_OFFSET)];
        buf[0..4].copy_from_slice(&MESH_MAGIC.to_le_bytes());
        write_u32(&mut buf, 4..8, vertex_count);
        write_u32(&mut buf, 8..12, index_count);
        write_u32(&mut buf, 12..16, vertex_offset);
        write_u32(&mut buf, 16..20, index_offset);
        buf[29] = flags;
        buf
    }

    #[test]
    fn rejects_payload_smaller_than_header_plus_aabb() {
        let buf = vec![0_u8; HEADER_SIZE]; // missing AABB
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("too small"), "{err}");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut buf = vec![0_u8; BODY_OFFSET];
        buf[0..4].copy_from_slice(b"NOPE");
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("magic"), "{err}");
    }

    #[test]
    fn rejects_excessive_vertex_count() {
        let buf = header_buffer(
            MAX_VERTICES as usize + 1,
            0,
            BODY_OFFSET,
            BODY_OFFSET,
            0,
            BODY_OFFSET,
        );
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("MAX_VERTICES"), "{err}");
    }

    #[test]
    fn rejects_excessive_index_count() {
        let buf = header_buffer(
            0,
            MAX_TRIANGLES as usize * 3 + 3,
            BODY_OFFSET,
            BODY_OFFSET,
            0,
            BODY_OFFSET,
        );
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("MAX_TRIANGLES"), "{err}");
    }

    #[test]
    fn rejects_index_count_not_multiple_of_three() {
        let buf = header_buffer(0, 7, BODY_OFFSET, BODY_OFFSET, 0, BODY_OFFSET + 14);
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("multiple of 3"), "{err}");
    }

    #[test]
    fn rejects_oversized_texture_dimension() {
        let mut buf = header_buffer(
            0,
            0,
            BODY_OFFSET,
            BODY_OFFSET,
            FLAG_HAS_TEXTURE | FLAG_HAS_UVS,
            BODY_OFFSET,
        );
        let oversize =
            u16::try_from(MAX_TEXTURE_SIZE + 1).expect("BUG: MAX_TEXTURE_SIZE+1 fits in u16");
        buf[24..26].copy_from_slice(&oversize.to_le_bytes());
        buf[26..28].copy_from_slice(&oversize.to_le_bytes());
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("MAX_TEXTURE_SIZE"), "{err}");
    }

    #[test]
    fn rejects_truncated_index_region() {
        // Claim 6 indices (12 bytes) at offset = HEADER+AABB but make the
        // buffer only large enough to hold the header.
        let buf = header_buffer(0, 6, BODY_OFFSET, BODY_OFFSET, 0, BODY_OFFSET);
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(err.contains("index region"), "{err}");
    }

    #[test]
    fn rejects_offset_overflow() {
        // index_offset = u32::MAX, index_count = 6 → end overflows
        let buf = header_buffer(0, 6, BODY_OFFSET, u32::MAX as usize, 0, BODY_OFFSET);
        let err = validate_mesh_header(&buf).unwrap_err().to_string();
        assert!(
            err.contains("end overflow") || err.contains("index region"),
            "{err}",
        );
    }

    #[test]
    fn is_dirty_treats_either_nan_operand_as_dirty() {
        // Existing behavior: first render sentinel.
        assert!(is_dirty(f32::NAN, 30.0));
        // Regression: lighting toggled off (Some(pitch) → None ≡ NaN) must
        // re-render. Naive `(old - new).abs()` returns NaN > eps == false.
        assert!(is_dirty(30.0, f32::NAN));
        // Both NaN preserves the prior "first render forced" semantics.
        assert!(is_dirty(f32::NAN, f32::NAN));
    }

    #[test]
    fn is_dirty_compares_finite_values_against_epsilon() {
        assert!(!is_dirty(1.0, 1.0));
        assert!(!is_dirty(1.0, 1.0 + DIRTY_EPSILON / 2.0));
        assert!(is_dirty(1.0, 1.0 + DIRTY_EPSILON * 2.0));
    }

    #[test]
    fn accepts_minimal_empty_mesh() {
        let buf = header_buffer(0, 0, BODY_OFFSET, BODY_OFFSET, 0, BODY_OFFSET);
        let h = validate_mesh_header(&buf)
            .expect("BUG: minimal valid header fixture must satisfy validate_mesh_header");
        assert_eq!(h.vertex_count, 0);
        assert_eq!(h.index_count, 0);
    }
}

#[cfg(feature = "profiling")]
mod profile {
    use crate::profile::{MemProbe, TARGET};

    pub(super) struct RegisterMeshProbe(MemProbe);

    impl RegisterMeshProbe {
        pub(super) fn start() -> Self {
            Self(MemProbe::start())
        }

        pub(super) fn finish(self, id: u16, data_bytes: usize) {
            let s = self.0.snapshot();
            tracing::info!(
                target: TARGET,
                "register_mesh id={id} data_bytes={data_bytes} upload_us={upload_us} \
                 vmrss_delta_kb={vmrss:+} rss_anon_delta_kb={anon:+} \
                 rss_shmem_delta_kb={shmem:+} \
                 cma_free_delta_kb={cma:+} cma_free_kb={cma_free} mem_free_kb={mem_free}",
                upload_us = s.elapsed_us,
                vmrss = s.vmrss_delta_kb,
                anon = s.rss_anon_delta_kb,
                shmem = s.rss_shmem_delta_kb,
                cma = s.cma_free_delta_kb,
                cma_free = s.cma_free_kb,
                mem_free = s.mem_free_kb,
            );
        }
    }
}
