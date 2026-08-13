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

use bmc_wasm_protocol::{BitmapId, BitmapSampling, MeshId, SvgId};

pub(crate) fn cached_bitmap_dimensions(blob: &crate::disk_cache::CachedBlob) -> Option<(u32, u32)> {
    let metadata = blob.metadata();
    let width = u32::from_le_bytes(metadata.get(..4)?.try_into().ok()?);
    let height = u32::from_le_bytes(metadata.get(4..8)?.try_into().ok()?);
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)?;
    (width != 0 && height != 0 && blob.bytes().len() == expected).then_some((width, height))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AssetBacking {
    Cache(String),
    Volatile,
}

impl AssetBacking {
    pub(crate) fn is_restorable(&self) -> bool {
        !matches!(self, Self::Volatile)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererAssetKind {
    Svg,
    Bitmap(BitmapSampling),
    Mesh,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererAssetId {
    Svg(SvgId),
    Bitmap(BitmapId),
    Mesh(MeshId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererAssetRecord {
    pub(crate) kind: RendererAssetKind,
    pub(crate) id: RendererAssetId,
    pub(crate) backing: AssetBacking,
}

#[derive(Debug, Default)]
pub(crate) struct RendererAssetLedger {
    records: BTreeMap<String, RendererAssetRecord>,
}

impl RendererAssetLedger {
    pub(crate) fn get(&self, tag: &str) -> Option<&RendererAssetRecord> {
        self.records.get(tag)
    }

    pub(crate) fn record(
        &mut self,
        tag: String,
        record: RendererAssetRecord,
    ) -> Result<(), RendererAssetRecord> {
        if let Some(existing) = self.records.get(&tag)
            && (existing.kind != record.kind || existing.id != record.id)
        {
            return Err(record);
        }
        self.records.insert(tag, record);
        Ok(())
    }

    pub(crate) fn restorable(&self) -> Vec<(String, RendererAssetRecord)> {
        self.records
            .iter()
            .filter(|(_, record)| record.backing.is_restorable())
            .map(|(tag, record)| (tag.clone(), record.clone()))
            .collect()
    }

    pub(crate) fn remove_prefix(&mut self, prefix: &str) {
        self.records
            .retain(|tag, _| !bmc_wasm_protocol::tag_matches_prefix(tag, prefix));
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg_record(id: u32, backing: AssetBacking) -> RendererAssetRecord {
        RendererAssetRecord {
            kind: RendererAssetKind::Svg,
            id: RendererAssetId::Svg(
                SvgId::from_ffi(id).expect("BUG: fixture SVG ID must be non-zero"),
            ),
            backing,
        }
    }

    #[test]
    fn restorable_records_exclude_volatile_assets_and_keep_tag_order() {
        let mut ledger = RendererAssetLedger::default();
        ledger
            .record("volatile".to_owned(), svg_record(1, AssetBacking::Volatile))
            .expect("first record must be accepted");
        ledger
            .record(
                "cache-b".to_owned(),
                svg_record(2, AssetBacking::Cache("b".into())),
            )
            .expect("first record must be accepted");
        ledger
            .record(
                "cache-a".to_owned(),
                svg_record(3, AssetBacking::Cache("a".into())),
            )
            .expect("first record must be accepted");

        assert_eq!(
            ledger
                .restorable()
                .into_iter()
                .map(|(tag, _)| tag)
                .collect::<Vec<_>>(),
            ["cache-a", "cache-b"]
        );
    }

    #[test]
    fn record_rejects_kind_or_id_changes_without_replacing_the_owner() {
        let mut ledger = RendererAssetLedger::default();
        ledger
            .record("asset".to_owned(), svg_record(1, AssetBacking::Volatile))
            .expect("first record must be accepted");

        assert!(
            ledger
                .record("asset".to_owned(), svg_record(2, AssetBacking::Volatile))
                .is_err()
        );
        assert_eq!(
            ledger.get("asset"),
            Some(&svg_record(1, AssetBacking::Volatile))
        );
    }

    #[test]
    fn prefix_removal_uses_renderer_segment_boundaries() {
        let mut ledger = RendererAssetLedger::default();
        for (tag, id) in [("image", 1), ("image:thumb", 2), ("image2", 3)] {
            ledger
                .record(tag.to_owned(), svg_record(id, AssetBacking::Volatile))
                .expect("fixture tags must be distinct");
        }

        ledger.remove_prefix("image");

        assert!(ledger.get("image").is_none());
        assert!(ledger.get("image:thumb").is_none());
        assert!(ledger.get("image2").is_some());
    }
}
