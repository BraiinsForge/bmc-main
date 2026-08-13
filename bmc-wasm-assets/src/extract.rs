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

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use bmc_wasm_protocol::{
    PACKAGE_ASSET_RECORD_MAGIC, PACKAGE_ASSET_REF_LEN, PACKAGE_ASSET_REF_MAGIC, PackageAssetId,
    PackageAssetKind, PackageAssetRef,
};
use tempfile::{Builder, NamedTempFile};
use walkdir::WalkDir;
use wasmparser::{Parser, Payload};

use crate::record::parse_record;
use crate::{RecordRef, contains_package_asset_section, rewrite_package_asset_sections};

pub fn extract_package_assets(
    input: &Path,
    artifact_root: Option<&Path>,
    wasm_output: &Path,
    asset_root: &Path,
) -> Result<()> {
    ensure!(
        input != wasm_output,
        "input and stripped WebAssembly output must differ"
    );
    ensure!(
        !wasm_output.exists(),
        "WebAssembly output already exists: {}",
        wasm_output.display()
    );
    ensure!(
        !asset_root.exists(),
        "asset root already exists: {}",
        asset_root.display()
    );

    let input_wasm = fs::read(input).with_context(|| format!("read {}", input.display()))?;
    let rewritten = rewrite_package_asset_sections(&input_wasm)?;
    ensure!(
        !contains_package_asset_section(&rewritten.wasm)?,
        "stripped WebAssembly validation failed"
    );
    let references = package_asset_references(&rewritten.wasm)?;
    let mut candidates = BTreeMap::new();
    add_records(&mut candidates, rewritten.records.iter())?;
    if let Some(artifact_root) = artifact_root {
        add_artifact_records(&mut candidates, artifact_root)?;
    }
    let selected = select_referenced_records(&references, &candidates)?;
    ensure_payloads_absent_from_linear_memory(&rewritten.wasm, selected.values())?;

    let asset_parent = asset_root
        .parent()
        .context("asset root has no parent directory")?;
    let wasm_parent = wasm_output
        .parent()
        .context("WebAssembly output has no parent directory")?;
    fs::create_dir_all(asset_parent)
        .with_context(|| format!("create asset parent {}", asset_parent.display()))?;
    fs::create_dir_all(wasm_parent)
        .with_context(|| format!("create WebAssembly parent {}", wasm_parent.display()))?;

    let asset_stage = Builder::new()
        .prefix(".bmc-assets-")
        .tempdir_in(asset_parent)
        .context("create temporary asset directory")?;
    write_assets(asset_stage.path(), selected.values())?;

    let mut wasm_stage =
        NamedTempFile::new_in(wasm_parent).context("create temporary stripped WebAssembly file")?;
    wasm_stage
        .write_all(&rewritten.wasm)
        .context("write stripped WebAssembly module")?;
    wasm_stage
        .flush()
        .context("flush stripped WebAssembly module")?;

    fs::rename(asset_stage.keep(), asset_root)
        .with_context(|| format!("publish asset root {}", asset_root.display()))?;
    wasm_stage.persist_noclobber(wasm_output).with_context(|| {
        format!(
            "publish stripped WebAssembly module {}",
            wasm_output.display()
        )
    })?;
    Ok(())
}

#[derive(Clone, Debug)]
struct OwnedRecord {
    kind: PackageAssetKind,
    id: PackageAssetId,
    payload: Vec<u8>,
}

fn add_records<'a>(
    candidates: &mut BTreeMap<(u8, PackageAssetId), OwnedRecord>,
    records: impl Iterator<Item = &'a RecordRef<'a>>,
) -> Result<()> {
    for record in records {
        let key = (record.kind.to_wire(), record.id);
        if let Some(previous) = candidates.get(&key) {
            ensure!(
                previous.payload == record.payload,
                "conflicting package asset payloads for {}/{}",
                record.kind.as_str(),
                record.id
            );
        } else {
            candidates.insert(
                key,
                OwnedRecord {
                    kind: record.kind,
                    id: record.id,
                    payload: record.payload.to_vec(),
                },
            );
        }
    }
    Ok(())
}

fn add_artifact_records(
    candidates: &mut BTreeMap<(u8, PackageAssetId), OwnedRecord>,
    artifact_root: &Path,
) -> Result<()> {
    ensure!(
        artifact_root.is_dir(),
        "package asset artifact root is not a directory: {}",
        artifact_root.display()
    );
    for entry in WalkDir::new(artifact_root).follow_links(false) {
        let entry = entry.with_context(|| {
            format!(
                "walk package asset artifact root {}",
                artifact_root.display()
            )
        })?;
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("rlib")
        {
            continue;
        }
        let bytes = fs::read(entry.path())
            .with_context(|| format!("read package asset artifact {}", entry.path().display()))?;
        let mut offset = 0;
        while let Some(relative) = bytes[offset..]
            .windows(PACKAGE_ASSET_RECORD_MAGIC.len())
            .position(|window| window == PACKAGE_ASSET_RECORD_MAGIC)
        {
            let record_offset = offset + relative;
            match parse_record(&bytes[record_offset..]) {
                Ok((record, consumed)) => {
                    add_records(candidates, std::iter::once(&record))?;
                    offset = record_offset + consumed;
                }
                Err(_) => offset = record_offset + 1,
            }
        }
    }
    Ok(())
}

fn package_asset_references(wasm: &[u8]) -> Result<BTreeSet<(u8, PackageAssetId)>> {
    let mut references = BTreeSet::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::DataSection(reader) = payload.context("parse WebAssembly module")? {
            for data in reader {
                let data = data.context("parse WebAssembly data segment")?;
                for offset in data
                    .data
                    .windows(PACKAGE_ASSET_REF_MAGIC.len())
                    .enumerate()
                    .filter_map(|(offset, window)| {
                        (window == PACKAGE_ASSET_REF_MAGIC).then_some(offset)
                    })
                {
                    let Some(bytes) = data.data.get(offset..offset + PACKAGE_ASSET_REF_LEN) else {
                        continue;
                    };
                    let Ok(package_ref) = PackageAssetRef::try_from(bytes) else {
                        continue;
                    };
                    references.insert((package_ref.kind().to_wire(), package_ref.id()));
                }
            }
        }
    }
    Ok(references)
}

fn select_referenced_records(
    references: &BTreeSet<(u8, PackageAssetId)>,
    candidates: &BTreeMap<(u8, PackageAssetId), OwnedRecord>,
) -> Result<BTreeMap<(u8, PackageAssetId), OwnedRecord>> {
    let mut selected = BTreeMap::new();
    for key @ (kind, id) in references {
        let Some(record) = candidates.get(key) else {
            let kind = PackageAssetKind::from_wire(*kind)
                .expect("BUG: package reference parser accepts only known kinds");
            bail!(
                "linked WebAssembly references missing package asset {}/{}",
                kind.as_str(),
                id
            );
        };
        selected.insert(*key, record.clone());
    }
    Ok(selected)
}

fn ensure_payloads_absent_from_linear_memory<'a>(
    wasm: &[u8],
    records: impl Iterator<Item = &'a OwnedRecord>,
) -> Result<()> {
    let mut segments = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::DataSection(reader) = payload.context("parse WebAssembly module")? {
            for data in reader {
                segments.push(
                    data.context("parse WebAssembly data segment")?
                        .data
                        .to_vec(),
                );
            }
        }
    }
    for record in records {
        if !record.payload.is_empty()
            && segments.iter().any(|segment| {
                segment
                    .windows(record.payload.len())
                    .any(|window| window == record.payload)
            })
        {
            bail!(
                "package asset payload {}/{} remains in WebAssembly linear memory",
                record.kind.as_str(),
                record.id
            );
        }
    }
    Ok(())
}

fn write_assets<'a>(root: &Path, records: impl Iterator<Item = &'a OwnedRecord>) -> Result<()> {
    for record in records {
        let directory = root.join("v1").join(record.kind.as_str());
        fs::create_dir_all(&directory)
            .with_context(|| format!("create asset directory {}", directory.display()))?;
        let path = directory.join(format!("{}.asset", record.id));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("create package asset {}", path.display()))?;
        file.write_all(&record.payload)
            .with_context(|| format!("write package asset {}", path.display()))?;
    }
    Ok(())
}
