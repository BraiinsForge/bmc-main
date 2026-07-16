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
