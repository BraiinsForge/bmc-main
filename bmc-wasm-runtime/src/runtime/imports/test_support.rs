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

//! Imports offered only to a module entered as a test module.
//!
//! No widget imports them, so no shipped host has to provide them.

use anyhow::Result;
use bmc_wasm_protocol::system::Snapshot;
use wasmi::{Caller, Linker};

use crate::host_api::HostState;
use crate::runtime::GuestPanic;
use crate::system::SystemSnapshot;

use super::super::memory::{read_bytes, read_string};

pub(super) fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        "env",
        "host_test_panic",
        |mut caller: Caller<'_, HostState>,
         message_ptr: u32,
         message_len: u32,
         file_ptr: u32,
         file_len: u32,
         line: u32|
         -> Result<(), wasmi::Error> {
            let message = read_string(&caller, message_ptr, message_len).ok_or_else(|| {
                wasmi::Error::new("host_test_panic: the message is not UTF-8 in guest memory")
            })?;
            let file = read_string(&caller, file_ptr, file_len).ok_or_else(|| {
                wasmi::Error::new("host_test_panic: the file name is not UTF-8 in guest memory")
            })?;
            caller.data_mut().guest_panic = Some(GuestPanic {
                message,
                file,
                line,
            });
            Ok(())
        },
    )?;

    linker.func_wrap(
        "env",
        "host_test_install_system",
        |mut caller: Caller<'_, HostState>, ptr: u32, len: u32| -> Result<(), wasmi::Error> {
            let bytes = read_bytes(&caller, ptr, len).ok_or_else(|| {
                wasmi::Error::new("host_test_install_system: the snapshot is outside guest memory")
            })?;
            let snapshot = SystemSnapshot::from(&Snapshot::from_bytes(bytes));
            caller.data_mut().system.replace(snapshot);
            Ok(())
        },
    )?;

    Ok(())
}
