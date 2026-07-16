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

//! Audio playback abstraction for keyboard sounds.
//!
//! The keyboard owns its sound assets (compiled from AOSP ogg files, Apache 2.0)
//! and hands them to the host-provided [`AudioSink`] on each play. The sink is
//! responsible for any caching it wants to do — the keyboard never holds a
//! sink-specific handle, so the host is free to swap or rebuild its backend
//! without coordinating with the keyboard.

/// Stable identifier for a sound asset, opaque to the [`AudioSink`].
///
/// The keyboard hands the same tag every time it asks the sink to play a given
/// sound, so the sink can use it as a cache key for its decoded form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundTag(pub &'static str);

/// Which keyboard sound to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySound {
    Standard,
    Delete,
    Return,
    Spacebar,
}

impl KeySound {
    fn asset(self) -> (SoundTag, &'static [u8]) {
        match self {
            Self::Standard => (SoundTag("kb.standard"), SOUND_STANDARD),
            Self::Delete => (SoundTag("kb.delete"), SOUND_DELETE),
            Self::Return => (SoundTag("kb.return"), SOUND_RETURN),
            Self::Spacebar => (SoundTag("kb.spacebar"), SOUND_SPACEBAR),
        }
    }
}

/// Host-provided audio playback capability.
///
/// The sink receives a stable [`SoundTag`] together with the raw asset bytes on
/// every call and decides for itself whether to decode-and-cache or replay.
pub trait AudioSink {
    fn play_sound(&mut self, tag: SoundTag, bytes: &[u8]);
}

/// No-op sink that silently discards all sounds.
#[derive(Debug)]
pub struct SilentSink;

impl AudioSink for SilentSink {
    fn play_sound(&mut self, _tag: SoundTag, _bytes: &[u8]) {}
}

// ── Sound assets ────────────────────────────────────────────────────

static SOUND_STANDARD: &[u8] = include_bytes!("../assets/sounds/KeypressStandard.ogg");
static SOUND_DELETE: &[u8] = include_bytes!("../assets/sounds/KeypressDelete.ogg");
static SOUND_RETURN: &[u8] = include_bytes!("../assets/sounds/KeypressReturn.ogg");
static SOUND_SPACEBAR: &[u8] = include_bytes!("../assets/sounds/KeypressSpacebar.ogg");

/// Play the appropriate sound for a key action.
pub fn play(sink: &mut dyn AudioSink, sound: KeySound) {
    let (tag, bytes) = sound.asset();
    sink.play_sound(tag, bytes);
}
