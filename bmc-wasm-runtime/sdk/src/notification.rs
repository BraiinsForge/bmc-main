// Copyright (C) 2026  Braiins Systems s.r.o.

//! Notification banner builder.

use crate::tree::Node;

/// Inline notification severity kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NotificationKind {
    Error = 0,
    Warning = 1,
    Success = 2,
    Info = 3,
}

/// Inline notification banner
pub fn notification(
    kind: NotificationKind,
    title: impl Into<String>,
    subtitle: impl Into<String>,
) -> Node {
    Node::Notification {
        kind,
        title: title.into(),
        subtitle: subtitle.into(),
    }
}
