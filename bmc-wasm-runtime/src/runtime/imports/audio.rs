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

//! Audio sample registration and playback guest imports.

use anyhow::Result;
use bmc_wasm_protocol::{AudioId, PackageAssetKind};
use wasmi::{Caller, Linker};

use crate::host_api::{FixtureEvent, FixtureEventKind, HostState};

use super::super::memory::{read_bytes, read_string};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_register_audio_import(linker)?;
    register_package_audio_import(linker)?;
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
            register_audio_data(caller.data_mut(), name, data)
        },
    )?;
    Ok(())
}

fn register_package_audio_import(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_register_audio_package",
        |mut caller: Caller<'_, HostState>, name_ptr: u32, name_len: u32, reference_ptr: u32|
         -> Result<u32, wasmi::Error> {
            let Some(name) = read_string(&caller, name_ptr, name_len) else {
                return Ok(0);
            };
            let Some(package_id) =
                super::read_package_ref(&caller, reference_ptr, PackageAssetKind::Audio)
            else {
                return Ok(0);
            };
            let name = caller.data().namespaced_tag(&name);
            if let Some(id) = caller.data().audio.get_by_tag(&name) {
                return Ok(id.to_wire().into());
            }
            let Some(store) = caller.data().package_assets.as_ref() else {
                return Err(wasmi::Error::new(format!(
                    "widget {} package audio `{name}` ({package_id}) cannot load: package asset store is unavailable",
                    caller.data().instance_id,
                )));
            };
            let data = match store.load(PackageAssetKind::Audio, package_id) {
                Ok(data) => data,
                Err(error) => {
                    return Err(wasmi::Error::new(format!(
                        "widget {} package audio `{name}` ({package_id}) cannot load: {error}",
                        caller.data().instance_id,
                    )));
                }
            };
            Ok(register_audio_data(caller.data_mut(), name, data))
        },
    )?;
    Ok(())
}

fn register_audio_data(state: &mut HostState, name: String, data: Vec<u8>) -> u32 {
    if let Some(id) = state.audio.get_by_tag(&name) {
        return id.to_wire().into();
    }

    #[cfg(feature = "audio")]
    let duration_ms = {
        use rodio::Source as _;
        let cursor = std::io::Cursor::new(data.clone());
        rodio::Decoder::new(cursor)
            .ok()
            .and_then(|decoder| decoder.total_duration())
            .map_or(0, |duration| {
                u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
            })
    };
    #[cfg(not(feature = "audio"))]
    let duration_ms = 0_u32;

    state
        .audio
        .register(name, data.into(), duration_ms)
        .map_or(0, |id| id.to_wire().into())
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

#[cfg(test)]
mod tests {
    use chrono::Local;

    use super::*;
    use crate::audio_registry::AudioRegistry;
    use crate::runtime::FetchAgent;
    use crate::runtime_limits::RuntimeResourceLimits;

    #[test]
    fn exhausted_registry_returns_absent_wire_id() {
        let mut state = HostState::new(
            RuntimeResourceLimits::default(),
            Local::now().fixed_offset(),
            FetchAgent::default(),
        );
        state.audio = AudioRegistry::with_id_cap(2);

        assert_ne!(register_audio_data(&mut state, "first".into(), vec![]), 0);
        assert_eq!(register_audio_data(&mut state, "second".into(), vec![]), 0);
    }
}
