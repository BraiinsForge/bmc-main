// Copyright (C) 2026  Braiins Systems s.r.o.

//! Shared mining overlays: the auth-error banner and the stale-data pill,
//! floated over a view's root via the SDK status overlay.

use bmc_wasm_sdk::{Node, SystemTime, ViewportShape, with_error_overlay, with_stale_overlay};

pub const AUTH_ERROR_TEXT: &str = "Cannot authenticate";

/// Which mining overlay to show: the auth-error banner or the stale pill.
#[derive(Clone, Copy, Debug)]
pub enum OverlayKind {
    Auth,
    Stale(SystemTime),
}

/// Float the chosen overlay over a view's root, placed per viewport shape.
#[must_use]
pub fn apply_overlay(root: Node, kind: Option<OverlayKind>, shape: ViewportShape) -> Node {
    match kind {
        Some(OverlayKind::Auth) => with_error_overlay(root, AUTH_ERROR_TEXT, shape),
        Some(OverlayKind::Stale(anchor)) => with_stale_overlay(root, anchor, shape),
        None => root,
    }
}
