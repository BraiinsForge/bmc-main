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

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use tempfile::{Builder, NamedTempFile};

use crate::{RecordRef, contains_package_asset_section, rewrite_package_asset_sections};

pub fn extract_package_assets(input: &Path, wasm_output: &Path, asset_root: &Path) -> Result<()> {
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
    let unique = deduplicate_records(&rewritten.records)?;

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
    write_assets(asset_stage.path(), unique.values().copied())?;

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

fn deduplicate_records<'a>(
    records: &'a [RecordRef<'a>],
) -> Result<BTreeMap<(u8, bmc_wasm_protocol::PackageAssetId), &'a RecordRef<'a>>> {
    let mut unique = BTreeMap::new();
    for record in records {
        let key = (record.kind.to_wire(), record.id);
        if let Some(previous) = unique.insert(key, record)
            && previous.payload != record.payload
        {
            bail!(
                "conflicting package asset payloads for {}/{}",
                record.kind.as_str(),
                record.id
            );
        }
    }
    Ok(unique)
}

fn write_assets<'a>(root: &Path, records: impl Iterator<Item = &'a RecordRef<'a>>) -> Result<()> {
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
        file.write_all(record.payload)
            .with_context(|| format!("write package asset {}", path.display()))?;
    }
    Ok(())
}
