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

//! GPU output pass that rewrites the composited RGB frame as BGR565 bytes.
//!
//! The ST7365P panel on BMM products expects red and blue swapped. The plane
//! advertises only `RG16`/`XR24`, so the swap cannot be expressed through the
//! fourcc and must be produced in the pixels. This pass samples the natural-RGB
//! XRGB8888 intermediate (left untouched for the capture path) and writes a
//! `.bgr`-swizzled `RG16` scanout buffer with optional panel color adjustment.

use std::borrow::Cow;

use anyhow::{Context, Result};
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        renderer::{
            Bind, Frame as RendererFrame, ImportDma, Renderer,
            gles::{GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName},
        },
    },
    reexports::drm::control::framebuffer,
    utils::{Buffer as BufferCoord, Rectangle, Size, Transform},
};

use super::buffer_pool::ScanoutFormat;
use super::{BufferPool, DrmOutput};

const SWIZZLE_SHADER: &str = include_str!("scanout_swizzle.frag");

pub struct ScanoutSwizzler {
    buffers: BufferPool,
    pass: SwizzlePass,
}

impl ScanoutSwizzler {
    pub fn new(
        renderer: &mut GlesRenderer,
        width: u32,
        height: u32,
        adjustment: Option<bmc_platform::ColorAdjustment>,
    ) -> Result<Self> {
        Ok(Self {
            buffers: BufferPool::new(width, height, ScanoutFormat::Rgb565),
            pass: SwizzlePass::new(renderer, width, height, adjustment)?,
        })
    }

    /// Apply the optional panel adjustment and write a BGR565 scanout buffer.
    /// Return its framebuffer handle for page-flip.
    pub fn present(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &DrmOutput,
        intermediate: &Dmabuf,
    ) -> Result<framebuffer::Handle> {
        let texture = renderer
            .import_dmabuf(intermediate, None)
            .context("Failed to import composited buffer as swizzle source")?;

        let scanout = self.buffers.back_buffer(output)?;
        let fb = scanout.fb;
        self.pass
            .draw(renderer, &mut scanout.dmabuf, &texture)
            .context("Failed to draw BGR565 scanout buffer")?;

        self.buffers.swap();
        Ok(fb)
    }
}

/// The shader with its uniforms, drawing into any target the renderer can bind.
struct SwizzlePass {
    program: GlesTexProgram,
    uniforms: Vec<Uniform<'static>>,
    width: u32,
    height: u32,
}

impl SwizzlePass {
    fn new(
        renderer: &mut GlesRenderer,
        width: u32,
        height: u32,
        adjustment: Option<bmc_platform::ColorAdjustment>,
    ) -> Result<Self> {
        let uniforms = adjustment_uniforms(adjustment);
        let uniform_names: Vec<_> = uniforms
            .iter()
            .map(|uniform| UniformName::new(uniform.name.clone(), uniform.value.type_()))
            .collect();
        let shader = if let Some(adjustment) = adjustment {
            tracing::info!(?adjustment, "Panel color adjustment");
            Cow::Owned(
                SWIZZLE_SHADER.replace("//_DEFINES_", "#define COLOR_ADJUSTMENT\n//_DEFINES_"),
            )
        } else {
            Cow::Borrowed(SWIZZLE_SHADER)
        };
        let program = renderer
            .compile_custom_texture_shader(shader, &uniform_names)
            .context("Failed to compile BGR565 swizzle shader")?;
        Ok(Self {
            program,
            uniforms,
            width,
            height,
        })
    }

    fn draw<T>(
        &self,
        renderer: &mut GlesRenderer,
        target: &mut T,
        texture: &GlesTexture,
    ) -> Result<()>
    where
        GlesRenderer: Bind<T>,
    {
        let mut framebuffer = renderer
            .bind(target)
            .context("Failed to bind BGR565 scanout target")?;

        #[expect(clippy::cast_possible_wrap)]
        let size = Size::from((self.width as i32, self.height as i32));
        let dst = Rectangle::from_size(size);
        let src: Rectangle<f64, BufferCoord> =
            Rectangle::from_size(Size::from((f64::from(self.width), f64::from(self.height))));

        let mut frame = renderer
            .render(&mut framebuffer, size, Transform::Normal)
            .context("Failed to begin swizzle frame")?;
        frame
            .render_texture_from_to(
                texture,
                src,
                dst,
                &[dst],
                &[],
                Transform::Normal,
                1.0,
                Some(&self.program),
                &self.uniforms,
            )
            .context("Failed to render BGR565 swizzle pass")?;
        let _sync = frame.finish().context("Failed to finish swizzle frame")?;
        Ok(())
    }
}

/// Segments as `slope * x + offset`: `x` is luma on the rise and the peak channel on the fall.
struct GainCurve {
    low_slope: f32,
    low_offset: f32,
    high_slope: f32,
    high_offset: f32,
}

impl GainCurve {
    fn new(value: bmc_platform::ColorAdjustment) -> Self {
        let shadow_floor = value.shadow_floor();
        let low_slope =
            (value.output_anchor() - shadow_floor) / (value.input_anchor() - shadow_floor);
        let high_slope = (1.0 - value.output_anchor()) / (1.0 - value.input_anchor());
        Self {
            low_slope,
            low_offset: shadow_floor * (1.0 - low_slope),
            high_slope,
            high_offset: 1.0 - high_slope,
        }
    }

    #[cfg(test)]
    fn adjust(&self, shadow_floor: f32, rgb: [f32; 3]) -> [f32; 3] {
        let [r, g, b] = rgb;
        let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b).max(shadow_floor);
        let peak = r.max(g).max(b).max(shadow_floor);
        let gain = (self.low_slope + self.low_offset / luma)
            .min(self.high_slope + self.high_offset / peak);
        rgb.map(|channel| channel * gain)
    }
}

fn adjustment_uniforms(adjustment: Option<bmc_platform::ColorAdjustment>) -> Vec<Uniform<'static>> {
    adjustment.map_or_else(Vec::new, |value| {
        let curve = GainCurve::new(value);
        vec![
            Uniform::new("shadow_floor", value.shadow_floor()),
            // One vec4: separate floats measured ~0.5 ms slower per frame on the BMM101 GC400.
            Uniform::new(
                "gain_curve",
                [
                    curve.low_slope,
                    curve.low_offset,
                    curve.high_slope,
                    curve.high_offset,
                ],
            ),
        ]
    })
}

#[cfg(test)]
mod tests {
    use bmc_platform::{HardwareProfile, Product};
    use smithay::backend::{
        allocator::Fourcc,
        egl::{EGLContext, EGLDisplay, native::EGLSurfacelessDisplay},
        renderer::{
            Bind, ExportMem, ImportMem, Offscreen,
            gles::{GlesRenderer, GlesTexture},
        },
    };
    use smithay::utils::{Rectangle, Size};

    use super::{GainCurve, SwizzlePass, adjustment_uniforms};

    #[test]
    fn unadjusted_panels_require_no_color_uniforms() {
        assert!(adjustment_uniforms(None).is_empty());
    }

    #[test]
    fn bmm101_preserves_shadows_and_white_and_lifts_clock_gray() {
        let (curve, floor) = bmm101();
        let mut previous = 0.0;
        for input in 0_u16..=255 {
            let peak = f32::from(input) / 255.0;
            let [output, ..] = curve.adjust(floor, [peak; 3]);
            assert!(
                output >= previous,
                "grayscale hierarchy must remain monotonic"
            );
            assert!(
                output <= 1.0,
                "panel mapping must not clip highlight gradients"
            );
            if peak <= floor || input == 255 {
                assert!(
                    (output - peak).abs() < 1.0 / 255.0,
                    "preserve shadows and white"
                );
            }
            if input == 111 {
                assert!(
                    (output * 255.0 - 198.0).abs() < 1.0,
                    "map Figma Gray 60 to Gray 30"
                );
            }
            previous = output;
        }
    }

    fn bmm101() -> (GainCurve, f32) {
        let adjustment = HardwareProfile::for_product(Product::Bmm101)
            .display
            .color_adjustment
            .expect("BUG: BMM101 must carry its tested readability profile");
        (GainCurve::new(adjustment), adjustment.shadow_floor())
    }

    fn srgb(hex: u32) -> [f32; 3] {
        [16, 8, 0].map(|shift| {
            f32::from(u8::try_from((hex >> shift) & 0xFF).expect("BUG: masked to a byte")) / 255.0
        })
    }

    /// WCAG 2 contrast ratio.
    fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
        let luminance = |rgb: [f32; 3]| {
            let [r, g, b] = rgb.map(|channel| {
                if channel <= 0.040_45 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            });
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn tinted_badges_keep_their_text_contrast() {
        let (curve, floor) = bmm101();
        // ticker-list and bitcoin-mining-data change badges: dark tinted fill, light text
        let badges = [
            (0x10_2B_19, 0x5A_DF_88),
            (0x4F_09_0D, 0xFF_B3_B2),
            (0x0E_3F_25, 0x34_C0_6A),
            (0x51_0B_27, 0xFF_83_A0),
        ];
        for (fill, text) in badges {
            let design = contrast(srgb(fill), srgb(text));
            let shown = contrast(
                curve.adjust(floor, srgb(fill)),
                curve.adjust(floor, srgb(text)),
            );
            assert!(
                shown >= design * (1.0 - 1e-3),
                "lifting a dark fill more than its text erodes the badge: \
                 {fill:06X}/{text:06X} contrast {design:.2} -> {shown:.2}"
            );
        }
    }

    #[test]
    fn light_text_on_gray_80_stays_readable() {
        let (curve, floor) = bmm101();
        let gray_80 = srgb(0x39_39_39);
        for text in [0xFF_FF_FF, 0xF4_F4_F4] {
            let shown = contrast(
                curve.adjust(floor, gray_80),
                curve.adjust(floor, srgb(text)),
            );
            assert!(
                shown >= 7.0,
                "lifting Gray 80 toward its text drops it below WCAG AAA 7:1: {text:06X} {shown:.2}"
            );
        }
    }

    #[test]
    fn lifted_accents_keep_their_hue() {
        let (curve, floor) = bmm101();
        for hex in [0xF4_C0_1A, 0xF9_53_55, 0x28_3C_C8, 0x42_BE_65, 0x5A_DF_88] {
            let rgb = srgb(hex);
            let shown = curve.adjust(floor, rgb);
            assert!(
                shown.iter().all(|channel| *channel <= 1.0),
                "a clipped channel shifts the hue: {hex:06X} -> {shown:?}"
            );
            assert!(
                shown[1] > rgb[1],
                "a bright accent still takes the peak curve's lift: {hex:06X}"
            );
        }
    }

    /// Set by the Nix `ci` profile, which supplies Mesa:
    /// an EGL failure there is a broken profile, and a skip would pass a test that never ran.
    const REQUIRE_EGL: &str = "BMC_REQUIRE_HEADLESS_EGL";

    fn headless_renderer() -> Option<GlesRenderer> {
        let init = || -> anyhow::Result<GlesRenderer> {
            // SAFETY: only smithay creates or terminates EGL displays in this test binary,
            // and it shares one tracked instance between the tests that run in parallel.
            let display = unsafe { EGLDisplay::new(EGLSurfacelessDisplay) }?;
            let context = EGLContext::new(&display)?;
            // SAFETY: the context was created just above and is current on no other thread.
            Ok(unsafe { GlesRenderer::new(context) }?)
        };
        match init() {
            Ok(renderer) => Some(renderer),
            Err(err) => {
                assert!(
                    std::env::var_os(REQUIRE_EGL).is_none(),
                    "{REQUIRE_EGL} is set, so skipping would pass a shader that never compiled: {err:#}"
                );
                eprintln!("skipping: headless EGL init failed, run in the `ci` dev shell: {err:#}");
                None
            }
        }
    }

    /// Draw `probes` as one row through the real shader and return the bytes it writes,
    /// in the swapped order the panel receives.
    fn draw_probes(
        adjustment: Option<bmc_platform::ColorAdjustment>,
        probes: &[u32],
    ) -> Option<Vec<[u8; 3]>> {
        let mut renderer = headless_renderer()?;
        let width = u32::try_from(probes.len()).expect("BUG: a handful of probes");
        let pass = SwizzlePass::new(&mut renderer, width, 1, adjustment)
            .expect("the swizzle shader must compile and link");
        let size = Size::from((i32::try_from(width).expect("BUG: a handful of probes"), 1));
        let pixels: Vec<u8> = probes
            .iter()
            .flat_map(|hex| {
                let [_, r, g, b] = hex.to_be_bytes();
                [r, g, b, 0xFF]
            })
            .collect();
        let texture = renderer
            .import_memory(&pixels, Fourcc::Abgr8888, size, false)
            .expect("BUG: llvmpipe imports RGBA8 textures");
        let mut target: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, size)
            .expect("BUG: llvmpipe renders into RGBA8 textures");
        pass.draw(&mut renderer, &mut target, &texture)
            .expect("the swizzle pass must draw");
        let framebuffer = renderer
            .bind(&mut target)
            .expect("BUG: the target was just rendered into");
        let mapping = renderer
            .copy_framebuffer(&framebuffer, Rectangle::from_size(size), Fourcc::Abgr8888)
            .expect("BUG: llvmpipe reads back RGBA8");
        let bytes = renderer
            .map_texture(&mapping)
            .expect("BUG: a finished copy maps");
        let (written, _) = bytes.as_chunks::<4>();
        Some(written.iter().map(|&[x, y, z, _]| [x, y, z]).collect())
    }

    #[test]
    fn unadjusted_shader_swaps_red_and_blue() {
        let Some(written) = draw_probes(None, &[0x12_34_56, 0xFF_00_00, 0x6F_6F_6F]) else {
            return;
        };
        assert_eq!(
            written,
            [[0x56, 0x34, 0x12], [0x00, 0x00, 0xFF], [0x6F, 0x6F, 0x6F]],
            "the ST7365P expects blue in the red bits and colors stay untouched"
        );
    }

    #[test]
    fn adjusted_shader_matches_the_tested_curve() {
        let (curve, floor) = bmm101();
        let adjustment = HardwareProfile::for_product(Product::Bmm101)
            .display
            .color_adjustment;
        let probes = [
            0x00_00_00, 0x26_26_26, 0x39_39_39, 0x6F_6F_6F, 0xC6_C6_C6, 0xFF_FF_FF, 0x4F_09_0D,
            0xFF_B3_B2, 0x0E_3F_25, 0x34_C0_6A, 0xF4_C0_1A, 0x28_3C_C8,
        ];
        let Some(written) = draw_probes(adjustment, &probes) else {
            return;
        };
        for (hex, [b, g, r]) in probes.into_iter().zip(written) {
            let expected = curve
                .adjust(floor, srgb(hex))
                .map(|channel| channel * 255.0);
            for (shader, rust) in [r, g, b].into_iter().zip(expected) {
                assert!(
                    (f32::from(shader) - rust).abs() <= 1.0,
                    "the GLSL curve drifted from the one the Rust tests check: \
                     {hex:06X} -> {r:02X}{g:02X}{b:02X}, expected {expected:?}"
                );
            }
        }
    }
}
