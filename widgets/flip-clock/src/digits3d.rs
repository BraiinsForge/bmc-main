// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! 3D extruded digit mesh generation for flip-clock widget
//!
//! Uses lyon for tessellation of font outlines, then extrudes to create 3D geometry.

use ab_glyph::{Font, FontRef, GlyphId, OutlinedGlyph, PxScale};
use anyhow::Result;
use glow::HasContext;
use lyon::geom::point;
use lyon::path::Path;
use lyon::tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, VertexBuffers};

/// Extrusion depth for 3D digits (in normalized coordinates)
/// Larger value = thicker digits visible from the side
const EXTRUSION_DEPTH: f32 = 0.35;

/// Embedded font - Braiins Deck Sans Regular (weight 400)
/// Note: OpenType features (ss04, liga) are not applied via ab_glyph
/// We normalize digit geometry to fit in a -0.5 to 0.5 range (unit cube centered at origin)
const FONT_DATA: &[u8] =
    include_bytes!("../../../bmc-display/ui/assets/fonts/BraiinsDeckSans-Regular.otf");

/// Vertex with position and normal for 3D rendering
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub struct Vertex3D {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// A 3D mesh for a single digit
pub struct DigitMesh {
    /// Vertex buffer object
    pub vbo: glow::Buffer,
    /// Total vertex count
    pub total_vertex_count: i32,
}

/// 3D digit meshes (0-9)
pub struct Digit3DMeshes {
    /// Meshes for digits 0-9
    meshes: [DigitMesh; 10],
    /// Shader program for 3D rendering
    pub program: glow::Program,
}

impl Digit3DMeshes {
    /// Create 3D digit meshes
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let font = FontRef::try_from_slice(FONT_DATA)
            .map_err(|e| anyhow::anyhow!("Failed to load font: {e}"))?;

        let mut meshes = Vec::with_capacity(10);

        for digit in 0..10_u8 {
            let mesh = create_digit_mesh(gl, &font, digit)?;
            meshes.push(mesh);
        }

        let meshes: [DigitMesh; 10] = meshes
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to create mesh array"))?;

        // Create shader program for 3D lit rendering
        let program = create_3d_shader(gl)?;

        tracing::info!("Created 10 3D digit meshes with extrusion");

        Ok(Self { meshes, program })
    }

    /// Get mesh for a digit (0-9)
    pub fn get(&self, digit: u8) -> &DigitMesh {
        &self.meshes[digit as usize % 10]
    }

    /// Draw a 3D digit at the specified position with transformation
    pub fn draw_digit(
        &self,
        gl: &glow::Context,
        digit: u8,
        mvp: &[f32; 16],
        normal_matrix: &[f32; 9],
        color: [f32; 3],
        light_dir: [f32; 3],
    ) {
        let mesh = self.get(digit);

        unsafe {
            gl.use_program(Some(self.program));

            // Set uniforms
            let mvp_loc = gl.get_uniform_location(self.program, "u_mvp");
            gl.uniform_matrix_4_f32_slice(mvp_loc.as_ref(), false, mvp);

            let normal_loc = gl.get_uniform_location(self.program, "u_normal_matrix");
            gl.uniform_matrix_3_f32_slice(normal_loc.as_ref(), false, normal_matrix);

            let color_loc = gl.get_uniform_location(self.program, "u_color");
            gl.uniform_3_f32(color_loc.as_ref(), color[0], color[1], color[2]);

            let light_loc = gl.get_uniform_location(self.program, "u_light_dir");
            gl.uniform_3_f32(light_loc.as_ref(), light_dir[0], light_dir[1], light_dir[2]);

            // Bind VBO and set up attributes
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(mesh.vbo));

            let pos_loc = gl
                .get_attrib_location(self.program, "a_position")
                .expect("BUG: a_position attribute not found");
            gl.enable_vertex_attrib_array(pos_loc);
            gl.vertex_attrib_pointer_f32(pos_loc, 3, glow::FLOAT, false, 24, 0);

            let normal_attrib_loc = gl
                .get_attrib_location(self.program, "a_normal")
                .expect("BUG: a_normal attribute not found");
            gl.enable_vertex_attrib_array(normal_attrib_loc);
            gl.vertex_attrib_pointer_f32(normal_attrib_loc, 3, glow::FLOAT, false, 24, 12);

            // Draw all faces
            gl.draw_arrays(glow::TRIANGLES, 0, mesh.total_vertex_count);
        }
    }
}

/// Vertex shader for 3D lit rendering
const VERTEX_SHADER_3D: &str = r"#version 100
attribute vec3 a_position;
attribute vec3 a_normal;
uniform mat4 u_mvp;
uniform mat3 u_normal_matrix;
varying vec3 v_normal;
varying vec3 v_position;

void main() {
    gl_Position = u_mvp * vec4(a_position, 1.0);
    v_normal = u_normal_matrix * a_normal;
    v_position = a_position;
}
";

/// Fragment shader for 3D lit rendering with different colors for front/side faces
const FRAGMENT_SHADER_3D: &str = r"#version 100
precision mediump float;
varying vec3 v_normal;
varying vec3 v_position;
uniform vec3 u_color;
uniform vec3 u_light_dir;

void main() {
    vec3 normal = normalize(v_normal);
    vec3 light = normalize(u_light_dir);

    // Determine if this is a front/back face (normal pointing in Z) or side face
    float is_side = 1.0 - abs(normal.z);

    // Side faces are darker gray, front faces are the main color
    vec3 side_color = vec3(0.4, 0.4, 0.4); // gray for sides
    vec3 face_color = mix(u_color, side_color, is_side);

    // Apply lighting
    float diffuse = abs(dot(normal, light));
    float ambient = 0.3;
    float intensity = ambient + diffuse * 0.7;

    gl_FragColor = vec4(face_color * intensity, 1.0);
}
";

/// Create shader program for 3D rendering
fn create_3d_shader(gl: &glow::Context) -> Result<glow::Program> {
    unsafe {
        let vert = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.shader_source(vert, VERTEX_SHADER_3D);
        gl.compile_shader(vert);
        if !gl.get_shader_compile_status(vert) {
            let log = gl.get_shader_info_log(vert);
            anyhow::bail!("Vertex shader error: {log}");
        }

        let frag = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.shader_source(frag, FRAGMENT_SHADER_3D);
        gl.compile_shader(frag);
        if !gl.get_shader_compile_status(frag) {
            let log = gl.get_shader_info_log(frag);
            anyhow::bail!("Fragment shader error: {log}");
        }

        let program = gl.create_program().map_err(|e| anyhow::anyhow!("{e}"))?;
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.link_program(program);
        gl.delete_shader(vert);
        gl.delete_shader(frag);

        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            anyhow::bail!("Program link error: {log}");
        }

        Ok(program)
    }
}

/// Create a 3D mesh for a single digit
fn create_digit_mesh(gl: &glow::Context, font: &FontRef<'_>, digit: u8) -> Result<DigitMesh> {
    // Get glyph outline
    let c = char::from_digit(u32::from(digit), 10).unwrap_or('0');
    let glyph_id = font.glyph_id(c);
    let scale = PxScale::from(200.0);

    let glyph = ab_glyph::Glyph {
        id: glyph_id,
        scale,
        position: ab_glyph::point(0.0, 0.0),
    };

    let outlined = font
        .outline_glyph(glyph)
        .ok_or_else(|| anyhow::anyhow!("No outline for digit {digit}"))?;

    // Convert glyph outline to lyon path
    let path = glyph_to_lyon_path(&outlined, font, glyph_id, scale);

    // Tessellate the path
    let mut geometry: VertexBuffers<[f32; 2], u16> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();

    tessellator
        .tessellate_path(
            &path,
            &FillOptions::default(),
            &mut BuffersBuilder::new(&mut geometry, |vertex: FillVertex<'_>| {
                vertex.position().to_array()
            }),
        )
        .map_err(|e| anyhow::anyhow!("Tessellation failed: {e:?}"))?;

    tracing::info!(
        "Digit {} tessellation: {} vertices, {} indices",
        digit,
        geometry.vertices.len(),
        geometry.indices.len()
    );

    // Calculate bounding box for centering
    let (min_x, max_x, min_y, max_y) = calculate_bounds(&geometry.vertices);
    let center_x = (min_x + max_x) / 2.0;
    let center_y = (min_y + max_y) / 2.0;
    let width = max_x - min_x;
    let height = max_y - min_y;
    let max_dim = width.max(height);

    // Build 3D vertices: front face, back face, and sides
    // Normalize vertices to -0.5 to 0.5 range (unit cube centered at origin)
    let mut vertices: Vec<Vertex3D> = Vec::new();

    // Front face (z = depth/2) - normal pointing forward
    for i in (0..geometry.indices.len()).step_by(3) {
        for &idx in &geometry.indices[i..i + 3] {
            let [x, y] = geometry.vertices[idx as usize];
            // Normalize to -0.5 to 0.5 range
            let nx = (x - center_x) / max_dim;
            let ny = (y - center_y) / max_dim;
            vertices.push(Vertex3D {
                position: [nx, -ny, EXTRUSION_DEPTH / 2.0],
                normal: [0.0, 0.0, 1.0],
            });
        }
    }

    // Back face (z = -depth/2) - normal pointing backward, reverse winding
    for i in (0..geometry.indices.len()).step_by(3) {
        for &idx in geometry.indices[i..i + 3].iter().rev() {
            let [x, y] = geometry.vertices[idx as usize];
            let nx = (x - center_x) / max_dim;
            let ny = (y - center_y) / max_dim;
            vertices.push(Vertex3D {
                position: [nx, -ny, -EXTRUSION_DEPTH / 2.0],
                normal: [0.0, 0.0, -1.0],
            });
        }
    }

    // Side faces - connect front and back edges
    add_side_faces(&mut vertices, &path, center_x, center_y, max_dim);

    // Create VBO
    let vbo = unsafe {
        let vbo = gl
            .create_buffer()
            .map_err(|e| anyhow::anyhow!("Failed to create VBO: {e}"))?;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        gl.buffer_data_u8_slice(
            glow::ARRAY_BUFFER,
            bytemuck::cast_slice(&vertices),
            glow::STATIC_DRAW,
        );
        vbo
    };

    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    Ok(DigitMesh {
        vbo,
        total_vertex_count: vertices.len() as i32,
    })
}

/// Convert ab_glyph outline to lyon path
fn glyph_to_lyon_path(
    outlined: &OutlinedGlyph,
    font: &FontRef<'_>,
    glyph_id: GlyphId,
    scale: PxScale,
) -> Path {
    let bounds = outlined.px_bounds();

    let mut builder = lyon::path::Path::builder();

    // Get the raw outline from the font (unscaled)
    if let Some(outline) = font.outline(glyph_id) {
        // Scale factor to convert from font units to pixels
        let scale_factor = scale.x / font.units_per_em().unwrap_or(1000.0);

        // Track if we're in a contour
        let mut in_contour = false;
        let mut last_point: Option<lyon::geom::Point<f32>> = None;

        for curve in outline.curves {
            match curve {
                ab_glyph::OutlineCurve::Line(p0, p1) => {
                    let start = point(p0.x * scale_factor, p0.y * scale_factor);
                    let end = point(p1.x * scale_factor, p1.y * scale_factor);

                    // Check if this starts a new contour
                    if !in_contour
                        || last_point.is_none_or(|lp| {
                            (lp.x - start.x).abs() > 0.01 || (lp.y - start.y).abs() > 0.01
                        })
                    {
                        // Close previous contour if any
                        if in_contour {
                            builder.close();
                        }
                        builder.begin(start);
                        in_contour = true;
                    }
                    builder.line_to(end);
                    last_point = Some(end);
                }
                ab_glyph::OutlineCurve::Quad(p0, p1, p2) => {
                    let start = point(p0.x * scale_factor, p0.y * scale_factor);
                    let ctrl = point(p1.x * scale_factor, p1.y * scale_factor);
                    let end = point(p2.x * scale_factor, p2.y * scale_factor);

                    if !in_contour
                        || last_point.is_none_or(|lp| {
                            (lp.x - start.x).abs() > 0.01 || (lp.y - start.y).abs() > 0.01
                        })
                    {
                        if in_contour {
                            builder.close();
                        }
                        builder.begin(start);
                        in_contour = true;
                    }
                    builder.quadratic_bezier_to(ctrl, end);
                    last_point = Some(end);
                }
                ab_glyph::OutlineCurve::Cubic(p0, p1, p2, p3) => {
                    let start = point(p0.x * scale_factor, p0.y * scale_factor);
                    let ctrl1 = point(p1.x * scale_factor, p1.y * scale_factor);
                    let ctrl2 = point(p2.x * scale_factor, p2.y * scale_factor);
                    let end = point(p3.x * scale_factor, p3.y * scale_factor);

                    if !in_contour
                        || last_point.is_none_or(|lp| {
                            (lp.x - start.x).abs() > 0.01 || (lp.y - start.y).abs() > 0.01
                        })
                    {
                        if in_contour {
                            builder.close();
                        }
                        builder.begin(start);
                        in_contour = true;
                    }
                    builder.cubic_bezier_to(ctrl1, ctrl2, end);
                    last_point = Some(end);
                }
            }
        }

        // Close final contour
        if in_contour {
            builder.close();
        }
    } else {
        // Fallback: simple rectangle
        let w = bounds.max.x - bounds.min.x;
        let h = bounds.max.y - bounds.min.y;

        builder.begin(point(0.0, 0.0));
        builder.line_to(point(w, 0.0));
        builder.line_to(point(w, h));
        builder.line_to(point(0.0, h));
        builder.close();
    }

    builder.build()
}

/// Calculate bounding box of vertices
fn calculate_bounds(vertices: &[[f32; 2]]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;

    for &[x, y] in vertices {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }

    (min_x, max_x, min_y, max_y)
}

/// Add side faces by extruding path edges
fn add_side_faces(
    vertices: &mut Vec<Vertex3D>,
    path: &Path,
    center_x: f32,
    center_y: f32,
    max_dim: f32,
) {
    use lyon::path::PathEvent;

    let mut last_point: Option<lyon::geom::Point<f32>> = None;
    let mut first_point: Option<lyon::geom::Point<f32>> = None;

    for event in path {
        match event {
            PathEvent::Begin { at } => {
                first_point = Some(at);
                last_point = Some(at);
            }
            PathEvent::Line { to, .. } => {
                if let Some(from) = last_point {
                    add_side_quad(vertices, from, to, center_x, center_y, max_dim);
                }
                last_point = Some(to);
            }
            PathEvent::Quadratic { ctrl, to, .. } => {
                // Approximate curve with line segments
                if let Some(from) = last_point {
                    let steps: u8 = 8;
                    let mut prev = from;
                    for i in 1..=steps {
                        let t = f32::from(i) / f32::from(steps);
                        let p = quadratic_bezier(from, ctrl, to, t);
                        add_side_quad(vertices, prev, p, center_x, center_y, max_dim);
                        prev = p;
                    }
                }
                last_point = Some(to);
            }
            PathEvent::Cubic {
                ctrl1, ctrl2, to, ..
            } => {
                // Approximate curve with line segments
                if let Some(from) = last_point {
                    let steps: u8 = 12;
                    let mut prev = from;
                    for i in 1..=steps {
                        let t = f32::from(i) / f32::from(steps);
                        let p = cubic_bezier(from, ctrl1, ctrl2, to, t);
                        add_side_quad(vertices, prev, p, center_x, center_y, max_dim);
                        prev = p;
                    }
                }
                last_point = Some(to);
            }
            PathEvent::End { close, .. } => {
                if close {
                    if let (Some(from), Some(to)) = (last_point, first_point) {
                        add_side_quad(vertices, from, to, center_x, center_y, max_dim);
                    }
                }
                last_point = None;
                first_point = None;
            }
        }
    }
}

/// Add a side quad between two edge points
fn add_side_quad(
    vertices: &mut Vec<Vertex3D>,
    from: lyon::geom::Point<f32>,
    to: lyon::geom::Point<f32>,
    center_x: f32,
    center_y: f32,
    max_dim: f32,
) {
    // Normalize to -0.5 to 0.5 range (same as front/back faces)
    let x0 = (from.x - center_x) / max_dim;
    let y0 = -(from.y - center_y) / max_dim;
    let x1 = (to.x - center_x) / max_dim;
    let y1 = -(to.y - center_y) / max_dim;

    let z_front = EXTRUSION_DEPTH / 2.0;
    let z_back = -EXTRUSION_DEPTH / 2.0;

    // Calculate normal (perpendicular to edge, pointing outward)
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    let nx = -dy / len;
    let ny = dx / len;

    // Two triangles for the quad
    // Triangle 1: front-from, back-from, front-to
    vertices.push(Vertex3D {
        position: [x0, y0, z_front],
        normal: [nx, ny, 0.0],
    });
    vertices.push(Vertex3D {
        position: [x0, y0, z_back],
        normal: [nx, ny, 0.0],
    });
    vertices.push(Vertex3D {
        position: [x1, y1, z_front],
        normal: [nx, ny, 0.0],
    });

    // Triangle 2: front-to, back-from, back-to
    vertices.push(Vertex3D {
        position: [x1, y1, z_front],
        normal: [nx, ny, 0.0],
    });
    vertices.push(Vertex3D {
        position: [x0, y0, z_back],
        normal: [nx, ny, 0.0],
    });
    vertices.push(Vertex3D {
        position: [x1, y1, z_back],
        normal: [nx, ny, 0.0],
    });
}

/// Evaluate quadratic bezier at t
fn quadratic_bezier(
    p0: lyon::geom::Point<f32>,
    p1: lyon::geom::Point<f32>,
    p2: lyon::geom::Point<f32>,
    t: f32,
) -> lyon::geom::Point<f32> {
    let mt = 1.0 - t;
    point(
        mt * mt * p0.x + 2.0 * mt * t * p1.x + t * t * p2.x,
        mt * mt * p0.y + 2.0 * mt * t * p1.y + t * t * p2.y,
    )
}

/// Evaluate cubic bezier at t
fn cubic_bezier(
    p0: lyon::geom::Point<f32>,
    p1: lyon::geom::Point<f32>,
    p2: lyon::geom::Point<f32>,
    p3: lyon::geom::Point<f32>,
    t: f32,
) -> lyon::geom::Point<f32> {
    let mt = 1.0 - t;
    let mt2 = mt * mt;
    let mt3 = mt2 * mt;
    let t2 = t * t;
    let t3 = t2 * t;
    point(
        mt3 * p0.x + 3.0 * mt2 * t * p1.x + 3.0 * mt * t2 * p2.x + t3 * p3.x,
        mt3 * p0.y + 3.0 * mt2 * t * p1.y + 3.0 * mt * t2 * p2.y + t3 * p3.y,
    )
}
