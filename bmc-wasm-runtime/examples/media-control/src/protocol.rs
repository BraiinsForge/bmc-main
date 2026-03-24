// Copyright (C) 2026  Braiins Systems s.r.o.
#![allow(dead_code)]

//! Unified media controller protocol abstraction.
//!
//! Every protocol backend (UPnP, Cast, future) populates a shared
//! [`MediaState`] and accepts commands through a common [`MediaController`]
//! trait. The UI works exclusively against this abstraction.

use bmc_wasm_sdk::FetchResponse;

// ── Unified state ────────────────────────────────────────────────

/// Transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    Stopped,
    Transitioning,
    NoMedia,
}

/// Which protocol is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Upnp,
    Cast,
}

/// Connected device identity.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub address: String,
    pub protocol: Protocol,
}

/// Track metadata.
#[derive(Debug, Clone, Default)]
pub struct TrackInfo {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_art_uri: Option<String>,
}

/// Album art + extracted palette.
#[derive(Debug, Clone, Default)]
pub struct ArtState {
    /// Host-side bitmap ID (0 = none).
    pub bitmap_id: u16,
    /// Dominant colors extracted from album art (RGB, up to 3).
    pub palette: Vec<u32>,
}

/// Volume state.
#[derive(Debug, Clone, Copy, Default)]
pub struct VolumeState {
    /// 0.0–1.0 normalized volume.
    pub level: f32,
    pub muted: bool,
}

/// Unified media state that the UI renders from.
///
/// Both UPnP and Cast controllers update this shared structure.
#[derive(Debug, Clone)]
pub struct MediaState {
    pub device: DeviceInfo,
    pub track: TrackInfo,
    pub art: ArtState,
    pub playback: PlaybackState,
    /// Current playback position in seconds.
    pub position_secs: u32,
    /// Track duration in seconds.
    pub duration_secs: u32,
    pub volume: VolumeState,
}

// ── Controller trait ─────────────────────────────────────────────

/// Common callback type for async protocol operations.
pub type ResponseCallback = fn(&FetchResponse);

/// Protocol-agnostic media controller interface.
///
/// Each protocol backend implements this trait. The widget dispatches
/// commands through it without knowing the underlying transport.
pub trait MediaController {
    /// Start or resume playback.
    fn play(&self, cb: ResponseCallback);
    /// Pause playback.
    fn pause(&self, cb: ResponseCallback);
    /// Stop playback.
    fn stop(&self, cb: ResponseCallback);
    /// Skip to next track.
    fn next(&self, cb: ResponseCallback);
    /// Skip to previous track.
    fn previous(&self, cb: ResponseCallback);
    /// Seek to a position in seconds.
    fn seek(&self, position_secs: u32, cb: ResponseCallback);

    /// Set volume (0.0–1.0).
    fn set_volume(&self, level: f32, cb: ResponseCallback);
    /// Set mute state.
    fn set_mute(&self, muted: bool, cb: ResponseCallback);

    /// Request current position info (triggers async callback with state update).
    fn poll_position(&self, cb: ResponseCallback);
    /// Request current position info after a delay (for periodic polling).
    fn poll_position_after(&self, delay_ms: u32, cb: ResponseCallback);
    /// Request current transport state.
    fn poll_transport(&self, cb: ResponseCallback);
    /// Request current volume level.
    fn poll_volume(&self, cb: ResponseCallback);
    /// Request current mute state.
    fn poll_mute(&self, cb: ResponseCallback);

    /// Device info for this controller.
    fn device(&self) -> DeviceInfo;
    /// Protocol in use.
    fn protocol(&self) -> Protocol;
}
