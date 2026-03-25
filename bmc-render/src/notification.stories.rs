// Copyright (C) 2026  Braiins Systems s.r.o.

use crate::prelude::*;

story_meta! { title: "Notification" }

#[story(default)]
fn notification_kinds(ctx: &mut StoryCtx) -> Node {
    let title = ctx.text("Title", "Something happened");
    let subtitle = ctx.text("Subtitle", "Additional details about this event");

    col(
        props!(gap: 8, padding: 16, width: 320),
        [
            notification(NotificationKind::Info, &title, &subtitle),
            notification(NotificationKind::Success, &title, &subtitle),
            notification(NotificationKind::Warning, &title, &subtitle),
            notification(NotificationKind::Error, &title, &subtitle),
        ],
    )
}
