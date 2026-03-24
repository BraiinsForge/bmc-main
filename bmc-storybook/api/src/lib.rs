// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared API types for the storybook framework.
//!
//! This crate is the boundary contract between the shell binary (`bmc-storybook`)
//! and the hot-swapped stories cdylib (`bmc-storybook-stories`). It intentionally
//! has no egui dependency — egui types must never cross the dlopen boundary.

pub mod audio;
pub mod knobs;
pub mod prelude;

use bmc_render::interaction::InteractionState;
use bmc_render::renderer::Renderer;
use bmc_wasm_sdk::tree::Node;
use knobs::StoryCtx;

/// Callback for custom-rendered frames that bypass the tree pipeline.
///
/// Arguments: `(renderer, interaction, width, height, delta_ms)`.
pub type CustomRenderFn = Box<dyn FnMut(&mut dyn Renderer, &mut InteractionState, f32, f32, u32)>;

// ── Document model types ────────────────────────────────────────────

/// Device display dimensions (Braiins clock: 1280x480).
pub const DEVICE_WIDTH: u32 = 1_280;
pub const DEVICE_HEIGHT: u32 = 480;

/// Maximum FBO height for auto-height frames (content-driven layout).
pub const AUTO_HEIGHT_MAX: u32 = 4_096;

/// Height dimension for a frame — fixed pixels or content-driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivHeight {
    Px(u32),
    /// Content-driven: layout with height=0, clip to actual content size.
    Auto,
}

impl From<u32> for DivHeight {
    fn from(v: u32) -> Self {
        Self::Px(v)
    }
}

/// Preset frame sizes for `ctx.ui.div(...)`.
///
/// All named presets resolve to fixed (width, height) dimensions.
/// `Custom` allows arbitrary sizing including auto-height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSize {
    /// Device display dimensions (1280x480).
    Full,
    /// 480x320.
    Large,
    /// 320x240.
    Medium,
    /// 160x120.
    Small,
    /// Fully content-driven (both width and height from layout).
    Auto,
    /// Arbitrary dimensions.
    Custom(u32, DivHeight),
}

impl FrameSize {
    /// FBO width in pixels (`Auto` → `DEVICE_WIDTH` as upper bound).
    #[must_use]
    pub fn width(self) -> u32 {
        match self {
            Self::Large => 480,
            Self::Medium => 320,
            Self::Small => 160,
            Self::Full | Self::Auto => DEVICE_WIDTH,
            Self::Custom(w, _) => w,
        }
    }

    /// Height specification.
    #[must_use]
    pub fn div_height(self) -> DivHeight {
        match self {
            Self::Full => DivHeight::Px(DEVICE_HEIGHT),
            Self::Large => DivHeight::Px(320),
            Self::Medium => DivHeight::Px(240),
            Self::Small => DivHeight::Px(120),
            Self::Auto => DivHeight::Auto,
            Self::Custom(_, h) => h,
        }
    }

    /// Whether width is content-driven.
    #[must_use]
    pub fn is_auto_width(self) -> bool {
        matches!(self, Self::Auto)
    }

    /// Resolved height in pixels (Auto → `AUTO_HEIGHT_MAX` for FBO allocation).
    #[must_use]
    pub fn fbo_height(self) -> u32 {
        match self.div_height() {
            DivHeight::Px(h) => h,
            DivHeight::Auto => AUTO_HEIGHT_MAX,
        }
    }

    /// Layout width passed to `process_tree` (Auto → 0 for content-driven).
    #[must_use]
    #[expect(clippy::cast_precision_loss)]
    pub fn layout_width(self) -> f32 {
        if self.is_auto_width() {
            0.0
        } else {
            self.width() as f32
        }
    }

    /// Layout height passed to `process_tree` (Auto → 0 for content-driven).
    #[must_use]
    #[expect(clippy::cast_precision_loss)]
    pub fn layout_height(self) -> f32 {
        match self.div_height() {
            DivHeight::Px(h) => h as f32,
            DivHeight::Auto => 0.0,
        }
    }
}

impl From<(u32, u32)> for FrameSize {
    fn from((w, h): (u32, u32)) -> Self {
        Self::Custom(w, DivHeight::Px(h))
    }
}

impl From<(f32, f32)> for FrameSize {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn from((w, h): (f32, f32)) -> Self {
        Self::Custom(w as u32, DivHeight::Px(h as u32))
    }
}

impl From<(u32, DivHeight)> for FrameSize {
    fn from((w, h): (u32, DivHeight)) -> Self {
        Self::Custom(w, h)
    }
}

/// A block in the story document.
pub enum DocBlock {
    /// A rendered component frame.
    Frame { size: FrameSize, node: Node },
    /// A frame rendered by a custom callback (bypasses tree pipeline).
    ///
    /// Use for components that call [`Renderer`] methods directly instead
    /// of building tree nodes.
    CustomRender {
        size: FrameSize,
        render_fn: CustomRenderFn,
    },
    /// Section header with optional subtitle.
    Header {
        title: String,
        subtitle: Option<String>,
    },
    /// Code snippet.
    Code { language: String, source: String },
    /// Prose text (markdown-ish).
    Prose { text: String },
    /// Horizontal divider.
    Divider,
    /// Grid of cells with fixed column count.
    ///
    /// Each cell is a `Vec<DocBlock>` rendered as a vertical group.
    Grid {
        cols: u32,
        gap: f32,
        cells: Vec<Vec<DocBlock>>,
    },
}

impl std::fmt::Debug for DocBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frame { size, .. } => f.debug_struct("Frame").field("size", size).finish(),
            Self::CustomRender { size, .. } => {
                f.debug_struct("CustomRender").field("size", size).finish()
            }
            Self::Header { title, subtitle } => f
                .debug_struct("Header")
                .field("title", title)
                .field("subtitle", subtitle)
                .finish(),
            Self::Code { language, source } => f
                .debug_struct("Code")
                .field("language", language)
                .field("source", source)
                .finish(),
            Self::Prose { text } => f.debug_struct("Prose").field("text", text).finish(),
            Self::Divider => write!(f, "Divider"),
            Self::Grid { cols, cells, .. } => f
                .debug_struct("Grid")
                .field("cols", cols)
                .field("cells", &cells.len())
                .finish(),
        }
    }
}

/// Builder for story document blocks.
///
/// Accessed as `ctx.ui` in story functions. Stories in document mode
/// push blocks via `div()`, `header()`, `code()`, etc.
#[derive(Debug)]
pub struct StoryUi {
    blocks: Vec<DocBlock>,
}

impl StoryUi {
    #[must_use]
    pub fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    /// Add a rendered component frame.
    pub fn div(&mut self, size: impl Into<FrameSize>, node: Node) {
        self.blocks.push(DocBlock::Frame {
            size: size.into(),
            node,
        });
    }

    /// Add a custom-rendered frame that bypasses the tree pipeline.
    ///
    /// The callback receives `(renderer, interaction, width, height, delta_ms)`.
    pub fn div_custom(&mut self, size: impl Into<FrameSize>, render_fn: CustomRenderFn) {
        self.blocks.push(DocBlock::CustomRender {
            size: size.into(),
            render_fn,
        });
    }

    /// Add a grid of cells with fixed column count.
    ///
    /// The closure receives a [`GridBuilder`] — call `cell(|ui| { ... })` on it
    /// to add cells. Each cell is a vertical group of blocks.
    pub fn grid(&mut self, cols: u32, gap: f32, build: impl FnOnce(&mut GridBuilder)) {
        let mut gb = GridBuilder { cells: Vec::new() };
        build(&mut gb);
        self.blocks.push(DocBlock::Grid {
            cols,
            gap,
            cells: gb.cells,
        });
    }

    /// Add a section header with subtitle.
    pub fn header(&mut self, title: &str, subtitle: &str) {
        self.blocks.push(DocBlock::Header {
            title: title.to_owned(),
            subtitle: if subtitle.is_empty() {
                None
            } else {
                Some(subtitle.to_owned())
            },
        });
    }

    /// Add a code snippet.
    pub fn code(&mut self, lang: &str, source: &str) {
        self.blocks.push(DocBlock::Code {
            language: lang.to_owned(),
            source: source.to_owned(),
        });
    }

    /// Add prose text.
    pub fn prose(&mut self, text: &str) {
        self.blocks.push(DocBlock::Prose {
            text: text.to_owned(),
        });
    }

    /// Add a horizontal divider.
    pub fn divider(&mut self) {
        self.blocks.push(DocBlock::Divider);
    }

    /// Drain all accumulated blocks.
    pub fn take_blocks(&mut self) -> Vec<DocBlock> {
        std::mem::take(&mut self.blocks)
    }

    /// Clear blocks for a new frame.
    pub fn clear(&mut self) {
        self.blocks.clear();
    }
}

impl Default for StoryUi {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for grid cells.
///
/// Each `cell()` call adds a vertical group of blocks as one grid cell.
#[derive(Debug)]
pub struct GridBuilder {
    cells: Vec<Vec<DocBlock>>,
}

impl GridBuilder {
    /// Add a grid cell. Blocks pushed inside the closure form a vertical group.
    pub fn cell(&mut self, build: impl FnOnce(&mut StoryUi)) {
        let mut inner = StoryUi::new();
        build(&mut inner);
        self.cells.push(inner.take_blocks());
    }
}

// ── Story entry ─────────────────────────────────────────────────────

/// A single story entry registered via `#[story]`.
#[derive(Debug, Clone, Copy)]
pub struct StoryEntry {
    pub render_fn: fn(&mut StoryCtx),
    pub name: &'static str,
    pub module_path: &'static str,
    /// Original source code of the story function (stringified by the macro).
    pub source: &'static str,
    /// When `true`, the story prefers rendering all size variants side-by-side by default.
    pub grid: bool,
    /// When `true` and the story is the only one in its group, the sidebar
    /// collapses the group to a flat entry and the header shows only the
    /// group title, not "Group / Story Name".
    pub default: bool,
}

inventory::collect!(StoryEntry);

/// Metadata for a story group — one per `.stories.rs` file, registered via `story_meta!{}`.
#[derive(Debug, Clone, Copy)]
pub struct StoryGroupMeta {
    pub module_path: &'static str,
    pub title: &'static str,
    pub grid: bool,
}

inventory::collect!(StoryGroupMeta);

/// Manifest returned by the cdylib's `__story_entries()` export.
///
/// Contains copies of all inventory-collected entries and groups from the .so.
/// The caller must convert `&'static str` fields to owned `String`s before
/// dropping the `Library` (dlclose), since those strings point into the .so's
/// read-only data segment.
#[derive(Debug)]
pub struct StoryManifest {
    pub entries: Vec<StoryEntry>,
    pub groups: Vec<StoryGroupMeta>,
}

/// Declare the group title (and optional grid default) for this story file.
///
/// Must be called exactly once per `.stories.rs` file. A second call in the same
/// module is a compile error ("the name `_StoryGroupDeclaredOnce` is defined multiple times").
///
/// # Examples
///
/// ```ignore
/// story_meta! { title: "Button" }
/// story_meta! { title: "Canvas", grid: true }
/// ```
#[macro_export]
macro_rules! story_meta {
    (title: $title:expr) => {
        // Compile error if story_meta! is used more than once in this module.
        enum _StoryGroupDeclaredOnce {}

        $crate::inventory::submit! {
            $crate::StoryGroupMeta {
                module_path: ::core::module_path!(),
                title: $title,
                grid: false,
            }
        }
    };
    (title: $title:expr, grid: $grid:expr) => {
        enum _StoryGroupDeclaredOnce {}

        $crate::inventory::submit! {
            $crate::StoryGroupMeta {
                module_path: ::core::module_path!(),
                title: $title,
                grid: $grid,
            }
        }
    };
}

// Re-export inventory so $crate::inventory works in story_meta! macro.
#[doc(hidden)]
pub use inventory;
