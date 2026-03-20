// Copyright (C) 2025  Braiins Systems s.r.o.
//
//! Digit texture generation for flip-clock widget using ab_glyph font rendering
//!
//! Generates digit textures (0-9) using a proper font renderer.

use ab_glyph::{FontRef, PxScale};
use anyhow::Result;
use glow::HasContext;

/// Width of each digit texture in pixels
pub const DIGIT_WIDTH: u32 = 128;
/// Height of each digit texture in pixels
pub const DIGIT_HEIGHT: u32 = 256;

/// Embedded font - Braiins Deck Sans Regular (weight 400)
const FONT_DATA: &[u8] = include_bytes!("../../../assets/fonts/BraiinsDeckSans-Regular.otf");

/// Digit textures (0-9)
pub struct DigitTextures {
    /// OpenGL textures for digits 0-9
    textures: [glow::Texture; 10],
}

impl DigitTextures {
    /// Create digit textures
    pub fn new(gl: &glow::Context) -> Result<Self> {
        let font = FontRef::try_from_slice(FONT_DATA)
            .map_err(|e| anyhow::anyhow!("Failed to load font: {e}"))?;

        let mut textures = Vec::with_capacity(10);

        for digit in 0..10 {
            let texture = create_digit_texture(gl, &font, digit)?;
            textures.push(texture);
        }

        let textures: [glow::Texture; 10] = textures
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to create texture array"))?;

        tracing::info!(
            "Created 10 digit textures ({}x{}) using ab_glyph",
            DIGIT_WIDTH,
            DIGIT_HEIGHT
        );

        Ok(Self { textures })
    }

    /// Get texture for a digit (0-9)
    pub fn get(&self, digit: u8) -> glow::Texture {
        self.textures[digit as usize % 10]
    }
}

/// Create a texture for a single digit using font rendering
#[expect(
    clippy::cast_possible_wrap,
    reason = "GL constants and small dimensions fit in i32"
)]
fn create_digit_texture(
    gl: &glow::Context,
    font: &FontRef<'_>,
    digit: u8,
) -> Result<glow::Texture> {
    // Generate pixel data for the digit
    let pixels = render_digit_to_pixels(font, digit);

    unsafe {
        let texture = gl
            .create_texture()
            .map_err(|e| anyhow::anyhow!("Failed to create texture: {e}"))?;

        gl.bind_texture(glow::TEXTURE_2D, Some(texture));

        // Upload texture data
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA as i32,
            DIGIT_WIDTH as i32,
            DIGIT_HEIGHT as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&pixels)),
        );

        // Set texture parameters
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

        Ok(texture)
    }
}

/// Render a digit character to RGBA pixel data using ab_glyph
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "pixel coordinates and font metrics are small positive values"
)]
fn render_digit_to_pixels(font: &FontRef<'_>, digit: u8) -> Vec<u8> {
    use ab_glyph::{Font, ScaleFont};

    let mut pixels = vec![0_u8; (DIGIT_WIDTH * DIGIT_HEIGHT * 4) as usize];

    // Scale font to fit the texture height with some padding
    let scale = PxScale::from(DIGIT_HEIGHT as f32 * 0.85);
    let scaled_font = font.as_scaled(scale);

    // Get the character to render
    let c = char::from_digit(u32::from(digit), 10).unwrap_or('0');
    let glyph_id = font.glyph_id(c);

    // Get glyph metrics for positioning
    let h_advance = scaled_font.h_advance(glyph_id);
    let ascent = scaled_font.ascent();

    // Center the glyph horizontally and vertically
    let x_offset = (DIGIT_WIDTH as f32 - h_advance) / 2.0;
    let y_offset = f32::midpoint(DIGIT_HEIGHT as f32, ascent);

    // Create positioned glyph
    let glyph = ab_glyph::Glyph {
        id: glyph_id,
        scale,
        position: ab_glyph::point(x_offset, y_offset),
    };

    // Render the glyph
    if let Some(outlined) = font.outline_glyph(glyph) {
        let bounds = outlined.px_bounds();

        outlined.draw(|px, py, coverage| {
            let x = px as i32 + bounds.min.x as i32;
            let y = py as i32 + bounds.min.y as i32;

            if x >= 0 && x < DIGIT_WIDTH as i32 && y >= 0 && y < DIGIT_HEIGHT as i32 {
                // Flip Y coordinate for OpenGL (origin at bottom-left)
                let flipped_y = DIGIT_HEIGHT as i32 - 1 - y;
                let idx = ((flipped_y as u32 * DIGIT_WIDTH + x as u32) * 4) as usize;
                let alpha = (coverage * 255.0) as u8;

                // White text with alpha
                pixels[idx] = 255; // R
                pixels[idx + 1] = 255; // G
                pixels[idx + 2] = 255; // B
                pixels[idx + 3] = alpha; // A
            }
        });
    }

    pixels
}
