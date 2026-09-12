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

//! GL sphere renderer — UV sphere mesh with an equirectangular texture.
//!
//! Renders a texture onto a 3D sphere in an offscreen FBO, then shares
//! the FBO color attachment with femtovg as a native texture (zero-copy).
//!
//! Optional features controlled per-draw:
//! - **Light shading**: directional light with terminator (day/night boundary)
//! - **Atmosphere**: limb darkening + bluish edge glow (earth-like haze)

#![expect(clippy::cast_precision_loss, clippy::cast_possible_wrap)]

use anyhow::{Result, bail};
use femtovg::renderer::OpenGl;
use femtovg::{Canvas, ImageFlags, ImageId, ImageInfo, PixelFormat};
use glow::HasContext;

// ── Shaders (GLSL ES 1.00 / #version 100) ──────────────────────────

const VERTEX_SHADER: &str = "\
#version 100
attribute vec3 a_position;
attribute vec2 a_uv;

uniform mat3 u_rotation;
uniform float u_zoom;
uniform float u_aspect;
uniform vec3 u_light_dir;
uniform float u_atmosphere;

varying vec2 v_uv;
varying vec2 v_lighting;

void main() {
    vec3 view = a_position * u_rotation;
    gl_Position = vec4(view.x / u_aspect, -view.y, 0.0, 1.0 - view.z / u_zoom);
    v_uv = a_uv;

    float shade = 1.0;
    if (dot(u_light_dir, u_light_dir) > 0.001) {
        shade = smoothstep(-0.1, 0.15, dot(a_position, u_light_dir));
    }

    float color_scale = mix(0.55, 1.0, shade);
    float glow = 0.0;
    if (u_atmosphere > 0.5) {
        float rim = clamp(1.0 - view.z, 0.0, 1.0);
        color_scale *= 1.0 - rim * rim * 0.7;
        glow = rim * sqrt(rim);
    }
    v_lighting = vec2(color_scale, glow);
}
";

const FRAGMENT_SHADER: &str = "\
#version 100
precision mediump float;

uniform sampler2D u_texture;

varying vec2 v_uv;
varying vec2 v_lighting;

void main() {
    vec3 tex_color = texture2D(u_texture, v_uv).rgb;
    gl_FragColor = vec4(
        tex_color * v_lighting.x + vec3(0.12, 0.22, 0.45) * v_lighting.y,
        1.0
    );
}
";

const DIRTY_EPSILON: f32 = 0.001;
// Keep the front vertex's perspective divisor positive under shader rounding.
const PROJECTION_ZOOM_MARGIN: f32 = 0.001;
const MIN_PROJECTION_ZOOM: f32 = 1.0 + PROJECTION_ZOOM_MARGIN;

// Equal 5.625° steps keep the projected silhouette error below one pixel.
const LONGITUDE_SEGMENTS: usize = 64;
const LATITUDE_SEGMENTS: usize = 32;
const VERTEX_COMPONENTS: usize = 5;
const POSITION_COMPONENTS: usize = 3;
#[expect(
    clippy::cast_possible_truncation,
    reason = "the fixed sphere vertex layout fits in GLsizei"
)]
const VERTEX_STRIDE_BYTES: i32 = (VERTEX_COMPONENTS * std::mem::size_of::<f32>()) as i32;
#[expect(
    clippy::cast_possible_truncation,
    reason = "the fixed sphere vertex layout fits in GLsizei"
)]
const UV_OFFSET_BYTES: i32 = (POSITION_COMPONENTS * std::mem::size_of::<f32>()) as i32;
// Target-device profiles put the native-size mesh just over the frame budget;
// three-quarter dimensions brought it under while captures stayed within 0.8% RMSE.
const OFFSCREEN_SCALE_NUMERATOR: u32 = 3;
const OFFSCREEN_SCALE_DENOMINATOR: u32 = 4;

fn offscreen_dimension(size: u32) -> u32 {
    size.saturating_mul(OFFSCREEN_SCALE_NUMERATOR)
        .div_ceil(OFFSCREEN_SCALE_DENOMINATOR)
}

fn sphere_rotation(lat_rad: f32, lon_rad: f32) -> [f32; 9] {
    let (sin_lat, cos_lat) = lat_rad.sin_cos();
    let (sin_lon, cos_lon) = lon_rad.sin_cos();
    [
        cos_lon,
        0.0,
        -sin_lon,
        -sin_lat * sin_lon,
        cos_lat,
        -sin_lat * cos_lon,
        cos_lat * sin_lon,
        sin_lat,
        cos_lat * cos_lon,
    ]
}

// ── SphereRenderer ──────────────────────────────────────────────────

/// Offscreen GL renderer that draws a texture onto a 3D sphere.
///
/// The rendered result lives in an FBO color attachment that is shared with
/// femtovg as a native texture — no pixel readback needed.
pub struct SphereRenderer {
    program: glow::Program,
    vao: Option<glow::VertexArray>,
    vbo: glow::Buffer,
    ibo: glow::Buffer,
    index_count: i32,
    fbo: glow::Framebuffer,
    fbo_texture: glow::Texture,
    texture: Option<glow::Texture>,
    image_id: ImageId,
    width: u32,
    height: u32,
    aspect: f32,
    // Uniform locations
    u_rotation: glow::UniformLocation,
    u_light_dir: glow::UniformLocation,
    u_zoom: glow::UniformLocation,
    u_aspect: glow::UniformLocation,
    u_atmosphere: glow::UniformLocation,
    u_texture: glow::UniformLocation,
    // Dirty tracking — NaN forces first render
    last_lat: f32,
    last_lon: f32,
    last_light_lat: f32,
    last_light_lon: f32,
    last_zoom: f32,
    last_atmosphere: bool,
}

struct PendingSphereResources<'a> {
    gl: &'a glow::Context,
    program: Option<glow::Program>,
    vao: Option<glow::VertexArray>,
    vbo: Option<glow::Buffer>,
    ibo: Option<glow::Buffer>,
    fbo: Option<glow::Framebuffer>,
    fbo_texture: Option<glow::Texture>,
}

impl<'a> PendingSphereResources<'a> {
    fn new(gl: &'a glow::Context, program: glow::Program) -> Self {
        Self {
            gl,
            program: Some(program),
            vao: None,
            vbo: None,
            ibo: None,
            fbo: None,
            fbo_texture: None,
        }
    }
}

impl Drop for PendingSphereResources<'_> {
    fn drop(&mut self) {
        unsafe {
            // SAFETY: every handle was created by `self.gl` and remains pending here.
            if let Some(vao) = self.vao.take() {
                self.gl.delete_vertex_array(vao);
            }
            if let Some(vbo) = self.vbo.take() {
                self.gl.delete_buffer(vbo);
            }
            if let Some(ibo) = self.ibo.take() {
                self.gl.delete_buffer(ibo);
            }
            if let Some(fbo) = self.fbo.take() {
                self.gl.delete_framebuffer(fbo);
            }
            if let Some(texture) = self.fbo_texture.take() {
                self.gl.delete_texture(texture);
            }
            if let Some(program) = self.program.take() {
                self.gl.delete_program(program);
            }
        }
    }
}

impl SphereRenderer {
    /// Compile shaders, create offscreen FBO, and register the FBO texture with
    /// femtovg for zero-copy sampling.
    pub fn new(
        gl: &glow::Context,
        canvas: &mut Canvas<OpenGl>,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        unsafe {
            let requested_width = width;
            let requested_height = height;
            let width = offscreen_dimension(requested_width);
            let height = offscreen_dimension(requested_height);
            let program = compile_program(gl)?;
            let mut resources = PendingSphereResources::new(gl, program);

            gl.bind_attrib_location(program, 0, "a_position");
            gl.bind_attrib_location(program, 1, "a_uv");
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                bail!("sphere shader link failed: {log}");
            }

            let get_uniform = |name: &str| -> Result<glow::UniformLocation> {
                gl.get_uniform_location(program, name)
                    .ok_or_else(|| anyhow::anyhow!("missing uniform: {name}"))
            };
            let u_rotation = get_uniform("u_rotation")?;
            let u_light_dir = get_uniform("u_light_dir")?;
            let u_zoom = get_uniform("u_zoom")?;
            let u_aspect = get_uniform("u_aspect")?;
            let u_atmosphere = get_uniform("u_atmosphere")?;
            let u_texture = get_uniform("u_texture")?;

            let (fbo, fbo_texture) = create_offscreen_fbo(gl, width, height)?;
            resources.fbo = Some(fbo);
            resources.fbo_texture = Some(fbo_texture);

            // VAO required on desktop GL core profile and ES 3.0+.
            // Optional on ES 2.0 (extension), so we try and skip if unavailable.
            resources.vao = gl.create_vertex_array().ok();

            let (vbo, ibo, index_count) = create_sphere_mesh(gl, resources.vao)?;
            resources.vbo = Some(vbo);
            resources.ibo = Some(ibo);
            let aspect = requested_width as f32 / requested_height as f32;

            // Y-flip is handled by the projected vertex position.
            let image_id = canvas.create_image_from_native_texture(
                fbo_texture,
                ImageInfo::new(
                    ImageFlags::empty(),
                    width as usize,
                    height as usize,
                    PixelFormat::Rgba8,
                ),
            )?;

            let gl_version = gl.get_parameter_string(glow::VERSION);
            let glsl_version = gl.get_parameter_string(glow::SHADING_LANGUAGE_VERSION);
            tracing::info!(
                "sphere renderer initialized ({width}x{height}), \
                 GL={gl_version}, GLSL={glsl_version}, VAO={}",
                resources.vao.is_some()
            );

            Ok(Self {
                program: resources
                    .program
                    .take()
                    .expect("BUG: initialized sphere owns its GL program"),
                vao: resources.vao.take(),
                vbo: resources
                    .vbo
                    .take()
                    .expect("BUG: initialized sphere owns its vertex buffer"),
                ibo: resources
                    .ibo
                    .take()
                    .expect("BUG: initialized sphere owns its index buffer"),
                index_count,
                fbo: resources
                    .fbo
                    .take()
                    .expect("BUG: initialized sphere owns its framebuffer"),
                fbo_texture: resources
                    .fbo_texture
                    .take()
                    .expect("BUG: initialized sphere owns its framebuffer texture"),
                texture: None,
                image_id,
                width,
                height,
                aspect,
                u_rotation,
                u_light_dir,
                u_zoom,
                u_aspect,
                u_atmosphere,
                u_texture,
                last_lat: f32::NAN,
                last_lon: f32::NAN,
                last_light_lat: f32::NAN,
                last_light_lon: f32::NAN,
                last_zoom: f32::NAN,
                last_atmosphere: false,
            })
        }
    }

    /// Store the sphere texture handle (borrowed from femtovg, not owned).
    pub fn set_texture(&mut self, tex: glow::Texture) {
        self.texture = Some(tex);
        // Force re-render with the new texture
        self.last_lat = f32::NAN;
    }

    /// Render the sphere to the offscreen FBO if any parameter changed.
    /// Returns the femtovg image backing the FBO, or `None` when no texture
    /// is set yet — callers must skip the draw in that case so stale FBO
    /// pixels aren't sampled.
    #[expect(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        gl: &glow::Context,
        lat: f32,
        lon: f32,
        zoom: f32,
        light_lat: f32,
        light_lon: f32,
        atmosphere: bool,
    ) -> Option<ImageId> {
        let tex = self.texture?;
        let zoom = projection_zoom(zoom);

        // Dirty check — skip render if nothing changed
        if !is_dirty(self.last_lat, lat)
            && !is_dirty(self.last_lon, lon)
            && !is_dirty(self.last_zoom, zoom)
            && !is_dirty(self.last_light_lat, light_lat)
            && !is_dirty(self.last_light_lon, light_lon)
            && self.last_atmosphere == atmosphere
        {
            return Some(self.image_id);
        }

        self.last_lat = lat;
        self.last_lon = lon;
        self.last_zoom = zoom;
        self.last_light_lat = light_lat;
        self.last_light_lon = light_lon;
        self.last_atmosphere = atmosphere;

        let lat_rad = lat.to_radians();
        let lon_rad = lon.to_radians();
        let light_lat_rad = light_lat.to_radians();
        let light_lon_rad = light_lon.to_radians();

        unsafe {
            // SAFETY: the caller's GL context is current for the whole pass.
            let caller_state = super::offscreen::OffscreenPassState::capture(gl);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, self.width as i32, self.height as i32);

            // Reset GL state that femtovg's previous flush may have left enabled.
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
            gl.front_face(glow::CW);
            gl.disable(glow::STENCIL_TEST);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);

            gl.use_program(Some(self.program));

            // Uniforms
            gl.uniform_matrix_3_f32_slice(
                Some(&self.u_rotation),
                false,
                &sphere_rotation(lat_rad, lon_rad),
            );
            gl.uniform_3_f32(
                Some(&self.u_light_dir),
                light_lat_rad.cos() * light_lon_rad.sin(),
                light_lat_rad.sin(),
                light_lat_rad.cos() * light_lon_rad.cos(),
            );
            gl.uniform_1_f32(Some(&self.u_zoom), zoom);
            gl.uniform_1_f32(Some(&self.u_aspect), self.aspect);
            gl.uniform_1_f32(Some(&self.u_atmosphere), if atmosphere { 1.0 } else { 0.0 });

            // Bind texture to sampler unit 0
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.uniform_1_i32(Some(&self.u_texture), 0);

            if let Some(vao) = self.vao {
                gl.bind_vertex_array(Some(vao));
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(
                0,
                i32::try_from(POSITION_COMPONENTS)
                    .expect("BUG: sphere position components fit in GLint"),
                glow::FLOAT,
                false,
                VERTEX_STRIDE_BYTES,
                0,
            );
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(
                1,
                i32::try_from(VERTEX_COMPONENTS - POSITION_COMPONENTS)
                    .expect("BUG: sphere UV components fit in GLint"),
                glow::FLOAT,
                false,
                VERTEX_STRIDE_BYTES,
                UV_OFFSET_BYTES,
            );
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.ibo));

            gl.draw_elements(glow::TRIANGLES, self.index_count, glow::UNSIGNED_SHORT, 0);

            let err = gl.get_error();
            if err != glow::NO_ERROR {
                tracing::error!("GL error after sphere draw: 0x{err:04X}");
            }

            gl.disable_vertex_attrib_array(0);
            gl.disable_vertex_attrib_array(1);
            gl.disable(glow::CULL_FACE);
            gl.front_face(glow::CCW);
            if self.vao.is_some() {
                gl.bind_vertex_array(None);
            } else {
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
            }
            caller_state.restore(gl);
        }

        Some(self.image_id)
    }

    /// Release FemtoVG bookkeeping and GL resources owned by the sphere renderer.
    pub fn destroy(self, gl: &glow::Context, canvas: &mut Canvas<OpenGl>) {
        canvas.delete_image(self.image_id);

        unsafe {
            if let Some(vao) = self.vao {
                gl.delete_vertex_array(vao);
            }
            gl.delete_buffer(self.vbo);
            gl.delete_buffer(self.ibo);
            gl.delete_framebuffer(self.fbo);
            gl.delete_texture(self.fbo_texture);
            gl.delete_program(self.program);
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// `NaN` is the sentinel for "parameter unset" (e.g. lighting disabled). The
/// naive `(old - new).abs() > eps` returns `false` for any `NaN` operand, so
/// a `valid → NaN` transition would silently keep the cached frame. Either
/// operand being `NaN` therefore forces a re-render.
fn is_dirty(old: f32, new: f32) -> bool {
    old.is_nan() || new.is_nan() || (old - new).abs() > DIRTY_EPSILON
}

fn projection_zoom(zoom: f32) -> f32 {
    zoom.max(MIN_PROJECTION_ZOOM)
}

fn sphere_mesh_data() -> (Vec<f32>, Vec<u16>) {
    let row_width = LONGITUDE_SEGMENTS + 1;
    let mut vertices = Vec::with_capacity((LATITUDE_SEGMENTS + 1) * row_width * VERTEX_COMPONENTS);
    // Keep this mapping aligned with widgets-wasm/iss-position/tools/_textures.py.
    for row in 0..=LATITUDE_SEGMENTS {
        let v = row as f32 / LATITUDE_SEGMENTS as f32;
        let latitude = std::f32::consts::FRAC_PI_2 - v * std::f32::consts::PI;
        let (sin_latitude, cos_latitude) = latitude.sin_cos();
        for column in 0..=LONGITUDE_SEGMENTS {
            let u = column as f32 / LONGITUDE_SEGMENTS as f32;
            let longitude = u * std::f32::consts::TAU - std::f32::consts::PI;
            let (sin_longitude, cos_longitude) = longitude.sin_cos();
            vertices.extend_from_slice(&[
                cos_latitude * sin_longitude,
                sin_latitude,
                cos_latitude * cos_longitude,
                u,
                v,
            ]);
        }
    }

    let mut indices = Vec::with_capacity(LATITUDE_SEGMENTS * LONGITUDE_SEGMENTS * 6);
    for row in 0..LATITUDE_SEGMENTS {
        for column in 0..LONGITUDE_SEGMENTS {
            let top_left = row * row_width + column;
            let bottom_left = (row + 1) * row_width + column;
            let top_right = top_left + 1;
            let bottom_right = bottom_left + 1;
            indices.extend(
                [
                    top_left,
                    bottom_left,
                    top_right,
                    top_right,
                    bottom_left,
                    bottom_right,
                ]
                .map(|index| {
                    u16::try_from(index)
                        .expect("BUG: sphere vertex index fits in GL_UNSIGNED_SHORT")
                }),
            );
        }
    }
    (vertices, indices)
}

unsafe fn create_sphere_mesh(
    gl: &glow::Context,
    vao: Option<glow::VertexArray>,
) -> Result<(glow::Buffer, glow::Buffer, i32)> {
    let (vertices, indices) = sphere_mesh_data();
    let vbo = unsafe { gl.create_buffer() }.map_err(|e| anyhow::anyhow!("{e}"))?;
    let ibo = match unsafe { gl.create_buffer() } {
        Ok(ibo) => ibo,
        Err(error) => {
            unsafe { gl.delete_buffer(vbo) };
            bail!("{error}");
        }
    };
    unsafe {
        if let Some(vao) = vao {
            gl.bind_vertex_array(Some(vao));
        }
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let vertex_bytes = std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            vertices.len() * std::mem::size_of::<f32>(),
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ibo));
        let index_bytes = std::slice::from_raw_parts(
            indices.as_ptr().cast::<u8>(),
            indices.len() * std::mem::size_of::<u16>(),
        );
        gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, index_bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        if vao.is_some() {
            gl.bind_vertex_array(None);
        } else {
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
        }
    }
    let index_count =
        i32::try_from(indices.len()).expect("BUG: sphere index count fits in GLsizei");
    Ok((vbo, ibo, index_count))
}

/// Create an offscreen FBO with an RGBA color texture attachment.
unsafe fn create_offscreen_fbo(
    gl: &glow::Context,
    width: u32,
    height: u32,
) -> Result<(glow::Framebuffer, glow::Texture)> {
    unsafe {
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

        let caller_state = super::offscreen::OffscreenPassState::capture(gl);
        let fbo = match gl.create_framebuffer() {
            Ok(fbo) => fbo,
            Err(error) => {
                gl.delete_texture(texture);
                bail!("{error}");
            }
        };
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(texture),
            0,
        );
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        caller_state.restore(gl);

        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fbo);
            gl.delete_texture(texture);
            bail!("sphere FBO incomplete: status 0x{status:04X}");
        }

        Ok((fbo, texture))
    }
}

/// Compile and attach vertex + fragment shaders (does NOT link the program).
unsafe fn compile_program(gl: &glow::Context) -> Result<glow::Program> {
    let vs = unsafe { compile_shader(gl, glow::VERTEX_SHADER, VERTEX_SHADER) }?;
    let fs = match unsafe { compile_shader(gl, glow::FRAGMENT_SHADER, FRAGMENT_SHADER) } {
        Ok(s) => s,
        Err(e) => {
            unsafe { gl.delete_shader(vs) };
            return Err(e);
        }
    };

    let program = match unsafe { gl.create_program() } {
        Ok(program) => program,
        Err(error) => {
            unsafe {
                gl.delete_shader(vs);
                gl.delete_shader(fs);
            }
            bail!("{error}");
        }
    };
    unsafe {
        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);

        // Shaders can be deleted after attaching — the program keeps refs.
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
        bail!("sphere {kind_name} shader compile failed: {log}");
    }
    Ok(shader)
}

#[cfg(test)]
mod tests {
    use super::{
        LATITUDE_SEGMENTS, LONGITUDE_SEGMENTS, VERTEX_COMPONENTS, offscreen_dimension,
        projection_zoom, sphere_mesh_data, sphere_rotation,
    };

    fn transform(matrix: [f32; 9], point: [f32; 3]) -> [f32; 3] {
        [
            matrix[0] * point[0] + matrix[3] * point[1] + matrix[6] * point[2],
            matrix[1] * point[0] + matrix[4] * point[1] + matrix[7] * point[2],
            matrix[2] * point[0] + matrix[5] * point[1] + matrix[8] * point[2],
        ]
    }

    fn inverse_transform(matrix: [f32; 9], point: [f32; 3]) -> [f32; 3] {
        [
            matrix[0] * point[0] + matrix[1] * point[1] + matrix[2] * point[2],
            matrix[3] * point[0] + matrix[4] * point[1] + matrix[5] * point[2],
            matrix[6] * point[0] + matrix[7] * point[1] + matrix[8] * point[2],
        ]
    }

    fn position(vertices: &[f32], index: u16) -> [f32; 3] {
        let start = usize::from(index) * VERTEX_COMPONENTS;
        [vertices[start], vertices[start + 1], vertices[start + 2]]
    }

    fn subtract(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
        [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
    }

    fn cross(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
        [
            left[1] * right[2] - left[2] * right[1],
            left[2] * right[0] - left[0] * right[2],
            left[0] * right[1] - left[1] * right[0],
        ]
    }

    fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
        left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
    }

    #[test]
    fn cpu_rotation_matches_the_original_shader_transform() {
        let lat = 0.61_f32;
        let lon = -1.17_f32;
        let point = [0.27_f32, -0.43_f32, 0.86_f32];
        let (sin_lat, cos_lat) = lat.sin_cos();
        let (sin_lon, cos_lon) = lon.sin_cos();
        let rotated_lat = [
            point[0],
            point[1] * cos_lat + point[2] * sin_lat,
            -point[1] * sin_lat + point[2] * cos_lat,
        ];
        let expected = [
            rotated_lat[0] * cos_lon + rotated_lat[2] * sin_lon,
            rotated_lat[1],
            -rotated_lat[0] * sin_lon + rotated_lat[2] * cos_lon,
        ];
        let actual = transform(sphere_rotation(lat, lon), point);

        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() <= f32::EPSILON * 8.0)
        );
    }

    #[test]
    fn projection_zoom_stays_beyond_the_front_vertex() {
        assert!(projection_zoom(0.9) > 1.0);
        assert_eq!(projection_zoom(1.8).to_bits(), 1.8_f32.to_bits());
    }

    #[test]
    fn selected_geographic_center_faces_the_camera() {
        let latitude = 0.61_f32;
        let longitude = -1.17_f32;
        let (sin_latitude, cos_latitude) = latitude.sin_cos();
        let (sin_longitude, cos_longitude) = longitude.sin_cos();
        let center = [
            cos_latitude * sin_longitude,
            sin_latitude,
            cos_latitude * cos_longitude,
        ];
        let actual = inverse_transform(sphere_rotation(latitude, longitude), center);

        assert!(
            actual
                .iter()
                .zip([0.0, 0.0, 1.0])
                .all(|(actual, expected)| (actual - expected).abs() <= f32::EPSILON * 8.0)
        );
    }

    #[test]
    fn sphere_mesh_uses_u16_indices_within_its_vertex_buffer() {
        let (vertices, indices) = sphere_mesh_data();
        let vertex_count = (LATITUDE_SEGMENTS + 1) * (LONGITUDE_SEGMENTS + 1);

        assert_eq!(
            (
                vertices.len(),
                indices.len(),
                usize::from(*indices.iter().max().expect("BUG: sphere mesh has indices"))
            ),
            (
                vertex_count * VERTEX_COMPONENTS,
                LATITUDE_SEGMENTS * LONGITUDE_SEGMENTS * 6,
                vertex_count - 1,
            )
        );
    }

    #[test]
    fn sphere_mesh_never_interpolates_across_the_texture_seam() {
        let (vertices, indices) = sphere_mesh_data();
        let max_span = 1.0 / LONGITUDE_SEGMENTS as f32 + f32::EPSILON;

        assert!(indices.chunks_exact(3).all(|triangle| {
            let mut u = triangle
                .iter()
                .map(|index| vertices[usize::from(*index) * VERTEX_COMPONENTS + 3]);
            let first = u.next().expect("BUG: triangle has three indices");
            let (minimum, maximum) = u.fold((first, first), |(minimum, maximum), value| {
                (minimum.min(value), maximum.max(value))
            });
            maximum - minimum <= max_span
        }));
    }

    #[test]
    fn sphere_mesh_non_degenerate_triangles_face_outwards() {
        let (vertices, indices) = sphere_mesh_data();

        assert!(indices.chunks_exact(3).all(|triangle| {
            let a = position(&vertices, triangle[0]);
            let b = position(&vertices, triangle[1]);
            let c = position(&vertices, triangle[2]);
            let normal = cross(subtract(b, a), subtract(c, a));
            let center = [a[0] + b[0] + c[0], a[1] + b[1] + c[1], a[2] + b[2] + c[2]];
            dot(normal, normal) <= f32::EPSILON || dot(normal, center) > 0.0
        }));
    }

    #[test]
    fn offscreen_resolution_reduces_fill_without_rounding_down_edges() {
        assert_eq!(
            (offscreen_dimension(560), offscreen_dimension(480)),
            (420, 360)
        );
        assert_eq!(offscreen_dimension(0), 0);
        assert_eq!(offscreen_dimension(1), 1);
    }
}
