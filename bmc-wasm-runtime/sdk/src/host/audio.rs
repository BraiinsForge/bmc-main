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

use bmc_ui::AudioId;

/// Audio playback volume, in 0–100.
///
/// Values are clamped at construction, so the host never sees an
/// out-of-range byte. Use [`Volume::SILENT`] / [`Volume::FULL`] for the
/// canonical endpoints, [`Volume::new`] for arbitrary values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume(u8);

impl Volume {
    pub const SILENT: Volume = Volume(0);
    pub const FULL: Volume = Volume(100);

    /// Build a volume in 0–100. Values above 100 are clamped down to 100.
    #[must_use]
    pub const fn new(v: u8) -> Self {
        Self(if v > 100 { 100 } else { v })
    }
}

impl From<Volume> for u32 {
    fn from(v: Volume) -> Self {
        Self::from(v.0)
    }
}

/// Play a registered audio sample at the given [`Volume`].
///
/// Fire-and-forget: the host mixes and plays asynchronously. `None`
/// no-ops, so callers can thread an `Option<AudioId>` straight from
/// `ensure_audio_registered` without unwrapping.
pub fn audio_play(sound_id: Option<AudioId>, volume: Volume) {
    let Some(id) = sound_id else { return };
    Platform::play(id, volume);
}

/// Stop playback of a registered audio sample. `None` no-ops.
pub fn audio_stop(sound_id: Option<AudioId>) {
    let Some(id) = sound_id else { return };
    Platform::stop(id);
}

trait AudioSink {
    fn play(id: AudioId, volume: Volume);
    fn stop(id: AudioId);
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use bmc_ui::AudioId;

    use super::Volume;

    #[link(wasm_import_module = "env")]
    unsafe extern "C" {
        fn host_audio_play(sound_id: u32, volume: u32);
        fn host_audio_stop(sound_id: u32);
    }

    pub(super) struct Host;

    impl super::AudioSink for Host {
        fn play(id: AudioId, volume: Volume) {
            unsafe { host_audio_play(id.to_ffi(), u32::from(volume)) }
        }

        fn stop(id: AudioId) {
            unsafe { host_audio_stop(id.to_ffi()) }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use bmc_ui::AudioId;

    use super::Volume;

    pub(super) struct Host;

    impl super::AudioSink for Host {
        fn play(_: AudioId, _: Volume) {
            panic!("BUG: audio_play() called off-target — no host plays audio")
        }

        fn stop(_: AudioId) {
            panic!("BUG: audio_stop() called off-target — no host plays audio")
        }
    }
}

#[cfg(target_arch = "wasm32")]
use wasm::Host as Platform;

#[cfg(not(target_arch = "wasm32"))]
use native::Host as Platform;
