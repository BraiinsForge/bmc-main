// Copyright (C) 2026  Braiins Systems s.r.o.

//! Unified media controller protocol abstraction.
//!
//! Every protocol backend (UPnP, Cast, Kodi, …) implements [`MediaController`].
//! The widget dispatches commands through it without knowing the underlying
//! transport. Protocol-specific status callbacks remain separate — each
//! protocol pushes updates through its own callback registered at connect time.

// ── Shared types ────────────────────────────────────────────

/// Track metadata pushed by protocols to the view.
#[derive(Debug, Clone, Default)]
pub struct TrackMeta {
    /// Primary display name (rendered big).
    pub title: Option<String>,
    /// Secondary metadata lines — protocol decides labels and order.
    /// e.g. `[("Artist", "Springsteen"), ("Album", "Born to Run")]`
    /// or `[("Series", "QI"), ("Season", "Season 21")]`
    pub fields: Vec<(String, String)>,
    pub album_art_uri: Option<String>,
}

/// A selectable sub-target within a connected device (e.g. a client session).
#[derive(Debug, Clone)]
pub struct SubTarget {
    /// Protocol-specific identifier (e.g. session ID).
    pub id: String,
    /// Primary display name (e.g. "Living Room TV").
    pub name: String,
    /// Secondary metadata — same pattern as `TrackMeta::fields`.
    pub fields: Vec<(String, String)>,
    /// Whether this is the currently controlled sub-target.
    pub active: bool,
}

/// Sub-target selection info returned by protocols that support it.
///
/// Used by server-based protocols (Emby, Jellyfin, future Plex) where one
/// server can have multiple controllable client sessions.
pub struct SubTargets {
    /// What to call these in the UI (e.g. "Session", "Player").
    pub term: &'static str,
    /// Available choices. Empty = still loading / none available.
    pub items: Vec<SubTarget>,
}

// ── Controller trait ─────────────────────────────────────────────

/// Protocol-agnostic media controller interface.
///
/// Each protocol backend implements this trait. The widget dispatches
/// commands through it without knowing the underlying transport.
///
/// **`&self` not `&mut self`** — mutations happen inside protocol modules
/// through their own thread-locals (Cast, Kodi) or are stateless HTTP (UPnP).
pub trait MediaController {
    // ── Lifecycle ────────────────────────────────────────────────

    /// Tear down connection and release resources.
    fn disconnect(&self);

    /// Whether the remote device is still reachable.
    fn is_alive(&self) -> bool;

    /// Drive internal timers (heartbeat, poll counters). Called every render.
    fn tick(&self, delta_ms: u32);

    // ── Commands (fire-and-forget) ──────────────────────────────

    /// Start or resume playback.
    fn play(&self);

    /// Pause playback.
    fn pause(&self);

    /// Skip to next track.
    fn next(&self);

    /// Skip to previous track.
    fn previous(&self);

    /// Seek to absolute position. `duration_secs` provided because some
    /// protocols (Kodi) need it to compute a percentage.
    fn seek(&self, position_secs: u32, duration_secs: u32);

    /// Set volume level (0.0–1.0 normalized).
    fn set_volume(&self, level: f32);

    /// Set mute state.
    fn set_mute(&self, muted: bool);

    // ── Target selection ──────────────────────────────────────

    /// Available sub-targets within this device. `None` means the protocol
    /// has no sub-target concept (device = target). `Some` with empty items
    /// means still loading or no targets available yet.
    fn sub_targets(&self) -> Option<SubTargets> {
        None
    }

    /// Select a specific sub-target by ID (from `SubTarget::id`).
    fn select_sub_target(&self, _id: &str) {}

    // ── Protocol metadata ───────────────────────────────────────

    /// How often to request a render frame while playing (ms).
    fn poll_interval_playing(&self) -> u32;

    /// How often to request a render frame while idle (ms).
    fn poll_interval_idle(&self) -> u32;

    /// Human-readable protocol name (e.g. "Cast", "UPnP", "Kodi").
    fn protocol_name(&self) -> &'static str;

    /// Fetch album art, handling protocol-specific auth.
    fn fetch_art(&self, url: &str, callback: fn(&bmc_wasm_sdk::FetchResponse)) {
        bmc_wasm_sdk::fetch(url, None, callback);
    }
}
