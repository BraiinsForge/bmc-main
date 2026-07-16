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

//! Unified media controller protocol abstraction.
//!
//! Every protocol backend (UPnP, Cast, Kodi, …) implements [`MediaController`].
//! The widget dispatches commands through it without knowing the underlying
//! transport. Protocol-specific status callbacks remain separate — each
//! protocol pushes updates through its own callback registered at connect time.

use bmc_wasm_sdk::{log_warn, ufmt};

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
        if bmc_wasm_sdk::fetch(url, None, callback).is_none() {
            log_warn!("{}: album art fetch rejected", self.protocol_name());
        }
    }
}
