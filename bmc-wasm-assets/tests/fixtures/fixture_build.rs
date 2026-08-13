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

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use bmc_wasm_assets::{Records, encode_record};
use bmc_wasm_protocol::{PACKAGE_ASSET_REF_MAGIC, PackageAssetKind};

pub fn emit_record(kind: PackageAssetKind, logical_name: &str, payload_path: &str) {
    println!("cargo::rerun-if-changed={payload_path}");
    let payload = fs::read(payload_path).expect("read linker fixture payload");
    let record = encode_record(kind, logical_name, &payload).expect("encode linker fixture record");
    let id = Records::new(&record)
        .next()
        .expect("one generated record")
        .expect("parse generated record")
        .id;
    let mut source = String::new();
    writeln!(source, "#[expect(dead_code)]").expect("write generated attribute");
    writeln!(source, "#[unsafe(link_section = \"bmc_assets_v1\")]")
        .expect("write generated link section");
    writeln!(source, "static ASSET_RECORD: [u8; {}] = [", record.len())
        .expect("write generated record declaration");
    for chunk in record.chunks(16) {
        source.push_str("    ");
        for byte in chunk {
            write!(source, "0x{byte:02x}, ").expect("write generated record byte");
        }
        source.push('\n');
    }
    source.push_str("];\n");
    let reference = PACKAGE_ASSET_REF_MAGIC
        .into_iter()
        .chain(std::iter::once(kind.to_wire()))
        .chain(id.as_bytes().iter().copied())
        .collect::<Vec<_>>();
    writeln!(
        source,
        "pub static ASSET_REF: [u8; {}] = [",
        reference.len()
    )
    .expect("write generated asset reference declaration");
    for chunk in reference.chunks(16) {
        source.push_str("    ");
        for byte in chunk {
            write!(source, "0x{byte:02x}, ").expect("write generated asset reference byte");
        }
        source.push('\n');
    }
    source.push_str("];\n");
    let output =
        Path::new(&env::var_os("OUT_DIR").expect("OUT_DIR is set")).join("asset_record.rs");
    fs::write(output, source).expect("write generated linker fixture source");
}
