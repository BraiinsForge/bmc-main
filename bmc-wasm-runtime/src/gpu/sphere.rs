// Copyright (C) 2026  Braiins Systems s.r.o.

//! GL sphere renderer — ray-sphere + equirectangular texture sampling.
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
attribute vec2 a_pos;
varying vec2 v_uv;
void main() {
    // Negate Y so the FBO content is pre-flipped for femtovg's top-down sampling.
    // GL FBOs have Y=0 at bottom; femtovg expects Y=0 at top.
    v_uv = vec2(a_pos.x, -a_pos.y);
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
";

const FRAGMENT_SHADER: &str = "\
#version 100
precision mediump float;

uniform vec2 u_center;        // (lat_rad, lon_rad)
uniform vec3 u_light_dir;     // light direction (zero-length = no shading)
uniform float u_zoom;         // camera distance (>1.0)
uniform float u_aspect;       // width / height
uniform float u_atmosphere;   // 1.0 = enable limb darkening + edge glow
uniform sampler2D u_texture;

varying vec2 v_uv;

const float PI = 3.14159265;

void main() {
    // Camera at (0, 0, u_zoom) looking at origin
    vec3 ray_dir = normalize(vec3(v_uv.x * u_aspect, v_uv.y, -u_zoom));
    vec3 cam_pos = vec3(0.0, 0.0, u_zoom);

    // Ray-sphere intersection: |cam_pos + t*ray_dir|^2 = 1
    float a = dot(ray_dir, ray_dir);
    float b = 2.0 * dot(cam_pos, ray_dir);
    float c = dot(cam_pos, cam_pos) - 1.0;
    float disc = b * b - 4.0 * a * c;

    if (disc < 0.0) {
        gl_FragColor = vec4(0.0, 0.0, 0.0, 1.0);
        return;
    }

    float t = (-b - sqrt(disc)) / (2.0 * a);
    vec3 hit = cam_pos + t * ray_dir;

    // Undo sphere rotation: view -> geographic via Ry(lon) * Rx(-lat)
    float cos_lat = cos(u_center.x);
    float sin_lat = sin(u_center.x);
    float cos_lon = cos(u_center.y);
    float sin_lon = sin(u_center.y);

    // Rx(-lat)
    vec3 p1 = vec3(
        hit.x,
        hit.y * cos_lat + hit.z * sin_lat,
       -hit.y * sin_lat + hit.z * cos_lat
    );
    // Ry(lon)
    vec3 p2 = vec3(
        p1.x * cos_lon + p1.z * sin_lon,
        p1.y,
       -p1.x * sin_lon + p1.z * cos_lon
    );

    // Geographic coords -> equirectangular UV.
    // This MUST match the texture convention (see tools/texture_render.py):
    //   u = 0 → lon=-180°, u = 0.5 → lon=0° (prime meridian), u = 1 → lon=+180°
    //   v = 0 → lat=+90°  (north pole),  v = 1 → lat=-90° (south pole)
    float lon = atan(p2.x, p2.z);
    float lat = asin(clamp(p2.y, -1.0, 1.0));
    vec2 tex_uv = vec2(lon / (2.0 * PI) + 0.5, 0.5 - lat / PI);

    vec3 tex_color = texture2D(u_texture, tex_uv).rgb;

    // Light shading (terminator) — only when a light direction is provided
    float shade = 1.0;
    if (dot(u_light_dir, u_light_dir) > 0.001) {
        shade = smoothstep(-0.1, 0.15, dot(p2, u_light_dir));
    }
    vec3 color = mix(tex_color * 0.55, tex_color, shade);

    // Atmosphere effects — limb darkening + edge glow (earth-like)
    if (u_atmosphere > 0.5) {
        float rim = 1.0 - max(dot(hit, vec3(0.0, 0.0, 1.0)), 0.0);
        color *= 1.0 - rim * rim * 0.7;
        color += vec3(0.12, 0.22, 0.45) * pow(rim, 1.5);
    }

    gl_FragColor = vec4(color, 1.0);
}
";

/// Epsilon for dirty-checking sphere parameters (≈ 0.06°).
const DIRTY_EPSILON: f32 = 0.001;

// ── SphereRenderer ──────────────────────────────────────────────────

/// Offscreen GL renderer that draws a texture onto a 3D sphere.
///
/// The rendered result lives in an FBO color attachment that is shared with
/// femtovg as a native texture — no pixel readback needed.
pub struct SphereRenderer {
    program: glow::Program,
    vao: Option<glow::VertexArray>,
    vbo: glow::Buffer,
    fbo: glow::Framebuffer,
    #[expect(dead_code)] // kept for potential cleanup
    fbo_texture: glow::Texture,
    texture: Option<glow::Texture>,
    image_id: ImageId,
    width: u32,
    height: u32,
    // Uniform locations
    u_center: glow::UniformLocation,
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
            let program = compile_program(gl)?;

            // Bind a_pos to location 0 before linking (ES 2.0 compatible)
            gl.bind_attrib_location(program, 0, "a_pos");
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                bail!("sphere shader link failed: {log}");
            }

            let get_uniform = |name: &str| -> Result<glow::UniformLocation> {
                gl.get_uniform_location(program, name)
                    .ok_or_else(|| anyhow::anyhow!("missing uniform: {name}"))
            };
            let u_center = get_uniform("u_center")?;
            let u_light_dir = get_uniform("u_light_dir")?;
            let u_zoom = get_uniform("u_zoom")?;
            let u_aspect = get_uniform("u_aspect")?;
            let u_atmosphere = get_uniform("u_atmosphere")?;
            let u_texture = get_uniform("u_texture")?;

            // VAO required on desktop GL core profile and ES 3.0+.
            // Optional on ES 2.0 (extension), so we try and skip if unavailable.
            let vao = gl.create_vertex_array().ok();

            let vbo = create_quad_vbo(gl)?;
            let (fbo, fbo_texture) = create_offscreen_fbo(gl, width, height)?;

            // Register FBO texture with femtovg (zero-copy).
            // Y-flip is handled in the vertex shader (v_uv.y negated) instead of
            // ImageFlags::FLIP_Y, which caused the image to disappear in testing.
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
                vao.is_some()
            );

            Ok(Self {
                program,
                vao,
                vbo,
                fbo,
                fbo_texture,
                texture: None,
                image_id,
                width,
                height,
                u_center,
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

    pub fn has_texture(&self) -> bool {
        self.texture.is_some()
    }

    /// Render the sphere to the offscreen FBO if any parameter changed.
    ///
    /// Does nothing if no texture has been set.
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
    ) {
        let Some(tex) = self.texture else {
            return;
        };

        // Dirty check — skip render if nothing changed
        if !is_dirty(self.last_lat, lat)
            && !is_dirty(self.last_lon, lon)
            && !is_dirty(self.last_zoom, zoom)
            && !is_dirty(self.last_light_lat, light_lat)
            && !is_dirty(self.last_light_lon, light_lon)
            && self.last_atmosphere == atmosphere
        {
            return;
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
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            gl.viewport(0, 0, self.width as i32, self.height as i32);

            // Reset GL state that femtovg's previous flush may have left enabled.
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::STENCIL_TEST);
            gl.color_mask(true, true, true, true);

            gl.use_program(Some(self.program));

            // Uniforms
            gl.uniform_2_f32(Some(&self.u_center), lat_rad, lon_rad);
            gl.uniform_3_f32(
                Some(&self.u_light_dir),
                light_lat_rad.cos() * light_lon_rad.sin(),
                light_lat_rad.sin(),
                light_lat_rad.cos() * light_lon_rad.cos(),
            );
            gl.uniform_1_f32(Some(&self.u_zoom), zoom);
            gl.uniform_1_f32(Some(&self.u_aspect), self.width as f32 / self.height as f32);
            gl.uniform_1_f32(Some(&self.u_atmosphere), if atmosphere { 1.0 } else { 0.0 });

            // Bind texture to sampler unit 0
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.uniform_1_i32(Some(&self.u_texture), 0);

            // Draw fullscreen quad (VAO required on core profile / ES 3.0+)
            if let Some(vao) = self.vao {
                gl.bind_vertex_array(Some(vao));
            }
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 0, 0);

            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

            let err = gl.get_error();
            if err != glow::NO_ERROR {
                tracing::error!("GL error after sphere draw: 0x{err:04X}");
            }

            gl.disable_vertex_attrib_array(0);
            if self.vao.is_some() {
                gl.bind_vertex_array(None);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        }
    }

    /// The femtovg image backed by the offscreen FBO texture.
    pub fn image_id(&self) -> ImageId {
        self.image_id
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn is_dirty(old: f32, new: f32) -> bool {
    old.is_nan() || (old - new).abs() > DIRTY_EPSILON
}

/// Create a VBO with a fullscreen quad (4 verts, triangle strip).
unsafe fn create_quad_vbo(gl: &glow::Context) -> Result<glow::Buffer> {
    let vbo = unsafe { gl.create_buffer() }.map_err(|e| anyhow::anyhow!("{e}"))?;
    unsafe {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let vertices: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
        let bytes: &[u8] = std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&vertices),
        );
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
    }
    Ok(vbo)
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
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);

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

    let program = unsafe { gl.create_program() }.map_err(|e| anyhow::anyhow!("{e}"))?;
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
