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

//! Framework for privileged system overlays rendered as wlr-layer-shell clients.

mod connectivity;
mod gpu;
mod hosted;
mod icon;
mod overlay;
mod standalone;
mod surface;
#[cfg(test)]
pub(crate) mod test_support;
mod tree;

pub use connectivity::{Snapshot, SnapshotVersion, VersionedSnapshot, snapshot_if_changed};
pub use gpu::{OverlayRenderTarget, wait_for_gpu};
pub use hosted::HostedOverlay;
pub use icon::register_icon;
pub use overlay::{
    AlarmEvent, AlarmRequest, DownloadProgress, InputRegion, LayerConfig, ScreenEdge, SettingsCaps,
    SettingsRequest, SystemOverlay, TickOutcome, TouchEvent, UpgradeKind, UpgradePhase,
    UpgradeSnapshot, UpgradeState,
};
pub use standalone::run_standalone;
pub use surface::LayerSurfaceClient;
pub use tree::TreeUi;

// Re-export the layer-shell client enums so overlays can build a LayerConfig
// without importing the protocol crate directly.
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
