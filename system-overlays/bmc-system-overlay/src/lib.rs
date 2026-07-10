// Copyright (C) 2026  Braiins Systems s.r.o.

//! Framework for privileged system overlays rendered as wlr-layer-shell clients.

mod connectivity;
mod gpu;
mod hosted;
mod overlay;
mod standalone;
mod surface;
#[cfg(test)]
pub(crate) mod test_support;
mod tree;

pub use connectivity::{Snapshot, SnapshotVersion, VersionedSnapshot, snapshot_if_changed};
pub use gpu::{OverlayRenderTarget, wait_for_gpu};
pub use hosted::HostedOverlay;
pub use overlay::{
    AlarmEvent, AlarmRequest, InputRegion, LayerConfig, ScreenEdge, SettingsCaps, SettingsRequest,
    SystemOverlay, TickOutcome, TouchEvent,
};
pub use standalone::run_standalone;
pub use surface::LayerSurfaceClient;
pub use tree::TreeUi;

// Re-export the layer-shell client enums so overlays can build a LayerConfig
// without importing the protocol crate directly.
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
