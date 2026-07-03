// Copyright (C) 2026  Braiins Systems s.r.o.

//! Tag pill component — Carbon status label rendered as a rounded, themed
//! chrome (background + leading icon) around an embedder-composed content child.

use bmc_wasm_protocol::{
    BLUE_50, Color, GRAY_100, ICON_ERROR, ICON_INFO, ICON_WARNING, ORANGE_40, RED_50, SvgId,
    TagKind,
};

use crate::renderer::Renderer;

// ── Geometry ───────────────────────────────────────────────────────────
const TAG_RADIUS: f32 = 16.0; // pill `border-radius`, clamped to a stadium (h/2)
const TAG_ICON_SIZE: f32 = 16.0;
const TAG_ICON_GAP: f32 = 8.0; // icon → text
/// Leading pad sets the icon in past the stadium cap, else the curve wraps it.
const TAG_PAD_LEAD: f32 = 12.0;
const TAG_PAD_TRAIL: f32 = 16.0; // text-side, and both sides when icon-less
/// Top/bottom breathing room; the pill height is `content + 2×this`.
pub(crate) const TAG_PAD_VERT: f32 = 6.0;

// ── Data ───────────────────────────────────────────────────────────────
/// Per-node tag chrome; `icon` is already resolved (`None` = no icon).
#[derive(Clone, Debug)]
pub(crate) struct TagData {
    pub kind: TagKind,
    pub icon: Option<SvgId>,
}

/// Resolved per-kind Carbon theme: solid dark pill, variant-colored icon/content.
#[derive(Clone, Copy, Debug)]
pub struct TagTheme {
    pub background: Color,
    pub content: Color,
    pub icon: SvgId,
}

#[must_use]
pub fn tag_theme(kind: TagKind) -> TagTheme {
    let (content, icon) = match kind {
        TagKind::Info => (BLUE_50, ICON_INFO),
        TagKind::Warning => (ORANGE_40, ICON_WARNING),
        TagKind::Error => (RED_50, ICON_ERROR),
    };
    TagTheme {
        background: GRAY_100,
        content,
        icon,
    }
}

/// Taffy `(left, right)` content padding, reserving the leading icon lane.
pub(crate) fn tag_content_padding(has_icon: bool) -> (f32, f32) {
    let left = if has_icon {
        TAG_PAD_LEAD + TAG_ICON_SIZE + TAG_ICON_GAP
    } else {
        TAG_PAD_TRAIL
    };
    (left, TAG_PAD_TRAIL)
}

// ── Rendering ──────────────────────────────────────────────────────────
/// Paint the pill background + leading icon behind the content child.
pub(crate) fn render_tag(
    tag: &TagData,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    renderer: &mut dyn Renderer,
) {
    let theme = tag_theme(tag.kind);
    // Clamp to a stadium: a radius above h/2 draws broken (non-half-circle) ends.
    let radius = TAG_RADIUS.min(h / 2.0);
    renderer.fill_rounded_rect(x, y, w, h, radius, theme.background);
    if let Some(icon) = tag.icon {
        let icon_y = y + (h - TAG_ICON_SIZE) / 2.0;
        renderer.draw_svg(
            x + TAG_PAD_LEAD,
            icon_y,
            TAG_ICON_SIZE,
            TAG_ICON_SIZE,
            theme.content,
            icon,
            false,
            &[],
        );
    }
}

#[cfg(test)]
mod tests {
    use bmc_wasm_protocol::{
        ICON_WARNING, NODE_SPACER, NODE_TAG, ORANGE_40, TAG_ICON_DEFAULT, TAG_ICON_HIDDEN, TagKind,
    };

    use super::tag_theme;
    use crate::tree::{TreeNode, deserialize_tree};

    /// Wire for a tag wrapping a spacer content child.
    fn tag_wire(kind: TagKind, icon_mode: u8, icon: u16) -> Vec<u8> {
        let mut bytes = vec![NODE_TAG, kind as u8, icon_mode];
        bytes.extend_from_slice(&icon.to_le_bytes());
        bytes.push(NODE_SPACER);
        bytes.extend_from_slice(&1.0_f32.to_le_bytes());
        bytes
    }

    #[test]
    fn default_icon_mode_resolves_to_theme() {
        let node = deserialize_tree(&tag_wire(TagKind::Warning, TAG_ICON_DEFAULT, 0))
            .expect("BUG: Tag should deserialize");
        assert!(matches!(
            node,
            TreeNode::Tag {
                kind: TagKind::Warning,
                icon: Some(id),
                ..
            } if id == ICON_WARNING
        ));
    }

    #[test]
    fn hidden_icon_mode_resolves_to_none() {
        let node = deserialize_tree(&tag_wire(TagKind::Error, TAG_ICON_HIDDEN, 0))
            .expect("BUG: Tag should deserialize");
        assert!(matches!(node, TreeNode::Tag { icon: None, .. }));
    }

    #[test]
    fn theme_maps_kind_to_carbon_tokens() {
        let warn = tag_theme(TagKind::Warning);
        assert_eq!(warn.icon, ICON_WARNING);
        assert_eq!(warn.content, ORANGE_40);
    }
}
