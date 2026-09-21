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

use crate::bos::AuthMode;

pub const AUTH_ERROR_TEXT: &str = "Cannot authenticate";
pub const UNBOUND_TEXT: &str = "Bind a BOS account";
pub const AMBIGUOUS_TEXT: &str = "Bind one BOS account, not both";

/// Which mining overlay to show: auth-error banner, stale pill,
/// a "failed to load" banner for a source that never loaded and is now failing,
/// or a binding prompt when the account slots are not usable.
#[derive(Clone, Copy, Debug)]
pub enum OverlayKind {
    Auth,
    Stale(SystemTime),
    Failed(&'static str),
    Unbound,
    Ambiguous,
}

/// The overlay a binding state earns before any data is consulted.
#[must_use]
pub fn binding_overlay(mode: &AuthMode) -> Option<OverlayKind> {
    match mode {
        AuthMode::Unbound => Some(OverlayKind::Unbound),
        AuthMode::Ambiguous => Some(OverlayKind::Ambiguous),
        AuthMode::Local { .. } | AuthMode::Remote { .. } => None,
    }
}

/// Float the chosen overlay over a view's root, placed per viewport shape.
#[must_use]
pub fn apply_overlay(root: Node, kind: Option<OverlayKind>, shape: ViewportShape) -> Node {
    match kind {
        Some(OverlayKind::Auth) => with_error_overlay(root, AUTH_ERROR_TEXT, shape),
        Some(OverlayKind::Stale(anchor)) => with_stale_overlay(root, anchor, shape),
        Some(OverlayKind::Failed(reason)) => with_error_overlay(root, reason, shape),
        Some(OverlayKind::Unbound) => with_error_overlay(root, UNBOUND_TEXT, shape),
        Some(OverlayKind::Ambiguous) => with_error_overlay(root, AMBIGUOUS_TEXT, shape),
        None => root,
    }
}

#[cfg(test)]
mod tests {
    use super::{AMBIGUOUS_TEXT, OverlayKind, UNBOUND_TEXT, binding_overlay};
    use crate::bos::{AuthMode, Placeholders};

    const PLACEHOLDERS: Placeholders = Placeholders {
        token: "t",
        username: "u",
        password: "p",
    };

    #[test]
    fn only_a_missing_or_double_binding_yields_an_overlay() {
        let derive =
            |local, remote| AuthMode::derive(local, remote, "http://m/api/v1", PLACEHOLDERS);
        assert!(matches!(
            binding_overlay(&derive(false, false)),
            Some(OverlayKind::Unbound)
        ));
        assert!(matches!(
            binding_overlay(&derive(true, true)),
            Some(OverlayKind::Ambiguous)
        ));
        assert!(binding_overlay(&derive(true, false)).is_none());
        assert!(binding_overlay(&derive(false, true)).is_none());
        assert_ne!(UNBOUND_TEXT, AMBIGUOUS_TEXT);
    }
}
