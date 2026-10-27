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

//! The logo and caption a product draws on scenes with no rendered content.

use bmc_platform::{DisplayTransform, Product};
use bmc_shared_utils::include_png;
use smithay::backend::renderer::{
    Texture,
    gles::{GlesFrame, GlesRenderer, GlesTexture},
};

use super::{OverlayTexture, blit_texture_on_frame};
use crate::compositor::render::DrmOutput;

// Embedded so there is no runtime file dependency.
const DECK_LOGO_PNG: &[u8] = include_png!("../../assets/deck_logo.png");
const BMM_LOGO_PNG: &[u8] = include_png!("../../assets/bmm_logo.png");
/// "Braiins BMM 101 / Powered by Braiins OS", under the logo on the BMM101 empty scene.
const BMM101_EMPTY_SCENE_TEXT_PNG: &[u8] = include_png!("../../assets/bmm101_empty_scene_text.png");
/// "Loading scene…", under the logo while a configured scene has not painted its first frame.
const LOADING_SCENE_TEXT_PNG: &[u8] = include_png!("../../assets/loading_scene_text.png");

/// A product's logo and empty-scene caption, and where they sit on the panel.
#[derive(Debug, Clone, Copy)]
pub struct Branding {
    logo_png: &'static [u8],
    /// Logical top edge of the logo, or `None` to center it vertically.
    logo_top: Option<i32>,
    /// Logical gap between the logo's bottom edge and the caption's top edge.
    caption_gap: i32,
    /// Caption under the logo on the empty scene, or `None` for the logo alone.
    empty_scene_caption_png: Option<&'static [u8]>,
}

impl Branding {
    /// Branding a product shows in scenes with no rendered content,
    /// or `None` for the plain cleared background, which also drops the "Loading scene…" caption.
    ///
    /// The BMM101 places its logo and empty-scene caption where its kernel boot logo has them
    /// (`logo_bmm101_clut224.ppm` in linux-stm, drawn at the panel's top-left corner),
    /// so the empty scene takes over from the boot screen without anything moving.
    /// The BMM100 has no Braiins boot logo to line up with, and the BFM100 is not branded yet.
    #[must_use]
    pub fn for_product(product: Product) -> Option<Self> {
        match product {
            Product::Bmc100 => Some(Self {
                logo_png: DECK_LOGO_PNG,
                logo_top: None,
                caption_gap: 42,
                empty_scene_caption_png: None,
            }),
            Product::Bmm101 => Some(Self {
                logo_png: BMM_LOGO_PNG,
                logo_top: Some(80),
                caption_gap: 24,
                empty_scene_caption_png: Some(BMM101_EMPTY_SCENE_TEXT_PNG),
            }),
            Product::Bmm100 | Product::Bfm100 => None,
        }
    }

    /// The image of `caption`, or `None` when this product has no such caption.
    fn caption_png(&self, caption: LogoCaption) -> Option<&'static [u8]> {
        match caption {
            LogoCaption::Empty => self.empty_scene_caption_png,
            LogoCaption::Loading => Some(LOADING_SCENE_TEXT_PNG),
        }
    }

    /// Logical top-left corner of a `logo`-sized logo on a `panel`-sized scene.
    fn logo_origin(&self, panel: (i32, i32), logo: (i32, i32)) -> (i32, i32) {
        let (panel_w, panel_h) = panel;
        let (logo_w, logo_h) = logo;
        let top = self
            .logo_top
            .unwrap_or_else(|| centered_start(panel_h, logo_h));
        (centered_start(panel_w, logo_w), top)
    }

    /// Logical top-left corner of a `caption`-sized caption,
    /// centered on the panel and below the `logo`-sized logo.
    fn caption_origin(
        &self,
        panel: (i32, i32),
        logo: (i32, i32),
        caption: (i32, i32),
    ) -> (i32, i32) {
        let (_, logo_top) = self.logo_origin(panel, logo);
        let (_, logo_h) = logo;
        let (caption_w, _) = caption;
        (
            centered_start(panel.0, caption_w),
            logo_top + logo_h + self.caption_gap,
        )
    }
}

/// Start coordinate (left/top edge along one axis) at which `content` sits
/// centered inside `container`; the 1px bias from integer division is fine
/// for the overlays this positions.
#[expect(
    clippy::integer_division,
    reason = "1px rounding when centering is fine"
)]
fn centered_start(container: i32, content: i32) -> i32 {
    (container - content) / 2
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LogoCaption {
    /// The product's empty-scene caption, on the no-scene sentinel.
    Empty,
    /// "Loading scene…", on a configured scene that has not painted yet.
    Loading,
}

impl LogoCaption {
    /// Names the caption in log messages.
    fn label(self) -> &'static str {
        match self {
            Self::Empty => "empty-scene caption",
            Self::Loading => "loading caption",
        }
    }
}

/// A scene showing the branding logo this frame:
/// the x-offset it slides with during swipes and transitions,
/// the opacity it fades with during cross-fades,
/// and the caption below the logo, which a product without that caption's image leaves out.
pub(super) struct LogoScene {
    pub(super) x_offset: i32,
    pub(super) alpha: f32,
    pub(super) caption: LogoCaption,
}

/// A product's [`Branding`] and its textures, each held only while an on-screen scene draws it.
pub(super) struct BrandingOverlay {
    branding: Branding,
    logo: OverlayTexture,
    empty_scene_caption: OverlayTexture,
    loading_caption: OverlayTexture,
}

impl BrandingOverlay {
    pub(super) fn new(branding: Branding) -> Self {
        Self {
            branding,
            logo: OverlayTexture::Unloaded,
            empty_scene_caption: OverlayTexture::Unloaded,
            loading_caption: OverlayTexture::Unloaded,
        }
    }

    /// Load the textures `scenes` draw and drop the rest,
    /// so the GPU only holds them while a "no content yet" scene is on screen.
    pub(super) fn sync(&mut self, renderer: &mut GlesRenderer, scenes: &[LogoScene]) {
        if scenes.is_empty() {
            self.logo.unload();
        } else {
            self.logo
                .ensure_loaded(renderer, self.branding.logo_png, "branding logo");
        }
        for caption in [LogoCaption::Empty, LogoCaption::Loading] {
            if let Some(png) = self.branding.caption_png(caption) {
                let slot = self.caption_slot_mut(caption);
                if scenes.iter().any(|s| s.caption == caption) {
                    slot.ensure_loaded(renderer, png, caption.label());
                } else {
                    slot.unload();
                }
            }
        }
    }

    /// Draw the logo, with each scene's caption below it, at every scene's x-offset and opacity.
    pub(super) fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        output: &DrmOutput,
        transform: DisplayTransform,
        scenes: &[LogoScene],
    ) {
        let Some(logo) = self.logo.texture() else {
            return;
        };
        let (lw, lh) = output.logical_size();
        let to_i32 = |v: u32| i32::try_from(v).expect("BUG: panel geometry fits i32");
        let output_w = to_i32(output.width());
        let output_h = to_i32(output.height());
        let logical_w = to_i32(lw);
        let logical_h = to_i32(lh);
        let branding = &self.branding;
        let panel = (logical_w, logical_h);
        let logo_size = (logo.size().w, logo.size().h);
        let (logo_x, logo_y) = branding.logo_origin(panel, logo_size);
        for scene in scenes {
            blit_texture_on_frame(
                frame,
                logo,
                scene.x_offset + logo_x,
                logo_y,
                None,
                output_w,
                output_h,
                transform,
                scene.alpha,
                "branding logo",
            );

            if let Some(texture) = self.caption_texture(scene.caption) {
                let caption_size = (texture.size().w, texture.size().h);
                let (caption_x, caption_y) =
                    branding.caption_origin(panel, logo_size, caption_size);
                blit_texture_on_frame(
                    frame,
                    texture,
                    scene.x_offset + caption_x,
                    caption_y,
                    None,
                    output_w,
                    output_h,
                    transform,
                    scene.alpha,
                    scene.caption.label(),
                );
            }
        }
    }

    fn caption_texture(&self, caption: LogoCaption) -> Option<&GlesTexture> {
        match caption {
            LogoCaption::Empty => self.empty_scene_caption.texture(),
            LogoCaption::Loading => self.loading_caption.texture(),
        }
    }

    fn caption_slot_mut(&mut self, caption: LogoCaption) -> &mut OverlayTexture {
        match caption {
            LogoCaption::Empty => &mut self.empty_scene_caption,
            LogoCaption::Loading => &mut self.loading_caption,
        }
    }
}

#[cfg(test)]
mod tests {
    use bmc_platform::{DisplayTransform, HardwareProfile, Product};
    use smithay::backend::renderer::{Texture, gles::GlesTexture};

    use super::{
        BMM_LOGO_PNG, BMM101_EMPTY_SCENE_TEXT_PNG, Branding, BrandingOverlay, DECK_LOGO_PNG,
        LOADING_SCENE_TEXT_PNG, LogoCaption, LogoScene,
    };
    use crate::compositor::render::headless_egl::headless_renderer;

    /// Every product. The match stops compiling on a new variant, as a reminder to list it.
    fn all_products() -> [Product; 4] {
        let all = [
            Product::Bmc100,
            Product::Bmm100,
            Product::Bmm101,
            Product::Bfm100,
        ];
        for product in all {
            match product {
                Product::Bmc100 | Product::Bmm100 | Product::Bmm101 | Product::Bfm100 => {}
            }
        }
        all
    }

    fn branding(product: Product) -> Branding {
        Branding::for_product(product).expect("BUG: the test picked a product with branding")
    }

    fn to_i32(v: u32) -> i32 {
        i32::try_from(v).expect("BUG: asset and panel sizes fit i32")
    }

    fn png_size(png: &[u8]) -> (i32, i32) {
        let image = image::load_from_memory(png).expect("BUG: embedded assets are valid PNGs");
        (to_i32(image.width()), to_i32(image.height()))
    }

    fn panel_size(product: Product) -> (i32, i32) {
        let display = HardwareProfile::for_product(product).display;
        (
            to_i32(display.logical_width),
            to_i32(display.logical_height),
        )
    }

    fn logo_scenes(captions: &[LogoCaption]) -> Vec<LogoScene> {
        captions
            .iter()
            .map(|&caption| LogoScene {
                x_offset: 0,
                alpha: 1.0,
                caption,
            })
            .collect()
    }

    /// Sizes of the logo, empty-scene caption and loading caption textures `overlay` holds,
    /// read through the accessors `draw` uses; `None` where a texture is not loaded.
    fn held_texture_sizes(overlay: &BrandingOverlay) -> [Option<(i32, i32)>; 3] {
        let size = |texture: Option<&GlesTexture>| texture.map(|t| (t.size().w, t.size().h));
        [
            size(overlay.logo.texture()),
            size(overlay.caption_texture(LogoCaption::Empty)),
            size(overlay.caption_texture(LogoCaption::Loading)),
        ]
    }

    #[test]
    fn bmm101_empty_scene_draws_the_product_lines() {
        assert!(
            branding(Product::Bmm101).caption_png(LogoCaption::Empty)
                == Some(BMM101_EMPTY_SCENE_TEXT_PNG),
            "the empty-scene caption is the BMM101 product lines",
        );
    }

    #[test]
    fn deck_has_no_empty_scene_caption() {
        assert!(
            branding(Product::Bmc100)
                .caption_png(LogoCaption::Empty)
                .is_none()
        );
    }

    #[test]
    fn loading_scene_draws_the_loading_caption() {
        for product in all_products() {
            let Some(branding) = Branding::for_product(product) else {
                continue;
            };
            assert!(
                branding.caption_png(LogoCaption::Loading) == Some(LOADING_SCENE_TEXT_PNG),
                "{product:?} draws the \"Loading scene…\" caption",
            );
        }
    }

    #[test]
    fn unbranded_products_keep_the_plain_background() {
        for product in [Product::Bmm100, Product::Bfm100] {
            assert!(Branding::for_product(product).is_none(), "{product:?}");
        }
    }

    /// The origins are where the ring and the text lines sit in the BMM101 kernel boot logo,
    /// a 480x306 image drawn unrotated and unscaled at the panel's top-left corner.
    #[test]
    fn bmm101_branding_matches_the_kernel_boot_logo() {
        let display = HardwareProfile::for_product(Product::Bmm101).display;
        assert_eq!(
            display.scanout_transform,
            DisplayTransform::Deg0,
            "the kernel draws its logo unrotated",
        );
        assert_eq!(
            (display.logical_width, display.logical_height),
            (display.advertised_width, display.advertised_height),
            "the kernel draws its logo unscaled",
        );

        let branding = branding(Product::Bmm101);
        let panel = panel_size(Product::Bmm101);
        let logo = png_size(branding.logo_png);
        assert_eq!(branding.logo_origin(panel, logo), (190, 80));
        let caption = png_size(BMM101_EMPTY_SCENE_TEXT_PNG);
        assert_eq!(branding.caption_origin(panel, logo, caption), (141, 204));
    }

    #[test]
    fn branding_fits_on_the_panel() {
        for product in all_products() {
            let Some(branding) = Branding::for_product(product) else {
                continue;
            };
            let panel = panel_size(product);
            let assert_fits = |what: &str, (x, y): (i32, i32), (w, h): (i32, i32)| {
                assert!(
                    x >= 0 && y >= 0 && x + w <= panel.0 && y + h <= panel.1,
                    "{product:?} {what} {w}x{h} at ({x}, {y}) must fit the {panel:?} panel",
                );
            };
            let logo = png_size(branding.logo_png);
            assert_fits("logo", branding.logo_origin(panel, logo), logo);
            for caption in [LogoCaption::Empty, LogoCaption::Loading] {
                if let Some(png) = branding.caption_png(caption) {
                    let size = png_size(png);
                    let origin = branding.caption_origin(panel, logo, size);
                    assert_fits(caption.label(), origin, size);
                }
            }
        }
    }

    #[test]
    fn bmm101_holds_only_the_textures_its_scenes_draw() {
        let Some(mut renderer) = headless_renderer() else {
            return;
        };
        let logo = png_size(BMM_LOGO_PNG);
        let empty = png_size(BMM101_EMPTY_SCENE_TEXT_PNG);
        let loading = png_size(LOADING_SCENE_TEXT_PNG);
        assert!(
            logo != empty && logo != loading && empty != loading,
            "BUG: the textures are told apart by size, so the assets must differ in size",
        );
        let mut overlay = BrandingOverlay::new(branding(Product::Bmm101));
        for (shown, held) in [
            (vec![LogoCaption::Empty], [Some(logo), Some(empty), None]),
            (
                vec![LogoCaption::Loading],
                [Some(logo), None, Some(loading)],
            ),
            (
                vec![LogoCaption::Empty, LogoCaption::Loading],
                [Some(logo), Some(empty), Some(loading)],
            ),
            (vec![], [None, None, None]),
        ] {
            overlay.sync(&mut renderer, &logo_scenes(&shown));
            assert_eq!(
                held_texture_sizes(&overlay),
                held,
                "BMM101 showing {shown:?}"
            );
        }
    }

    #[test]
    fn deck_empty_scene_holds_the_logo_texture_alone() {
        let Some(mut renderer) = headless_renderer() else {
            return;
        };
        let mut overlay = BrandingOverlay::new(branding(Product::Bmc100));
        overlay.sync(&mut renderer, &logo_scenes(&[LogoCaption::Empty]));
        assert_eq!(
            held_texture_sizes(&overlay),
            [Some(png_size(DECK_LOGO_PNG)), None, None]
        );
    }
}
