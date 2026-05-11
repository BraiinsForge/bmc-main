// Copyright (C) 2026  Braiins Systems s.r.o.

//! Audio sample registration and playback guest imports.

use anyhow::Result;
use bmc_wasm_protocol::AudioId;
use wasmi::{Caller, Linker};

use crate::host_api::{FixtureEvent, FixtureEventKind, HostState};

use super::super::memory::{read_bytes, read_string};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_register_audio_import(linker)?;
    register_audio_play_import(linker)?;
    register_audio_stop_import(linker)?;
    Ok(())
}

fn register_register_audio_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_audio",
        |mut caller: Caller<'_, HostState>,
         data_ptr: u32,
         data_len: u32,
         name_ptr: u32,
         name_len: u32|
         -> u32 {
            let Some(data) = read_bytes(&caller, data_ptr, data_len) else {
                return 0;
            };
            let name =
                read_string(&caller, name_ptr, name_len).unwrap_or_else(|| "unknown".to_owned());
            let name = caller.data().namespaced_tag(&name);

            if let Some(id) = caller.data().audio.get_by_tag(&name) {
                return id.to_wire().into();
            }

            #[cfg(feature = "audio")]
            let duration_ms = {
                use rodio::Source as _;
                let cursor = std::io::Cursor::new(data.clone());
                rodio::Decoder::new(cursor)
                    .ok()
                    .and_then(|d| d.total_duration())
                    .map_or(0, |d| u32::try_from(d.as_millis()).unwrap_or(u32::MAX))
            };
            #[cfg(not(feature = "audio"))]
            let duration_ms = 0_u32;

            let id = caller
                .data_mut()
                .audio
                .register(name, data.into(), duration_ms);
            id.to_wire().into()
        },
    )?;
    Ok(())
}

fn register_audio_play_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_audio_play",
        |mut caller: Caller<'_, HostState>, sound_id: u32, volume: u32| {
            let state = caller.data_mut();

            let Ok(raw) = u16::try_from(sound_id) else {
                return;
            };
            let Some(id) = AudioId::from_wire(raw) else {
                return;
            };
            let Some(sample) = state.audio.get(id) else {
                return;
            };

            let sample_name = sample.name.clone();
            let sample_duration_ms = sample.duration_ms;
            let data = sample.data.clone();

            if state.record_events {
                state.recorded_events.push(FixtureEvent {
                    at_ms: state.monotonic_ms,
                    kind: FixtureEventKind::AudioPlay {
                        sound_id: id,
                        volume,
                        name: sample_name,
                        duration_ms: sample_duration_ms,
                    },
                });
            }

            #[cfg(feature = "audio")]
            {
                let Some((_, ref handle)) = state.audio_stream else {
                    return;
                };
                let cursor = std::io::Cursor::new(data);
                if let Ok(decoder) = rodio::Decoder::new(cursor)
                    && let Ok(sink) = rodio::Sink::try_new(handle)
                {
                    let volume = u8::try_from(volume.min(100))
                        .expect("BUG: volume bounded to 100 must fit in u8");
                    let vol = f32::from(volume) / 100.0;
                    sink.set_volume(vol);
                    sink.append(decoder);

                    state.audio.push_sink(id, sink);
                }
            }

            #[cfg(not(feature = "audio"))]
            let _ = (data, volume);
        },
    )?;
    Ok(())
}

fn register_audio_stop_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_audio_stop",
        |caller: Caller<'_, HostState>, sound_id: u32| {
            let Ok(raw) = u16::try_from(sound_id) else {
                return;
            };
            let Some(id) = AudioId::from_wire(raw) else {
                return;
            };
            #[cfg(feature = "audio")]
            {
                let mut caller = caller;
                caller.data_mut().audio.stop(id);
            }
            #[cfg(not(feature = "audio"))]
            let _ = (caller, id);
        },
    )?;
    Ok(())
}
