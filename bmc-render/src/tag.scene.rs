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

use bmc_gallery::prelude::*;

scene_meta! { title: "Components / Feedback / Tag" }

fn tag_kind(index: usize) -> TagKind {
    match index {
        0 => TagKind::Info,
        2 => TagKind::Error,
        _ => TagKind::Warning,
    }
}

/// The embedder styles its content to match the variant (icon + text share it).
fn kind_color(kind: TagKind) -> Color {
    match kind {
        TagKind::Info => BLUE_50,
        TagKind::Warning => ORANGE_40,
        TagKind::Error => RED_50,
    }
}

fn pill(kind: TagKind, icon: TagIcon, label: &str) -> Node {
    tag(
        kind,
        icon,
        text(label, style!(size: 14, color: kind_color(kind))),
    )
}

#[scene(default)]
fn tag_pill(ctx: &mut SceneCtx, ui: &mut Ui) {
    let kind = tag_kind(ctx.select("Kind", &["Info", "Warning", "Error"], 1));
    let icon = match ctx.select("Icon", &["Default", "Hidden", "Custom"], 0) {
        1 => TagIcon::Hidden,
        2 => TagIcon::Custom(ICON_WARN_FILLED),
        _ => TagIcon::Default,
    };
    let label = ctx.text("Label", "Last refresh 2m ago");

    ctx.node_stage(ui, DeckSize::Custom(320, AutoH), || {
        col(
            props!(gap: 12, padding: 24),
            [
                pill(kind, icon, &label),
                // Static showcase — every kind (default icon), plus an explicit override.
                pill(TagKind::Info, TagIcon::Default, "Info"),
                pill(TagKind::Warning, TagIcon::Default, "Warning"),
                pill(TagKind::Error, TagIcon::Default, "Error"),
                pill(TagKind::Info, TagIcon::Custom(ICON_CLOSE), "Custom icon"),
            ],
        )
    });
}
