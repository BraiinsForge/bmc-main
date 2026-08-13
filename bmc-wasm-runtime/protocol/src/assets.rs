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

use core::fmt;
use core::str::FromStr;

pub const PACKAGE_ASSET_SECTION_NAME: &str = "bmc_assets_v1";
pub const PACKAGE_ASSET_RECORD_MAGIC: [u8; 8] = *b"BMCASSET";
pub const PACKAGE_ASSET_FORMAT_VERSION: u16 = 1;
pub const PACKAGE_ASSET_FORMAT_FLAGS: u8 = 0;
pub const PACKAGE_ASSET_ID_LEN: usize = 32;
pub const PACKAGE_ASSET_ID_HEX_LEN: usize = PACKAGE_ASSET_ID_LEN * 2;
pub const PACKAGE_ASSET_REF_MAGIC: [u8; 8] = *b"BMCREFV1";
pub const PACKAGE_ASSET_REF_LEN: usize = PACKAGE_ASSET_REF_MAGIC.len() + 1 + PACKAGE_ASSET_ID_LEN;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageAssetId([u8; PACKAGE_ASSET_ID_LEN]);

impl PackageAssetId {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; PACKAGE_ASSET_ID_LEN]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PACKAGE_ASSET_ID_LEN] {
        &self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageAssetRef([u8; PACKAGE_ASSET_REF_LEN]);

impl PackageAssetRef {
    #[must_use]
    pub const fn new(kind: PackageAssetKind, id: PackageAssetId) -> Self {
        let mut bytes = [0; PACKAGE_ASSET_REF_LEN];
        let mut index = 0;
        while index < PACKAGE_ASSET_REF_MAGIC.len() {
            bytes[index] = PACKAGE_ASSET_REF_MAGIC[index];
            index += 1;
        }
        bytes[PACKAGE_ASSET_REF_MAGIC.len()] = kind.to_wire();
        let mut id_index = 0;
        while id_index < PACKAGE_ASSET_ID_LEN {
            bytes[PACKAGE_ASSET_REF_MAGIC.len() + 1 + id_index] = id.as_bytes()[id_index];
            id_index += 1;
        }
        Self(bytes)
    }

    #[must_use]
    pub fn kind(self) -> PackageAssetKind {
        PackageAssetKind::from_wire(self.0[PACKAGE_ASSET_REF_MAGIC.len()])
            .expect("BUG: package asset reference contains a validated kind")
    }

    #[must_use]
    pub fn id(self) -> PackageAssetId {
        PackageAssetId::from_bytes(
            self.0[PACKAGE_ASSET_REF_MAGIC.len() + 1..]
                .try_into()
                .expect("BUG: package asset reference ID slice has the declared length"),
        )
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PACKAGE_ASSET_REF_LEN] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageAssetRefError {
    Length(usize),
    Magic,
    Kind(u8),
}

impl TryFrom<&[u8]> for PackageAssetRef {
    type Error = PackageAssetRefError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() != PACKAGE_ASSET_REF_LEN {
            return Err(PackageAssetRefError::Length(bytes.len()));
        }
        if bytes[..PACKAGE_ASSET_REF_MAGIC.len()] != PACKAGE_ASSET_REF_MAGIC {
            return Err(PackageAssetRefError::Magic);
        }
        let kind = bytes[PACKAGE_ASSET_REF_MAGIC.len()];
        PackageAssetKind::from_wire(kind).map_err(|_| PackageAssetRefError::Kind(kind))?;
        Ok(Self(bytes.try_into().expect(
            "BUG: checked package asset reference length must fit its array",
        )))
    }
}

/// Target-specific source for an asset produced by a static asset macro.
///
/// Packaged WASM carries only a package reference.
/// Native storybook builds bypass widget packaging and retain the processed payload.
#[derive(Clone, Copy, Debug)]
pub struct StaticAssetSource {
    #[cfg(target_arch = "wasm32")]
    package_ref: &'static PackageAssetRef,
    #[cfg(not(target_arch = "wasm32"))]
    data: &'static [u8],
}

impl StaticAssetSource {
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub const fn package(package_ref: &'static PackageAssetRef) -> Self {
        Self { package_ref }
    }

    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub const fn package_ref(self) -> &'static PackageAssetRef {
        self.package_ref
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn embedded(data: &'static [u8]) -> Self {
        Self { data }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn data(self) -> &'static [u8] {
        self.data
    }
}

impl fmt::Display for PackageAssetId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParsePackageAssetIdError {
    Length(usize),
    NonLowercaseHex,
    InvalidHex,
}

impl fmt::Display for ParsePackageAssetIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(length) => write!(
                formatter,
                "package asset ID has {length} hexadecimal characters; expected {PACKAGE_ASSET_ID_HEX_LEN}"
            ),
            Self::NonLowercaseHex => {
                formatter.write_str("package asset ID must use lowercase hexadecimal")
            }
            Self::InvalidHex => {
                formatter.write_str("package asset ID contains invalid hexadecimal")
            }
        }
    }
}

impl FromStr for PackageAssetId {
    type Err = ParsePackageAssetIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != PACKAGE_ASSET_ID_HEX_LEN {
            return Err(ParsePackageAssetIdError::Length(value.len()));
        }
        if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(ParsePackageAssetIdError::NonLowercaseHex);
        }
        let mut bytes = [0; PACKAGE_ASSET_ID_LEN];
        hex::decode_to_slice(value, &mut bytes)
            .map_err(|_| ParsePackageAssetIdError::InvalidHex)?;
        Ok(Self(bytes))
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PackageAssetKind {
    Svg,
    Bitmap,
    Mesh,
    Audio,
}

impl PackageAssetKind {
    #[must_use]
    pub const fn to_wire(self) -> u8 {
        match self {
            Self::Svg => 1,
            Self::Bitmap => 2,
            Self::Mesh => 3,
            Self::Audio => 4,
        }
    }

    pub const fn from_wire(value: u8) -> Result<Self, UnknownPackageAssetKind> {
        match value {
            1 => Ok(Self::Svg),
            2 => Ok(Self::Bitmap),
            3 => Ok(Self::Mesh),
            4 => Ok(Self::Audio),
            other => Err(UnknownPackageAssetKind(other)),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Bitmap => "bitmap",
            Self::Mesh => "mesh",
            Self::Audio => "audio",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnknownPackageAssetKind(pub u8);

impl fmt::Display for UnknownPackageAssetKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown package asset kind {}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BitmapSampling {
    Linear,
    Nearest,
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use super::{
        PACKAGE_ASSET_ID_LEN, PACKAGE_ASSET_REF_LEN, PackageAssetId, PackageAssetKind,
        PackageAssetRef, PackageAssetRefError, ParsePackageAssetIdError,
    };

    #[test]
    fn package_asset_kinds_round_trip_through_their_wire_values() {
        for kind in [
            PackageAssetKind::Svg,
            PackageAssetKind::Bitmap,
            PackageAssetKind::Mesh,
            PackageAssetKind::Audio,
        ] {
            assert_eq!(PackageAssetKind::from_wire(kind.to_wire()), Ok(kind));
        }
    }

    #[test]
    fn unknown_package_asset_kind_is_rejected() {
        assert!(PackageAssetKind::from_wire(0).is_err());
        assert!(PackageAssetKind::from_wire(5).is_err());
    }

    #[test]
    fn package_asset_id_round_trips_lowercase_hex() {
        let id = PackageAssetId::from_bytes([0xab; PACKAGE_ASSET_ID_LEN]);
        assert_eq!(PackageAssetId::from_str(&id.to_hex()), Ok(id));
    }

    #[test]
    fn package_asset_id_rejects_bad_length_and_hex() {
        assert_eq!(
            PackageAssetId::from_str("ab"),
            Err(ParsePackageAssetIdError::Length(2))
        );
        assert_eq!(
            PackageAssetId::from_str(&"AB".repeat(PACKAGE_ASSET_ID_LEN)),
            Err(ParsePackageAssetIdError::NonLowercaseHex)
        );
        assert_eq!(
            PackageAssetId::from_str(&"zz".repeat(PACKAGE_ASSET_ID_LEN)),
            Err(ParsePackageAssetIdError::InvalidHex)
        );
    }

    #[test]
    fn package_asset_reference_round_trips_kind_and_complete_id() {
        let id = PackageAssetId::from_bytes([0xab; PACKAGE_ASSET_ID_LEN]);
        let reference = PackageAssetRef::new(PackageAssetKind::Mesh, id);
        let parsed = PackageAssetRef::try_from(reference.as_bytes().as_slice())
            .expect("BUG: generated package reference must parse");

        assert_eq!(parsed.kind(), PackageAssetKind::Mesh);
        assert_eq!(parsed.id(), id);
        assert_eq!(reference.as_bytes().len(), PACKAGE_ASSET_REF_LEN);
    }

    #[test]
    fn package_asset_reference_rejects_wrong_length_magic_and_kind() {
        let reference = PackageAssetRef::new(
            PackageAssetKind::Bitmap,
            PackageAssetId::from_bytes([0xcd; PACKAGE_ASSET_ID_LEN]),
        );
        let mut bytes = *reference.as_bytes();
        assert_eq!(
            PackageAssetRef::try_from(&bytes[..PACKAGE_ASSET_REF_LEN - 1]),
            Err(PackageAssetRefError::Length(PACKAGE_ASSET_REF_LEN - 1))
        );
        bytes[0] ^= 1;
        assert_eq!(
            PackageAssetRef::try_from(bytes.as_slice()),
            Err(PackageAssetRefError::Magic)
        );
        bytes = *reference.as_bytes();
        bytes[8] = 0;
        assert_eq!(
            PackageAssetRef::try_from(bytes.as_slice()),
            Err(PackageAssetRefError::Kind(0))
        );
    }
}
