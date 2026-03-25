// Copyright (C) 2026  Braiins Systems s.r.o.

//! SVG icon loading — rasterizes embedded SVGs into egui textures at startup.
//!
//! Icons are monochrome (white on transparent) and tinted at render time via
//! the egui `Image::tint()` API.

use egui::{ColorImage, TextureHandle, TextureOptions};

/// Icon size in logical pixels.
const ICON_SIZE: u32 = 16;

/// All loaded icon textures.
pub struct Icons {
    pub app: TextureHandle,
    pub close: TextureHandle,
    pub code: TextureHandle,
    pub color_palette: TextureHandle,
    pub folder: TextureHandle,
    pub pause: TextureHandle,
    pub play: TextureHandle,
    pub renew: TextureHandle,
    pub search: TextureHandle,
    pub touch: TextureHandle,
}

impl Icons {
    /// Rasterize all embedded SVGs and register as egui textures.
    pub fn load(ctx: &egui::Context) -> Self {
        Self {
            app: load_svg(ctx, "app", include_bytes!("../assets/icons/app.svg")),
            close: load_svg(ctx, "close", include_bytes!("../assets/icons/close.svg")),
            code: load_svg(ctx, "code", include_bytes!("../assets/icons/code.svg")),
            color_palette: load_svg(
                ctx,
                "color_palette",
                include_bytes!("../assets/icons/color-palette.svg"),
            ),
            folder: load_svg(ctx, "folder", include_bytes!("../assets/icons/folder.svg")),
            pause: load_svg(ctx, "pause", include_bytes!("../assets/icons/pause.svg")),
            play: load_svg(ctx, "play", include_bytes!("../assets/icons/play.svg")),
            renew: load_svg(ctx, "renew", include_bytes!("../assets/icons/renew.svg")),
            search: load_svg(ctx, "search", include_bytes!("../assets/icons/search.svg")),
            touch: load_svg(ctx, "touch", include_bytes!("../assets/icons/touch.svg")),
        }
    }
}

/// Rasterize an SVG to a white-on-transparent egui texture at `ICON_SIZE`.
fn load_svg(ctx: &egui::Context, name: &str, svg_bytes: &[u8]) -> TextureHandle {
    let tree = resvg::usvg::Tree::from_data(svg_bytes, &resvg::usvg::Options::default())
        .unwrap_or_else(|e| panic!("BUG: failed to parse {name}.svg: {e}"));

    let svg_size = tree.size();
    #[expect(clippy::cast_precision_loss)]
    let scale = (ICON_SIZE as f32) / svg_size.width().max(svg_size.height());

    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE).expect("BUG: failed to create pixmap");

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Normalize to white-on-transparent: any pixel with alpha > 0 becomes
    // white at that alpha. This makes the source SVG fill color irrelevant —
    // the actual display color is applied via egui's Image::tint() at render time.
    let pixels: Vec<egui::Color32> = pixmap
        .pixels()
        .iter()
        .map(|p| {
            let a = p.alpha();
            egui::Color32::from_rgba_premultiplied(a, a, a, a)
        })
        .collect();

    let size = [ICON_SIZE as usize, ICON_SIZE as usize];
    let image = ColorImage::new(size, pixels);

    ctx.load_texture(
        format!("icon_{name}"),
        image,
        TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            ..Default::default()
        },
    )
}

/// Paint a tinted icon at a given size. Returns the response rect.
pub fn icon_image(texture: &TextureHandle, size: f32, tint: egui::Color32) -> egui::Image<'_> {
    egui::Image::new(texture)
        .fit_to_exact_size(egui::vec2(size, size))
        .tint(tint)
}
