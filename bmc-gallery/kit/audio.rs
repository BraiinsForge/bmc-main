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

//! Rodio-backed audio sink, so a scene staging a component that makes noise on
//! the device makes it here too.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use bmc_render_keyboard::sound::{AudioSink, SoundTag};
use rodio::source::Source;

/// Pre-decoded PCM samples for instant playback.
#[derive(Clone)]
struct DecodedSound {
    samples: Arc<[f32]>,
    sample_rate: u32,
    channels: u16,
}

/// `rodio::Source` over a shared decoded sample buffer.
///
/// Cheap to construct (clones the `Arc`, no buffer copy) so callers can replay
/// a cached sound on every keystroke without re-allocating the PCM.
struct SharedSamplesSource {
    sound: DecodedSound,
    position: usize,
}

impl Iterator for SharedSamplesSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.sound.samples.get(self.position).copied();
        if sample.is_some() {
            self.position += 1;
        }
        sample
    }
}

impl Source for SharedSamplesSource {
    fn current_frame_len(&self) -> Option<usize> {
        // The whole buffer is one "frame" for rodio's purposes; signal that
        // remaining samples form a single uninterrupted span.
        Some(self.sound.samples.len().saturating_sub(self.position))
    }

    fn channels(&self) -> u16 {
        self.sound.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sound.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        // rodio treats `None` as "unknown duration"; fine for fire-and-forget
        // playback where the caller never inspects this. Avoids both lossy
        // integer division and `usize`→`f64` casts on the hot path.
        None
    }
}

/// Desktop audio sink backed by rodio.
///
/// Decodes each sound on first play and caches it under the consumer-provided
/// [`SoundTag`] for subsequent plays.
///
/// If the host has no default audio backend (rodio's `try_default` fails), the
/// sink falls back to silent — `play_sound` becomes a no-op, and construction
/// logs why once so a quiet keyboard isn't a mystery.
#[expect(missing_debug_implementations, reason = "rodio types are not Debug")]
pub struct RodioSink {
    #[expect(dead_code, reason = "must be kept alive for the stream to work")]
    stream: Option<rodio::OutputStream>,
    handle: Option<rodio::OutputStreamHandle>,
    cache: HashMap<&'static str, DecodedSound>,
}

impl RodioSink {
    #[must_use]
    pub fn new() -> Self {
        let (stream, handle) = match rodio::OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h)),
            Err(e) => {
                tracing::error!(
                    "gallery audio disabled: rodio could not open a default output ({e}). \
                     Sound is silent for this session — fix the audio backend \
                     (PulseAudio / PipeWire / CoreAudio) and restart."
                );
                (None, None)
            }
        };
        Self {
            stream,
            handle,
            cache: HashMap::new(),
        }
    }
}

impl Default for RodioSink {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSink for RodioSink {
    fn play_sound(&mut self, tag: SoundTag, bytes: &[u8]) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        let sound = self.cache.entry(tag.0).or_insert_with(|| {
            let cursor = Cursor::new(bytes.to_vec());
            let decoder = rodio::Decoder::new(cursor).expect("BUG: failed to decode sound");
            let sample_rate = decoder.sample_rate();
            let channels = decoder.channels();
            let samples: Vec<f32> = decoder.convert_samples().collect();
            DecodedSound {
                samples: Arc::from(samples),
                sample_rate,
                channels,
            }
        });
        let source = SharedSamplesSource {
            sound: sound.clone(),
            position: 0,
        };
        let _ = handle.play_raw(source.convert_samples());
    }
}
