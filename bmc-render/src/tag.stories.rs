// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Tag" }

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

#[story(default)]
fn tag_pill(ctx: &mut StoryCtx) {
    let kind = tag_kind(ctx.select("Kind", &["Info", "Warning", "Error"], 1).get());
    let icon = match ctx
        .select("Icon", &["Default", "Hidden", "Custom"], 0)
        .get()
    {
        1 => TagIcon::Hidden,
        2 => TagIcon::Custom(ICON_WARN_FILLED),
        _ => TagIcon::Default,
    };
    let label = ctx.text("Label", "Last refresh 2m ago");

    let content = col(
        props!(gap: 12, padding: 24),
        [
            pill(kind, icon, label.get()),
            // Static showcase — every kind (default icon), plus an explicit override.
            pill(TagKind::Info, TagIcon::Default, "Info"),
            pill(TagKind::Warning, TagIcon::Default, "Warning"),
            pill(TagKind::Error, TagIcon::Default, "Error"),
            pill(TagKind::Info, TagIcon::Custom(ICON_CLOSE), "Custom icon"),
        ],
    );
    ctx.ui.div(FrameSize::Custom(320, AutoH), content);
}
