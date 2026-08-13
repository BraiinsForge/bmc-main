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

use anyhow::{Context, Result, ensure};
use bmc_wasm_protocol::PACKAGE_ASSET_SECTION_NAME;
use wasmparser::{Parser, Payload};

use crate::{RecordRef, Records};

#[derive(Debug)]
pub struct RewrittenModule<'a> {
    pub wasm: Vec<u8>,
    pub records: Vec<RecordRef<'a>>,
}

pub fn contains_package_asset_section(wasm: &[u8]) -> Result<bool> {
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(section) = payload.context("parse WebAssembly module")?
            && section.name() == PACKAGE_ASSET_SECTION_NAME
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn rewrite_package_asset_sections(wasm: &[u8]) -> Result<RewrittenModule<'_>> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    let mut records = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.context("parse WebAssembly module")?;
        if let Payload::CustomSection(section) = &payload
            && section.name() == PACKAGE_ASSET_SECTION_NAME
        {
            records.extend(
                Records::new(section.data())
                    .collect::<Result<Vec<_>, _>>()
                    .context("parse package asset records")?,
            );
            continue;
        }
        if let Some((id, range)) = payload.as_section() {
            let data = wasm
                .get(range)
                .context("parser returned an out-of-bounds section range")?;
            append_section(&mut module, id, data)?;
        }
    }
    let wasm = module;
    ensure!(
        !contains_package_asset_section(&wasm)?,
        "rewritten WebAssembly module still contains package asset records"
    );
    Ok(RewrittenModule { wasm, records })
}

fn append_section(module: &mut Vec<u8>, id: u8, data: &[u8]) -> Result<()> {
    module.push(id);
    let mut remaining = u32::try_from(data.len()).context("WebAssembly section exceeds 4 GiB")?;
    loop {
        let mut byte = u8::try_from(remaining & 0x7f).expect("BUG: seven bits always fit in u8");
        remaining >>= 7;
        if remaining != 0 {
            byte |= 0x80;
        }
        module.push(byte);
        if remaining == 0 {
            break;
        }
    }
    module.extend_from_slice(data);
    Ok(())
}
