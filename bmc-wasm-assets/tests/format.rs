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

use std::fs;

use assert_matches::assert_matches;
use bmc_wasm_assets::{
    MAX_PACKAGE_ASSET_PAYLOAD_LEN, RecordError, Records, contains_package_asset_section,
    encode_record, extract_package_assets, rewrite_package_asset_sections,
};
use bmc_wasm_protocol::{PACKAGE_ASSET_SECTION_NAME, PackageAssetKind};
use tempfile::tempdir;
use wasmparser::{Parser, Payload};

const ID_OFFSET: usize = 12;
const LOGICAL_NAME_LEN_OFFSET: usize = 44;
const PAYLOAD_LEN_OFFSET: usize = 48;
const HEADER_LEN: usize = 56;

#[test]
fn records_round_trip_every_kind_and_concatenate_without_padding() {
    let kinds = [
        PackageAssetKind::Svg,
        PackageAssetKind::Bitmap,
        PackageAssetKind::Mesh,
        PackageAssetKind::Audio,
    ];
    let mut section = Vec::new();
    for kind in kinds {
        section.extend(
            encode_record(kind, kind.as_str(), &[kind.to_wire(); 3])
                .expect("encode fixture record"),
        );
    }
    let records = Records::new(&section)
        .collect::<Result<Vec<_>, _>>()
        .expect("parse concatenated fixture records");
    assert_eq!(records.len(), kinds.len());
    for (record, kind) in records.iter().zip(kinds) {
        assert_eq!(record.kind, kind);
        assert_eq!(record.logical_name, kind.as_str());
        assert_eq!(record.payload, [kind.to_wire(); 3]);
    }
}

#[test]
fn record_rejects_truncated_and_trailing_bytes() {
    let record =
        encode_record(PackageAssetKind::Svg, "icon", b"svg").expect("encode fixture record");
    assert_matches!(
        Records::new(&record[..HEADER_LEN - 1]).next(),
        Some(Err(RecordError::Truncated("header")))
    );
    let mut trailing = record;
    trailing.push(0xff);
    let results = Records::new(&trailing).collect::<Vec<_>>();
    assert_matches!(
        results.as_slice(),
        [Ok(_), Err(RecordError::Truncated("header"))]
    );
}

#[test]
fn record_rejects_unknown_header_values() {
    let record =
        encode_record(PackageAssetKind::Bitmap, "image", b"png").expect("encode fixture record");
    for (offset, value, expected) in [
        (8, 2, RecordError::UnsupportedVersion(2)),
        (10, 0xff, RecordError::UnknownKind(0xff)),
        (11, 1, RecordError::UnsupportedFlags(1)),
    ] {
        let mut malformed = record.clone();
        malformed[offset] = value;
        assert_eq!(
            Records::new(&malformed).next(),
            Some(Err(expected)),
            "header byte {offset}"
        );
    }
}

#[test]
fn record_rejects_invalid_name_digest_and_declared_lengths() {
    let record =
        encode_record(PackageAssetKind::Mesh, "mesh", b"payload").expect("encode fixture record");

    let mut invalid_name = record.clone();
    invalid_name[HEADER_LEN] = 0xff;
    assert_matches!(
        Records::new(&invalid_name).next(),
        Some(Err(RecordError::LogicalNameUtf8))
    );

    let mut invalid_digest = record.clone();
    invalid_digest[ID_OFFSET] ^= 1;
    assert_matches!(
        Records::new(&invalid_digest).next(),
        Some(Err(RecordError::DigestMismatch))
    );

    let mut truncated_name = record.clone();
    truncated_name[LOGICAL_NAME_LEN_OFFSET..LOGICAL_NAME_LEN_OFFSET + 4]
        .copy_from_slice(&u32::MAX.to_le_bytes());
    assert_matches!(
        Records::new(&truncated_name).next(),
        Some(Err(RecordError::Truncated("logical name")))
    );

    let mut oversized_payload = record;
    oversized_payload[PAYLOAD_LEN_OFFSET..PAYLOAD_LEN_OFFSET + 8].copy_from_slice(
        &u64::try_from(MAX_PACKAGE_ASSET_PAYLOAD_LEN + 1)
            .expect("limit fits u64")
            .to_le_bytes(),
    );
    assert_matches!(
        Records::new(&oversized_payload).next(),
        Some(Err(RecordError::PayloadTooLarge { .. }))
    );
}

#[test]
fn payload_limit_accepts_the_boundary_and_rejects_the_next_byte() {
    let payload = vec![0; MAX_PACKAGE_ASSET_PAYLOAD_LEN];
    let record = encode_record(PackageAssetKind::Audio, "largest", &payload)
        .expect("encode boundary record");
    assert_eq!(
        Records::new(&record)
            .next()
            .expect("one record")
            .expect("parse boundary record")
            .payload
            .len(),
        MAX_PACKAGE_ASSET_PAYLOAD_LEN
    );
    let too_large = vec![0; MAX_PACKAGE_ASSET_PAYLOAD_LEN + 1];
    assert_matches!(
        encode_record(PackageAssetKind::Audio, "too-large", &too_large),
        Err(RecordError::PayloadTooLarge { .. })
    );
}

#[test]
fn rewrite_removes_only_asset_sections_and_preserves_other_section_payloads() {
    let first = encode_record(PackageAssetKind::Svg, "first", b"one").expect("encode first record");
    let second =
        encode_record(PackageAssetKind::Bitmap, "second", b"two").expect("encode second record");
    let input = module_with_custom_sections(&[
        ("before", b"ordinary-before"),
        (PACKAGE_ASSET_SECTION_NAME, &first),
        ("after", b"ordinary-after"),
        (PACKAGE_ASSET_SECTION_NAME, &second),
    ]);

    let rewritten = rewrite_package_asset_sections(&input).expect("rewrite fixture module");
    assert_eq!(rewritten.records.len(), 2);
    assert!(!contains_package_asset_section(&rewritten.wasm).expect("inspect rewritten module"));
    assert_eq!(
        ordinary_custom_sections(&rewritten.wasm),
        vec![
            ("before".to_owned(), b"ordinary-before".to_vec()),
            ("after".to_owned(), b"ordinary-after".to_vec()),
        ]
    );
}

#[test]
fn extractor_writes_content_addressed_assets_and_a_stripped_module() {
    let record = encode_record(PackageAssetKind::Bitmap, "image", b"encoded-image")
        .expect("encode extraction record");
    let id = Records::new(&record)
        .next()
        .expect("one record")
        .expect("parse extraction record")
        .id;
    let module = module_with_custom_sections(&[(PACKAGE_ASSET_SECTION_NAME, &record)]);
    let temporary = tempdir().expect("create fixture directory");
    let input = temporary.path().join("compiler.wasm");
    let output = temporary.path().join("packaged.wasm");
    let assets = temporary.path().join("assets");
    fs::write(&input, module).expect("write compiler fixture");

    extract_package_assets(&input, &output, &assets).expect("extract fixture assets");

    let output_wasm = fs::read(output).expect("read stripped fixture");
    assert!(!contains_package_asset_section(&output_wasm).expect("inspect stripped fixture"));
    assert_eq!(
        fs::read(assets.join("v1/bitmap").join(format!("{id}.asset")))
            .expect("read extracted bitmap"),
        b"encoded-image"
    );
}

fn module_with_custom_sections(sections: &[(&str, &[u8])]) -> Vec<u8> {
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    for (name, data) in sections {
        let mut payload = Vec::new();
        encode_u32_leb(&mut payload, name.len());
        payload.extend_from_slice(name.as_bytes());
        payload.extend_from_slice(data);
        module.push(0);
        encode_u32_leb(&mut module, payload.len());
        module.extend_from_slice(&payload);
    }
    module
}

fn encode_u32_leb(output: &mut Vec<u8>, value: usize) {
    let mut remaining = u32::try_from(value).expect("fixture length fits in u32");
    loop {
        let mut byte = u8::try_from(remaining & 0x7f).expect("BUG: seven bits always fit in u8");
        remaining >>= 7;
        if remaining != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if remaining == 0 {
            break;
        }
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "wasmparser::Payload is non-exhaustive"
)]
fn ordinary_custom_sections(wasm: &[u8]) -> Vec<(String, Vec<u8>)> {
    Parser::new(0)
        .parse_all(wasm)
        .filter_map(|payload| match payload.expect("parse fixture module") {
            Payload::CustomSection(section) => {
                Some((section.name().to_owned(), section.data().to_vec()))
            }
            _ => None,
        })
        .collect()
}
