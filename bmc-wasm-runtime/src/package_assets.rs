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

use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};

use bmc_wasm_protocol::{PACKAGE_ASSET_SECTION_NAME, PackageAssetId, PackageAssetKind};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use wasmparser::{Parser, Payload};

const PACKAGE_ASSET_DIGEST_DOMAIN: &[u8] = b"bmc-package-asset-v1\0";
const MAX_PACKAGE_ASSET_PAYLOAD_LEN: u64 = 24 * 1_024 * 1_024;

#[derive(Clone, Debug)]
pub struct PackageAssetStore {
    root: PathBuf,
    /// A share of whatever owns the extraction `root` points into. Opaque:
    /// the store needs it dropped no sooner than itself, never read.
    keep_alive: Option<std::sync::Arc<dyn std::any::Any + Send + Sync>>,
}

#[derive(Debug, Error)]
pub enum PackageAssetError {
    #[error("inspect package asset {path}: {source}")]
    Inspect {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("package asset is not a regular file: {0}")]
    NotRegular(PathBuf),
    #[error("package asset exceeds {MAX_PACKAGE_ASSET_PAYLOAD_LEN} bytes: {0}")]
    TooLarge(PathBuf),
    #[error("read package asset {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("package asset digest mismatch for {kind}/{id}")]
    DigestMismatch {
        kind: &'static str,
        id: PackageAssetId,
    },
}

impl PackageAssetStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            keep_alive: None,
        }
    }

    /// Hold `keeper` for as long as this store lives.
    ///
    /// For a root someone else owns and may replace: reads happen on demand,
    /// so the store cannot rely on the extractor still holding it.
    #[must_use]
    pub fn kept_alive_by(
        mut self,
        keeper: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    ) -> Self {
        self.keep_alive = Some(keeper);
        self
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(
        &self,
        kind: PackageAssetKind,
        id: PackageAssetId,
    ) -> Result<Vec<u8>, PackageAssetError> {
        let path = self
            .root
            .join("v1")
            .join(kind.as_str())
            .join(format!("{id}.asset"));
        let metadata =
            fs::symlink_metadata(&path).map_err(|source| PackageAssetError::Inspect {
                path: path.clone(),
                source,
            })?;
        if !metadata.file_type().is_file() {
            return Err(PackageAssetError::NotRegular(path));
        }
        if metadata.len() > MAX_PACKAGE_ASSET_PAYLOAD_LEN {
            return Err(PackageAssetError::TooLarge(path));
        }
        let file = File::open(&path).map_err(|source| PackageAssetError::Read {
            path: path.clone(),
            source,
        })?;
        let mut payload = Vec::new();
        file.take(MAX_PACKAGE_ASSET_PAYLOAD_LEN + 1)
            .read_to_end(&mut payload)
            .map_err(|source| PackageAssetError::Read {
                path: path.clone(),
                source,
            })?;
        if payload.len() as u64 > MAX_PACKAGE_ASSET_PAYLOAD_LEN {
            return Err(PackageAssetError::TooLarge(path));
        }
        if package_asset_id(kind, &payload) != id {
            return Err(PackageAssetError::DigestMismatch {
                kind: kind.as_str(),
                id,
            });
        }
        Ok(payload)
    }
}

pub(crate) fn reject_embedded_package_assets(wasm: &[u8]) -> anyhow::Result<()> {
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(section) = payload?
            && section.name() == PACKAGE_ASSET_SECTION_NAME
        {
            anyhow::bail!(
                "WebAssembly module contains unstripped {PACKAGE_ASSET_SECTION_NAME} section"
            );
        }
    }
    Ok(())
}

fn package_asset_id(kind: PackageAssetKind, payload: &[u8]) -> PackageAssetId {
    let digest = Sha256::new()
        .chain_update(PACKAGE_ASSET_DIGEST_DOMAIN)
        .chain_update([kind.to_wire()])
        .chain_update(payload)
        .finalize();
    PackageAssetId::from_bytes(digest.into())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::*;

    fn write_asset(root: &Path, kind: PackageAssetKind, payload: &[u8]) -> PackageAssetId {
        let id = package_asset_id(kind, payload);
        let directory = root.join("v1").join(kind.as_str());
        fs::create_dir_all(&directory).expect("BUG: create package asset test directory");
        fs::write(directory.join(format!("{id}.asset")), payload)
            .expect("BUG: write package asset test payload");
        id
    }

    #[test]
    fn a_store_outlives_whoever_extracted_it() {
        let payload = b"compiled svg";
        let owner = std::sync::Arc::new(tempdir().expect("BUG: create package asset directory"));
        let id = write_asset(owner.path(), PackageAssetKind::Svg, payload);
        let store = PackageAssetStore::new(owner.path())
            .kept_alive_by(std::sync::Arc::clone(&owner) as std::sync::Arc<_>);

        // What a hot reload does to the handle the extraction came from.
        drop(owner);

        assert_eq!(
            store
                .load(PackageAssetKind::Svg, id)
                .expect("a store holding its extraction must still read from it"),
            payload
        );
    }

    #[test]
    fn load_accepts_only_the_requested_kind_and_digest() {
        let directory = tempdir().expect("BUG: create package asset test directory");
        let payload = b"compiled svg";
        let id = write_asset(directory.path(), PackageAssetKind::Svg, payload);
        let store = PackageAssetStore::new(directory.path());

        assert_eq!(
            MAX_PACKAGE_ASSET_PAYLOAD_LEN,
            u64::try_from(bmc_wasm_assets::MAX_PACKAGE_ASSET_PAYLOAD_LEN)
                .expect("BUG: package asset payload cap must fit u64")
        );
        assert_eq!(
            package_asset_id(PackageAssetKind::Svg, payload),
            bmc_wasm_assets::package_asset_id(PackageAssetKind::Svg, payload)
        );

        assert_eq!(
            store
                .load(PackageAssetKind::Svg, id)
                .expect("valid package asset should load"),
            payload
        );
        assert!(store.load(PackageAssetKind::Bitmap, id).is_err());

        fs::write(
            directory.path().join("v1/svg").join(format!("{id}.asset")),
            b"changed",
        )
        .expect("BUG: replace package asset test payload");
        assert!(matches!(
            store.load(PackageAssetKind::Svg, id),
            Err(PackageAssetError::DigestMismatch { .. })
        ));
    }

    #[test]
    fn load_rejects_symlinks() {
        let directory = tempdir().expect("BUG: create package asset test directory");
        let payload = b"mesh";
        let id = package_asset_id(PackageAssetKind::Mesh, payload);
        let kind_directory = directory.path().join("v1/mesh");
        fs::create_dir_all(&kind_directory).expect("BUG: create mesh package directory");
        let target = directory.path().join("payload");
        fs::write(&target, payload).expect("BUG: write symlink target");
        symlink(target, kind_directory.join(format!("{id}.asset")))
            .expect("BUG: create package asset symlink");
        let store = PackageAssetStore::new(directory.path());

        assert!(matches!(
            store.load(PackageAssetKind::Mesh, id),
            Err(PackageAssetError::NotRegular(_))
        ));
    }

    #[test]
    fn embedded_transport_section_is_rejected() {
        let wasm = wat::parse_str(r#"(module (@custom "bmc_assets_v1" (after data) "payload"))"#)
            .expect("BUG: build custom-section fixture");
        let result = crate::WasmWidgetRuntime::new(
            &wasm,
            1,
            1,
            bmc_wasm_protocol::ViewportShape::Rectangular,
            crate::RuntimeDisplayInfo {
                width: 1,
                height: 1,
                shape: bmc_wasm_protocol::DisplayShape::Rectangular,
                dpi: 1,
            },
            chrono::Local::now().fixed_offset(),
            crate::RuntimeConfig::default(),
        );
        let Err(error) = result else {
            panic!("runtime construction must reject unstripped package transport");
        };

        assert!(error.to_string().contains(PACKAGE_ASSET_SECTION_NAME));
    }
}
