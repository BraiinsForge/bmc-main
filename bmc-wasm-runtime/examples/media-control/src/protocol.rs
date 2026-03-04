// Copyright (C) 2026  Braiins Systems s.r.o.

//! Unified media controller protocol abstraction.
//!
//! Every protocol backend (UPnP, Cast, Kodi, …) implements [`MediaController`].
//! The widget dispatches commands through it without knowing the underlying
//! transport. Protocol-specific status callbacks remain separate — each
//! protocol pushes updates through its own callback registered at connect time.

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
