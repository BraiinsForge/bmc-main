// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! OpenGL ES renderer for flip-clock widget
//!
//! Renders textured quads with perspective projection for the flip-flap animation.

use anyhow::{Context, Result};
use glow::HasContext;

/// Vertex shader source
const VERTEX_SHADER: &str = r"#version 100
attribute vec3 a_position;
attribute vec2 a_texcoord;
uniform mat4 u_mvp;
varying vec2 v_texcoord;

void main() {
    gl_Position = u_mvp * vec4(a_position, 1.0);
    v_texcoord = a_texcoord;
}
";

/// Fragment shader source for textured rendering
const FRAGMENT_SHADER_TEXTURED: &str = r"#version 100
precision mediump float;
varying vec2 v_texcoord;
uniform sampler2D u_texture;

void main() {
    gl_FragColor = texture2D(u_texture, v_texcoord);
}
";

/// Fragment shader source for solid color rendering
const FRAGMENT_SHADER_SOLID: &str = r"#version 100
precision mediump float;
uniform vec4 u_color;

void main() {
    gl_FragColor = u_color;
}
";

/// OpenGL ES renderer
pub struct Renderer {
    /// Shader program for solid colors
    solid_program: glow::Program,
    /// Shader program for textured rendering
    textured_program: glow::Program,
    /// Vertex buffer for quad geometry
    vbo: glow::Buffer,
    /// Current viewport dimensions
    width: u32,
    height: u32,
}

/// A 4x4 matrix stored in column-major order
#[derive(Clone, Copy)]
pub struct Mat4([f32; 16]);

impl Mat4 {
    /// Identity matrix
    #[expect(dead_code)]
    pub fn identity() -> Self {
        Self([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Orthographic projection matrix
    pub fn ortho(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        let tx = -(right + left) / (right - left);
        let ty = -(top + bottom) / (top - bottom);
        let tz = -(far + near) / (far - near);

        Self([
            2.0 / (right - left),
            0.0,
            0.0,
            0.0,
            0.0,
            2.0 / (top - bottom),
            0.0,
            0.0,
            0.0,
            0.0,
            -2.0 / (far - near),
            0.0,
            tx,
            ty,
            tz,
            1.0,
        ])
    }

    /// Perspective projection matrix
    #[expect(dead_code, reason = "available for future use")]
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let f = 1.0 / (fov_y / 2.0).tan();
        let nf = 1.0 / (near - far);

        Self([
            f / aspect,
            0.0,
            0.0,
            0.0,
            0.0,
            f,
            0.0,
            0.0,
            0.0,
            0.0,
            (far + near) * nf,
            -1.0,
            0.0,
            0.0,
            2.0 * far * near * nf,
            0.0,
        ])
    }

    /// Translation matrix
    pub fn translate(x: f32, y: f32, z: f32) -> Self {
        Self([
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, z, 1.0,
        ])
    }

    /// Scale matrix
    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        Self([
            x, 0.0, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 0.0, z, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Rotation matrix around X axis
    pub fn rotate_x(angle: f32) -> Self {
        let c = angle.cos();
        let s = angle.sin();
        Self([
            1.0, 0.0, 0.0, 0.0, 0.0, c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Rotation matrix around Y axis
    pub fn rotate_y(angle: f32) -> Self {
        let c = angle.cos();
        let s = angle.sin();
        Self([
            c, 0.0, -s, 0.0, 0.0, 1.0, 0.0, 0.0, s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Rotation matrix around Z axis
    #[expect(dead_code)]
    pub fn rotate_z(angle: f32) -> Self {
        let c = angle.cos();
        let s = angle.sin();
        Self([
            c, s, 0.0, 0.0, -s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// Multiply two matrices
    pub fn mul(&self, other: &Self) -> Self {
        let mut result = [0.0_f32; 16];
        for i in 0..4 {
            for j in 0..4 {
                for k in 0..4 {
                    result[i * 4 + j] += self.0[k * 4 + j] * other.0[i * 4 + k];
                }
            }
        }
        Self(result)
    }

    /// Get raw array
    pub fn as_array(&self) -> &[f32; 16] {
        &self.0
    }

    /// Extract the upper-left 3x3 matrix for normal transformation
    /// Returns a column-major 3x3 matrix
    pub fn to_normal_matrix(self) -> [f32; 9] {
        [
            self.0[0], self.0[1], self.0[2], // column 0
            self.0[4], self.0[5], self.0[6], // column 1
            self.0[8], self.0[9], self.0[10], // column 2
        ]
    }
}

impl Renderer {
    /// Create a new renderer
    pub fn new(gl: &glow::Context, width: u32, height: u32) -> Result<Self> {
        // Compile shaders and create programs
        let solid_program = Self::create_program(gl, VERTEX_SHADER, FRAGMENT_SHADER_SOLID)
            .context("Failed to create solid shader program")?;

        let textured_program = Self::create_program(gl, VERTEX_SHADER, FRAGMENT_SHADER_TEXTURED)
            .context("Failed to create textured shader program")?;

        // Create vertex buffer for quad (position + texcoord)
        // Quad vertices: 2 triangles forming a rectangle
        // Layout: x, y, z, u, v
        #[rustfmt::skip]
        let vertices: [f32; 30] = [
            // Triangle 1
            -0.5, -0.5, 0.0,  0.0, 1.0,  // bottom-left
             0.5, -0.5, 0.0,  1.0, 1.0,  // bottom-right
             0.5,  0.5, 0.0,  1.0, 0.0,  // top-right
            // Triangle 2
            -0.5, -0.5, 0.0,  0.0, 1.0,  // bottom-left
             0.5,  0.5, 0.0,  1.0, 0.0,  // top-right
            -0.5,  0.5, 0.0,  0.0, 0.0,  // top-left
        ];

        let vbo = unsafe {
            let vbo = gl.create_buffer().map_err(|e| anyhow::anyhow!("{e}"))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&vertices),
                glow::STATIC_DRAW,
            );
            vbo
        };

        tracing::info!("Renderer initialized with shaders");

        Ok(Self {
            solid_program,
            textured_program,
            vbo,
            width,
            height,
        })
    }

    /// Compile a shader
    fn compile_shader(gl: &glow::Context, shader_type: u32, source: &str) -> Result<glow::Shader> {
        unsafe {
            let shader = gl
                .create_shader(shader_type)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);

            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                anyhow::bail!("Shader compilation failed: {log}");
            }

            Ok(shader)
        }
    }

    /// Create a shader program from vertex and fragment shader sources
    fn create_program(gl: &glow::Context, vert_src: &str, frag_src: &str) -> Result<glow::Program> {
        let vert_shader = Self::compile_shader(gl, glow::VERTEX_SHADER, vert_src)?;
        let frag_shader = Self::compile_shader(gl, glow::FRAGMENT_SHADER, frag_src)?;

        unsafe {
            let program = gl.create_program().map_err(|e| anyhow::anyhow!("{e}"))?;
            gl.attach_shader(program, vert_shader);
            gl.attach_shader(program, frag_shader);
            gl.link_program(program);

            // Shaders can be deleted after linking
            gl.delete_shader(vert_shader);
            gl.delete_shader(frag_shader);

            if !gl.get_program_link_status(program) {
                let log = gl.get_program_info_log(program);
                gl.delete_program(program);
                anyhow::bail!("Program linking failed: {log}");
            }

            Ok(program)
        }
    }

    /// Update viewport dimensions
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Get projection matrix for current viewport
    pub fn projection(&self) -> Mat4 {
        // Use orthographic projection - no perspective distortion
        // This ensures all digits look the same regardless of X position
        #[expect(
            clippy::cast_precision_loss,
            reason = "viewport dimensions are small enough"
        )]
        let aspect = self.width as f32 / self.height as f32;

        let half_height = 0.5;
        let half_width = half_height * aspect;
        Mat4::ortho(
            -half_width,
            half_width,
            -half_height,
            half_height,
            -10.0,
            10.0,
        )
    }

    /// Draw a colored rectangle
    pub fn draw_rect(
        &self,
        gl: &glow::Context,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) {
        let projection = self.projection();
        let model = Mat4::translate(x, y, 0.0).mul(&Mat4::scale(width, height, 1.0));
        let mvp = projection.mul(&model);

        unsafe {
            gl.use_program(Some(self.solid_program));

            // Set MVP uniform
            let mvp_loc = gl.get_uniform_location(self.solid_program, "u_mvp");
            gl.uniform_matrix_4_f32_slice(mvp_loc.as_ref(), false, mvp.as_array());

            // Set color uniform
            let color_loc = gl.get_uniform_location(self.solid_program, "u_color");
            gl.uniform_4_f32(color_loc.as_ref(), color[0], color[1], color[2], color[3]);

            // Bind VBO and set up attributes
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let pos_loc = gl
                .get_attrib_location(self.solid_program, "a_position")
                .expect("BUG: a_position attribute not found");
            gl.enable_vertex_attrib_array(pos_loc);
            gl.vertex_attrib_pointer_f32(pos_loc, 3, glow::FLOAT, false, 20, 0);

            let tex_loc = gl.get_attrib_location(self.solid_program, "a_texcoord");
            if let Some(loc) = tex_loc {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, 2, glow::FLOAT, false, 20, 12);
            }

            // Draw quad
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
    }

    /// Draw a colored rectangle with rotation around X axis (for flip animation)
    /// The rotation is around the center of the rectangle
    #[expect(dead_code, clippy::too_many_arguments)]
    pub fn draw_rect_rotated(
        &self,
        gl: &glow::Context,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        angle_x: f32,
        color: [f32; 4],
    ) {
        let projection = self.projection();
        let model = Mat4::translate(x, y, 0.0)
            .mul(&Mat4::rotate_x(angle_x))
            .mul(&Mat4::scale(width, height, 1.0));
        let mvp = projection.mul(&model);

        self.draw_quad_with_mvp(gl, &mvp, color);
    }

    /// Helper to draw a quad with a given MVP matrix and color
    fn draw_quad_with_mvp(&self, gl: &glow::Context, mvp: &Mat4, color: [f32; 4]) {
        unsafe {
            gl.use_program(Some(self.solid_program));

            let mvp_loc = gl.get_uniform_location(self.solid_program, "u_mvp");
            gl.uniform_matrix_4_f32_slice(mvp_loc.as_ref(), false, mvp.as_array());

            let color_loc = gl.get_uniform_location(self.solid_program, "u_color");
            gl.uniform_4_f32(color_loc.as_ref(), color[0], color[1], color[2], color[3]);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let pos_loc = gl
                .get_attrib_location(self.solid_program, "a_position")
                .expect("BUG: a_position attribute not found");
            gl.enable_vertex_attrib_array(pos_loc);
            gl.vertex_attrib_pointer_f32(pos_loc, 3, glow::FLOAT, false, 20, 0);

            let tex_loc = gl.get_attrib_location(self.solid_program, "a_texcoord");
            if let Some(loc) = tex_loc {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, 2, glow::FLOAT, false, 20, 12);
            }

            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
    }

    /// Draw a flipping panel for split-flap animation
    /// The flap rotates around a hinge at y=0 (center of the digit).
    /// - x: center x position
    /// - y: hinge y position (usually 0 for center)
    /// - width, height: size of the flap (half the digit height)
    /// - angle: rotation angle (0 = flat, PI = fully flipped)
    /// - extends_up: if true, flap extends upward from hinge; if false, downward
    #[expect(clippy::too_many_arguments)]
    pub fn draw_flap(
        &self,
        gl: &glow::Context,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        angle: f32,
        extends_up: bool,
        color: [f32; 4],
    ) {
        let projection = self.projection();

        // The flap extends from the hinge either up or down
        let y_offset = if extends_up {
            height / 2.0
        } else {
            -height / 2.0
        };

        // Transform order (right to left):
        // 1. Scale quad to size
        // 2. Translate so hinge edge is at origin
        // 3. Rotate around X axis at origin (the hinge)
        // 4. Translate to final position
        let model = Mat4::translate(x, y, 0.0)
            .mul(&Mat4::rotate_x(angle))
            .mul(&Mat4::translate(0.0, y_offset, 0.0))
            .mul(&Mat4::scale(width, height, 1.0));
        let mvp = projection.mul(&model);

        self.draw_quad_with_mvp(gl, &mvp, color);
    }

    /// Draw a textured rectangle
    pub fn draw_textured_rect(
        &self,
        gl: &glow::Context,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        texture: glow::Texture,
    ) {
        let projection = self.projection();
        let model = Mat4::translate(x, y, 0.0).mul(&Mat4::scale(width, height, 1.0));
        let mvp = projection.mul(&model);

        self.draw_textured_quad_with_mvp(gl, &mvp, texture);
    }

    /// Draw a textured flap for split-flap animation
    /// Same as draw_flap but with texture instead of solid color
    #[expect(clippy::too_many_arguments)]
    pub fn draw_textured_flap(
        &self,
        gl: &glow::Context,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        angle: f32,
        extends_up: bool,
        texture: glow::Texture,
        top_half: bool,
        flip_v: bool,
    ) {
        let projection = self.projection();

        let y_offset = if extends_up {
            height / 2.0
        } else {
            -height / 2.0
        };

        let model = Mat4::translate(x, y, 0.0)
            .mul(&Mat4::rotate_x(angle))
            .mul(&Mat4::translate(0.0, y_offset, 0.0))
            .mul(&Mat4::scale(width, height, 1.0));
        let mvp = projection.mul(&model);

        self.draw_textured_quad_half_with_mvp_flip(gl, &mvp, texture, top_half, flip_v);
    }

    /// Draw a textured half-rectangle (top or bottom half of the texture)
    #[expect(clippy::too_many_arguments)]
    pub fn draw_textured_half_rect(
        &self,
        gl: &glow::Context,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        texture: glow::Texture,
        top_half: bool,
    ) {
        let projection = self.projection();
        let model = Mat4::translate(x, y, 0.0).mul(&Mat4::scale(width, height, 1.0));
        let mvp = projection.mul(&model);

        self.draw_textured_quad_half_with_mvp(gl, &mvp, texture, top_half);
    }

    /// Helper to draw a textured quad with a given MVP matrix
    fn draw_textured_quad_with_mvp(&self, gl: &glow::Context, mvp: &Mat4, texture: glow::Texture) {
        unsafe {
            gl.use_program(Some(self.textured_program));

            let mvp_loc = gl.get_uniform_location(self.textured_program, "u_mvp");
            gl.uniform_matrix_4_f32_slice(mvp_loc.as_ref(), false, mvp.as_array());

            // Bind texture
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let tex_loc = gl.get_uniform_location(self.textured_program, "u_texture");
            gl.uniform_1_i32(tex_loc.as_ref(), 0);

            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));

            let pos_loc = gl
                .get_attrib_location(self.textured_program, "a_position")
                .expect("BUG: a_position attribute not found");
            gl.enable_vertex_attrib_array(pos_loc);
            gl.vertex_attrib_pointer_f32(pos_loc, 3, glow::FLOAT, false, 20, 0);

            let tex_coord_loc = gl
                .get_attrib_location(self.textured_program, "a_texcoord")
                .expect("BUG: a_texcoord attribute not found");
            gl.enable_vertex_attrib_array(tex_coord_loc);
            gl.vertex_attrib_pointer_f32(tex_coord_loc, 2, glow::FLOAT, false, 20, 12);

            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
    }

    /// Helper to draw half of a textured quad (top or bottom half of texture)
    fn draw_textured_quad_half_with_mvp(
        &self,
        gl: &glow::Context,
        mvp: &Mat4,
        texture: glow::Texture,
        top_half: bool,
    ) {
        self.draw_textured_quad_half_with_mvp_flip(gl, mvp, texture, top_half, false);
    }

    /// Helper to draw half of a textured quad with optional vertical flip
    fn draw_textured_quad_half_with_mvp_flip(
        &self,
        gl: &glow::Context,
        mvp: &Mat4,
        texture: glow::Texture,
        top_half: bool,
        flip_v: bool,
    ) {
        // Create custom vertices for half texture
        // Top half uses v=0.0 to 0.5, bottom half uses v=0.5 to 1.0
        let (mut v_start, mut v_end) = if top_half { (0.0, 0.5) } else { (0.5, 1.0) };

        // Flip vertically if requested (swap v coordinates)
        if flip_v {
            std::mem::swap(&mut v_start, &mut v_end);
        }

        #[rustfmt::skip]
        let vertices: [f32; 30] = [
            // Triangle 1
            -0.5, -0.5, 0.0,  0.0, v_end,   // bottom-left
             0.5, -0.5, 0.0,  1.0, v_end,   // bottom-right
             0.5,  0.5, 0.0,  1.0, v_start, // top-right
            // Triangle 2
            -0.5, -0.5, 0.0,  0.0, v_end,   // bottom-left
             0.5,  0.5, 0.0,  1.0, v_start, // top-right
            -0.5,  0.5, 0.0,  0.0, v_start, // top-left
        ];

        unsafe {
            gl.use_program(Some(self.textured_program));

            let mvp_loc = gl.get_uniform_location(self.textured_program, "u_mvp");
            gl.uniform_matrix_4_f32_slice(mvp_loc.as_ref(), false, mvp.as_array());

            // Bind texture
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            let tex_loc = gl.get_uniform_location(self.textured_program, "u_texture");
            gl.uniform_1_i32(tex_loc.as_ref(), 0);

            // Upload custom vertices to VBO
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&vertices),
                glow::DYNAMIC_DRAW,
            );

            let pos_loc = gl
                .get_attrib_location(self.textured_program, "a_position")
                .expect("BUG: a_position attribute not found");
            gl.enable_vertex_attrib_array(pos_loc);
            gl.vertex_attrib_pointer_f32(pos_loc, 3, glow::FLOAT, false, 20, 0);

            let tex_coord_loc = gl
                .get_attrib_location(self.textured_program, "a_texcoord")
                .expect("BUG: a_texcoord attribute not found");
            gl.enable_vertex_attrib_array(tex_coord_loc);
            gl.vertex_attrib_pointer_f32(tex_coord_loc, 2, glow::FLOAT, false, 20, 12);

            gl.draw_arrays(glow::TRIANGLES, 0, 6);

            // Restore original vertices
            #[rustfmt::skip]
            let orig_vertices: [f32; 30] = [
                -0.5, -0.5, 0.0,  0.0, 1.0,
                 0.5, -0.5, 0.0,  1.0, 1.0,
                 0.5,  0.5, 0.0,  1.0, 0.0,
                -0.5, -0.5, 0.0,  0.0, 1.0,
                 0.5,  0.5, 0.0,  1.0, 0.0,
                -0.5,  0.5, 0.0,  0.0, 0.0,
            ];
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&orig_vertices),
                glow::STATIC_DRAW,
            );
        }
    }
}
