// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared mining overlays: the auth-error banner and the stale-data pill,
//! floated over a view's root via the SDK status overlay.

use bmc_wasm_sdk::{Node, SystemTime, ViewportShape, with_error_overlay, with_stale_overlay};

pub const AUTH_ERROR_TEXT: &str = "Cannot authenticate";

/// Which mining overlay to show: auth-error banner, stale pill, or a
/// "failed to load" banner for a source that never loaded and is now failing.
#[derive(Clone, Copy, Debug)]
pub enum OverlayKind {
    Auth,
    Stale(SystemTime),
    Failed(&'static str),
}

/// Float the chosen overlay over a view's root, placed per viewport shape.
#[must_use]
pub fn apply_overlay(root: Node, kind: Option<OverlayKind>, shape: ViewportShape) -> Node {
    match kind {
        Some(OverlayKind::Auth) => with_error_overlay(root, AUTH_ERROR_TEXT, shape),
        Some(OverlayKind::Stale(anchor)) => with_stale_overlay(root, anchor, shape),
        Some(OverlayKind::Failed(reason)) => with_error_overlay(root, reason, shape),
        None => root,
    }
}
