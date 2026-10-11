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

//! A widget package as a host loads it:
//! the runtime module and the package asset root it reads from,
//! split out of the compiler's output into a temporary directory
//! unless the caller already holds both.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};

#[derive(Debug)]
pub struct PreparedWidget {
    wasm_path: PathBuf,
    asset_root: PathBuf,
    /// Shared, so an extraction outlives this handle —
    /// a hot reload replaces it while runtimes that read from it by path are still installed.
    temporary_directory: Option<Arc<tempfile::TempDir>>,
}

impl PreparedWidget {
    pub fn new(source_wasm: &Path, asset_root: Option<&Path>) -> Result<Self> {
        if let Some(asset_root) = asset_root {
            ensure!(
                asset_root.is_dir(),
                "package asset root is not a directory: {}",
                asset_root.display()
            );
            return Ok(Self {
                wasm_path: source_wasm.to_owned(),
                asset_root: asset_root.to_owned(),
                temporary_directory: None,
            });
        }

        let temporary_directory = tempfile::Builder::new()
            .prefix("bmc-widget-")
            .tempdir()
            .context("create temporary prepared widget directory")?;
        let wasm_path = temporary_directory.path().join("widget.wasm");
        let asset_root = temporary_directory.path().join("assets");
        let artifact_root = source_wasm
            .parent()
            .map(|directory| {
                if directory.file_name().is_some_and(|name| name == "deps") {
                    directory.to_owned()
                } else {
                    directory.join("deps")
                }
            })
            .filter(|directory| directory.is_dir());
        bmc_wasm_assets::extract_package_assets(
            source_wasm,
            artifact_root.as_deref(),
            &wasm_path,
            &asset_root,
        )
        .with_context(|| format!("prepare package assets from {}", source_wasm.display()))?;

        Ok(Self {
            wasm_path,
            asset_root,
            temporary_directory: Some(Arc::new(temporary_directory)),
        })
    }

    #[must_use]
    pub fn wasm_path(&self) -> &Path {
        &self.wasm_path
    }

    #[must_use]
    pub fn asset_root(&self) -> &Path {
        &self.asset_root
    }

    /// A store on the asset root, holding whatever keeps it on disk.
    #[must_use]
    pub fn asset_store(&self) -> crate::package_assets::PackageAssetStore {
        let store = crate::package_assets::PackageAssetStore::new(&self.asset_root);
        match &self.temporary_directory {
            Some(keeper) => store.kept_alive_by(Arc::clone(keeper) as Arc<_>),
            None => store,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use bmc_ui::{PackageAssetKind, PackageAssetRef};
    use bmc_wasm_assets::{Records, contains_package_asset_section, encode_record};
    use bmc_wasm_protocol::PACKAGE_ASSET_SECTION_NAME;
    use wasm_encoder::{CustomSection, DataSection, Module};

    use super::*;

    #[test]
    fn extracts_assets_before_runtime_load() {
        let record = encode_record(PackageAssetKind::Bitmap, "cover", b"encoded-image")
            .expect("encode package asset fixture");
        let id = Records::new(&record)
            .next()
            .expect("one package asset record")
            .expect("parse package asset fixture")
            .id;
        let mut compiler_wasm = Module::new();
        compiler_wasm.section(&CustomSection {
            name: PACKAGE_ASSET_SECTION_NAME.into(),
            data: record.as_slice().into(),
        });
        let mut linear_data = DataSection::new();
        linear_data.passive(
            PackageAssetRef::new(PackageAssetKind::Bitmap, id)
                .as_bytes()
                .iter()
                .copied(),
        );
        compiler_wasm.section(&linear_data);
        let directory = tempfile::tempdir().expect("create compiler output directory");
        let input = directory.path().join("widget.wasm");
        fs::write(&input, compiler_wasm.finish()).expect("write compiler output");

        let prepared = PreparedWidget::new(&input, None).expect("prepare widget package");

        let runtime_wasm = fs::read(prepared.wasm_path()).expect("read runtime module");
        assert!(
            !contains_package_asset_section(&runtime_wasm).expect("inspect runtime module"),
            "runtime module must not retain package asset records"
        );
        assert_eq!(
            fs::read(
                prepared
                    .asset_root()
                    .join("v1/bitmap")
                    .join(format!("{id}.asset"))
            )
            .expect("read extracted package asset"),
            b"encoded-image"
        );
    }
}
