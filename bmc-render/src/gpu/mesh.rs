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

mod atlas;
mod decoder;
mod math;
mod raii;

use bmc_wasm_protocol::MeshId;

use atlas::{
    ATLAS_COLS, ATLAS_H, ATLAS_ROWS, ATLAS_W, MAX_SLOTS, SLOT_SIZE, SlotState, is_dirty,
    mesh_id_from_storage_index, mesh_id_to_storage_index, warn_slot_overflow_once,
};
use decoder::parse_and_upload;
use math::{compute_mvp, flatten_mat3, quat_to_mat3};
use raii::{FramebufferGuard, RenderbufferGuard, TextureGuard};

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

impl UploadedMesh {
    /// Release every GL handle owned by this mesh (VBO, IBO, optional diffuse
    /// texture, optional normal map).
    ///
    /// # Safety
    /// The caller must ensure a current GL context matching `gl` and that no
    /// outstanding draw call still references these handles. After this call
    /// the `UploadedMesh` must not be used again.
    pub unsafe fn drop_gl(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ibo);
            if let Some(tex) = self.texture {
                gl.delete_texture(tex);
            }
            if let Some(nmap) = self.normal_map {
                gl.delete_texture(nmap);
            }
        }
    }
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
    // Tag → MeshId for idempotent registration.
    by_tag: std::collections::HashMap<String, MeshId>,
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
                by_tag: std::collections::HashMap::new(),
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

    /// Upload mesh binary data to GPU (VBO + IBO + optional texture) under
    /// `tag`. Idempotent: a second call with the same tag returns the cached
    /// ID without re-uploading.
    pub fn register_mesh(&mut self, gl: &glow::Context, tag: &str, data: &[u8]) -> Option<MeshId> {
        if let Some(&id) = self.by_tag.get(tag) {
            return Some(id);
        }

        #[cfg(feature = "profiling")]
        let profile_before = profile::RegisterMeshProbe::start();

        // Reserve the ID before uploading so an exhausted id space
        // doesn't leak the freshly-allocated VBO/IBO/textures —
        // `UploadedMesh` has no `Drop` impl that frees GPU handles.
        let Some(id) = mesh_id_from_storage_index(self.meshes.len()) else {
            tracing::error!("mesh registration failed: id space exhausted ({tag})");
            return None;
        };

        let mesh = match parse_and_upload(gl, data) {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("mesh upload failed ({tag}): {e}");
                return None;
            }
        };

        self.meshes.push(Some(mesh));
        self.by_tag.insert(tag.to_owned(), id);
        // No slot invalidation needed: `id` is fresh (one-based index of
        // the just-pushed entry), so no existing slot's `prev_mesh_id` can
        // collide. `SlotState::check_and_update` already triggers a redraw
        // when a slot is first bound to this mesh via the `prev_mesh_id !=
        // mesh_id` check.

        #[cfg(feature = "profiling")]
        profile_before.finish(id.to_wire(), data.len());

        Some(id)
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
        mesh_id: MeshId,
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

        let mesh_idx = mesh_id_to_storage_index(mesh_id);
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
                m.drop_gl(gl);
            }
        }
    }

    /// Evict a single tag's mesh: drop its GL handles and clear the storage
    /// slot. The `MeshId` returned by earlier `register_mesh(tag, …)` calls
    /// becomes invalid; subsequent renders that reference it no-op safely
    /// because the slot is now `None`. The atlas pixels last drawn under that
    /// slot remain until something else redraws.
    ///
    /// Returns `true` if a tag was found and evicted, `false` otherwise.
    ///
    /// IDs are not recycled — registering a fresh tag after eviction allocates
    /// a new storage slot. Slot recycling is a separate concern from resource
    /// release and is not implemented here.
    pub fn evict(&mut self, gl: &glow::Context, tag: &str) -> bool {
        let Some(id) = self.by_tag.remove(tag) else {
            return false;
        };
        let idx = mesh_id_to_storage_index(id);
        let Some(slot) = self.meshes.get_mut(idx) else {
            return false;
        };
        let Some(mesh) = slot.take() else {
            return false;
        };
        unsafe { mesh.drop_gl(gl) };
        true
    }

    /// Evict every tag whose key starts with `prefix`. Returns the number of
    /// tags removed.
    pub fn evict_prefix(&mut self, gl: &glow::Context, prefix: &str) -> usize {
        // Collect first; can't mutate `by_tag` while iterating it.
        let tags: Vec<String> = self
            .by_tag
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        let mut n = 0;
        for tag in tags {
            if self.evict(gl, &tag) {
                n += 1;
            }
        }
        n
    }
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

// ── Tests ──────────────────────────────────────────────────────────
//
// Linux-only: the test harness boots a real headless GLES 2.0 context via
// EGL + Mesa llvmpipe (same path used by the `capture` binary). Other
// platforms would need their own headless GL bootstrap; for now BDK-458
// only cares about correctness on the device target, which mirrors Linux.
#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use super::UploadedMesh;
    use glow::HasContext;

    /// Real headless GL context (EGL surfaceless + ES 2.0 via Mesa
    /// llvmpipe). Keeps the EGL display / surface / context alive for the
    /// duration of the test via the `_keepalive` field.
    struct GlHarness {
        gl: glow::Context,
        _keepalive: Box<dyn std::any::Any>,
    }

    fn create_headless_gl() -> anyhow::Result<GlHarness> {
        use std::ffi::CString;
        use std::num::NonZeroU32;

        use anyhow::{Context as _, anyhow};
        use glutin::config::{ConfigSurfaceTypes, ConfigTemplateBuilder};
        use glutin::context::{ContextApi, ContextAttributesBuilder, Version};
        use glutin::display::{Display, GetGlDisplay};
        use glutin::prelude::*;
        use glutin::surface::{PbufferSurface, SurfaceAttributesBuilder};

        let devices: Vec<_> = glutin::api::egl::device::Device::query_devices()
            .context("EGL device enumeration not supported")?
            .collect();
        let device = devices
            .iter()
            .find(|d| d.extensions().contains("EGL_MESA_device_software"))
            .or_else(|| devices.first())
            .ok_or_else(|| anyhow!("no EGL devices found"))?;
        let egl_display = unsafe { glutin::api::egl::display::Display::with_device(device, None) }
            .context("failed to create EGL display")?;
        let display = Display::Egl(egl_display);

        let template = ConfigTemplateBuilder::new()
            .with_surface_type(ConfigSurfaceTypes::PBUFFER)
            .build();
        let gl_config = unsafe { display.find_configs(template) }
            .map_err(|e| anyhow!("find_configs failed: {e}"))?
            .next()
            .ok_or_else(|| anyhow!("no GL configs"))?;
        let gl_display = gl_config.display();

        let context_attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(2, 0))))
            .build(None);
        let gl_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .context("create_context failed")?
        };

        let surface_attrs = SurfaceAttributesBuilder::<PbufferSurface>::new().build(
            NonZeroU32::new(1).expect("BUG: const 1 is non-zero"),
            NonZeroU32::new(1).expect("BUG: const 1 is non-zero"),
        );
        let surface = unsafe {
            gl_display
                .create_pbuffer_surface(&gl_config, &surface_attrs)
                .context("create_pbuffer_surface failed")?
        };
        let gl_context = gl_context
            .make_current(&surface)
            .context("make_current failed")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_display.get_proc_address(&CString::new(s).unwrap_or_default())
            })
        };

        Ok(GlHarness {
            gl,
            _keepalive: Box::new((surface, gl_context)),
        })
    }

    /// Allocate a buffer and bind it once so `gl.is_buffer` reports `true`.
    /// Per the GLES 2.0 spec, names returned by `glGenBuffers` only become
    /// "real" buffers (queryable by `glIsBuffer`) once first bound.
    fn create_real_buffer(gl: &glow::Context, target: u32) -> glow::Buffer {
        let buf = unsafe { gl.create_buffer() }.expect("BUG: create_buffer failed");
        unsafe {
            gl.bind_buffer(target, Some(buf));
            gl.bind_buffer(target, None);
        }
        buf
    }

    /// Allocate a texture and bind it once — same reason as above:
    /// `glIsTexture` only returns true after first bind.
    fn create_real_texture(gl: &glow::Context) -> glow::Texture {
        let tex = unsafe { gl.create_texture() }.expect("BUG: create_texture failed");
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        tex
    }

    /// Build an `UploadedMesh` whose GL handles are real, freshly allocated
    /// objects (not wrapped in any draw state). Used to verify `drop_gl`
    /// releases them all.
    fn build_test_mesh(
        gl: &glow::Context,
        with_texture: bool,
        with_normal_map: bool,
    ) -> UploadedMesh {
        UploadedMesh {
            vbo: create_real_buffer(gl, glow::ARRAY_BUFFER),
            ibo: create_real_buffer(gl, glow::ELEMENT_ARRAY_BUFFER),
            index_count: 0,
            texture: with_texture.then(|| create_real_texture(gl)),
            normal_map: with_normal_map.then(|| create_real_texture(gl)),
            has_uvs: false,
            has_tangents: false,
            is_msdf: false,
            body_color: [0.0; 4],
            label_color: [0.0; 4],
        }
    }

    #[test]
    fn drop_gl_releases_all_handles() {
        let harness = create_headless_gl().expect("BUG: headless GL setup failed");
        let gl = &harness.gl;

        let mesh = build_test_mesh(gl, true, true);
        let vbo = mesh.vbo;
        let ibo = mesh.ibo;
        let texture = mesh.texture.expect("BUG: texture should be Some");
        let normal_map = mesh.normal_map.expect("BUG: normal_map should be Some");

        unsafe {
            assert!(gl.is_buffer(vbo), "BUG: vbo should be live before drop");
            assert!(gl.is_buffer(ibo), "BUG: ibo should be live before drop");
            assert!(
                gl.is_texture(texture),
                "BUG: texture should be live before drop"
            );
            assert!(
                gl.is_texture(normal_map),
                "BUG: normal_map should be live before drop",
            );
        }

        unsafe { mesh.drop_gl(gl) };

        unsafe {
            assert!(!gl.is_buffer(vbo), "BUG: vbo leaked past drop_gl");
            assert!(!gl.is_buffer(ibo), "BUG: ibo leaked past drop_gl");
            assert!(!gl.is_texture(texture), "BUG: texture leaked past drop_gl");
            assert!(
                !gl.is_texture(normal_map),
                "BUG: normal_map leaked past drop_gl",
            );
        }
    }

    #[test]
    fn drop_gl_handles_optional_textures() {
        let harness = create_headless_gl().expect("BUG: headless GL setup failed");
        let gl = &harness.gl;

        let mesh = build_test_mesh(gl, false, false);
        let vbo = mesh.vbo;
        let ibo = mesh.ibo;

        unsafe {
            assert!(gl.is_buffer(vbo), "BUG: vbo should be live before drop");
            assert!(gl.is_buffer(ibo), "BUG: ibo should be live before drop");
        }

        unsafe { mesh.drop_gl(gl) };

        unsafe {
            assert!(!gl.is_buffer(vbo), "BUG: vbo leaked past drop_gl");
            assert!(!gl.is_buffer(ibo), "BUG: ibo leaked past drop_gl");
        }
    }
}
