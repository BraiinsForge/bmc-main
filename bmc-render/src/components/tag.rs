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

//! Tag pill component — Carbon status label rendered as a rounded, themed
//! chrome (background + leading icon) around an embedder-composed content child.

use bmc_ui::{
    BLUE_50, Color, GRAY_100, ICON_ERROR, ICON_INFO, ICON_WARNING, ORANGE_40, RED_50, SvgId,
    TagIcon, TagKind,
};

use crate::renderer::RenderTarget;
use crate::renderer::prelude::*;

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

/// The glyph a tag of `kind` leads with: the theme's for `Default`, none for `Hidden`.
#[must_use]
pub fn resolve_icon(kind: TagKind, icon: TagIcon) -> Option<SvgId> {
    match icon {
        TagIcon::Default => Some(tag_theme(kind).icon),
        TagIcon::Hidden => None,
        TagIcon::Custom(id) => Some(id),
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
    renderer: &mut RenderTarget<'_, '_, '_>,
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
    use bmc_ui::TagIcon;
    use bmc_ui::{ICON_CLOSE, ICON_WARNING, ORANGE_40, TagKind};

    use super::{resolve_icon, tag_theme};

    #[test]
    fn each_icon_mode_resolves_against_the_theme() {
        assert_eq!(
            resolve_icon(TagKind::Warning, TagIcon::Default),
            Some(ICON_WARNING)
        );
        assert_eq!(resolve_icon(TagKind::Error, TagIcon::Hidden), None);
        assert_eq!(
            resolve_icon(TagKind::Info, TagIcon::Custom(ICON_CLOSE)),
            Some(ICON_CLOSE)
        );
    }

    #[test]
    fn theme_maps_kind_to_carbon_tokens() {
        let warn = tag_theme(TagKind::Warning);
        assert_eq!(warn.icon, ICON_WARNING);
        assert_eq!(warn.content, ORANGE_40);
    }
}
