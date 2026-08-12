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

//! What the UI asks a device view to do.
//!
//! Every operator-driven mutation is a value rather than a method call.
//! The UI never reaches into a runtime, and the same request can later cross
//! a channel to a worker thread: these payloads are all `Send` already, while
//! what is not — the runtime, the renderer — stays on the side that owns it.

use bmc_render::interaction::TouchEvent;

/// A state update pushed into a widget.
///
/// Each variant carries the whole new snapshot rather than a diff: the runtime
/// compares against what it holds and ignores a delivery that changes nothing.
#[derive(Debug)]
pub(crate) enum Delivery {
    Params(
        std::collections::BTreeMap<bmc_widget_manifest::ParamKey, bmc_widget_manifest::ParamValue>,
    ),
    System(Box<bmc_wasm_runtime::SystemSnapshot>),
    Credentials {
        view: Box<bmc_wasm_runtime::CredentialView>,
        secrets: Box<bmc_widget_protocol::CredentialSecrets>,
    },
}

/// One request from the UI to a view, applied at the start of its next tick.
#[derive(Debug)]
pub(crate) enum ViewCommand {
    Deliver(Delivery),
    /// A pointer event in widget-local coordinates.
    Touch(TouchEvent),
    /// End of a gesture: fire `on_touch` once for everything pushed
    /// since the last one, mirroring the device host's per-drain delivery.
    DeliverTouch,
}
