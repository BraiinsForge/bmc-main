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

use std::collections::{BTreeMap, HashSet};

use bmc_wasm_protocol::{
    BitmapId, BitmapSampling, MeshId, PackageAssetId, SvgId, decode_image_meta,
};

pub(crate) fn cached_bitmap_dimensions(blob: &crate::disk_cache::CachedBlob) -> Option<(u32, u32)> {
    let (width, height, _identity) = decode_image_meta(blob.metadata())?;
    let expected = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(4)?;
    (width != 0 && height != 0 && blob.bytes().len() == expected).then_some((width, height))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AssetBacking {
    Package(PackageAssetId),
    Cache(String),
    Volatile,
}

impl AssetBacking {
    pub(crate) fn is_restorable(&self) -> bool {
        !matches!(self, Self::Volatile)
    }

    pub(crate) fn can_transition_to(&self, next: &Self) -> bool {
        self == next
            || matches!(
                (self, next),
                (Self::Volatile, Self::Cache(_))
                    | (Self::Cache(_), Self::Volatile | Self::Cache(_))
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererAssetKind {
    Svg,
    Bitmap(BitmapSampling),
    Mesh,
}

impl RendererAssetKind {
    #[cfg(feature = "profiling")]
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Bitmap(_) => "bitmap",
            Self::Mesh => "mesh",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum RendererAssetId {
    Svg(SvgId),
    Bitmap(BitmapId),
    Mesh(MeshId),
}

impl RendererAssetId {
    pub(crate) fn to_ffi(self) -> u32 {
        match self {
            Self::Svg(id) => id.to_ffi(),
            Self::Bitmap(id) => id.to_ffi(),
            Self::Mesh(id) => id.to_ffi(),
        }
    }

    pub(crate) fn kind_name(self) -> &'static str {
        match self {
            Self::Svg(_) => "svg",
            Self::Bitmap(_) => "bitmap",
            Self::Mesh(_) => "mesh",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererAssetRecord {
    pub(crate) kind: RendererAssetKind,
    pub(crate) id: RendererAssetId,
    pub(crate) backing: AssetBacking,
    pub(crate) demand_restoration: DemandRestoration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DemandRestoration {
    Pending,
    Resident,
    Unavailable,
}

#[derive(Debug, Default)]
pub(crate) struct RendererAssetLedger {
    records: BTreeMap<String, RendererAssetRecord>,
    owned_ids: HashSet<RendererAssetId>,
    warned_unowned_draw: bool,
}

impl RendererAssetLedger {
    pub(crate) fn get(&self, tag: &str) -> Option<&RendererAssetRecord> {
        self.records.get(tag)
    }

    pub(crate) fn owns(&self, id: RendererAssetId) -> bool {
        self.owned_ids.contains(&id)
    }

    pub(crate) fn should_warn_unowned(&mut self) -> bool {
        !std::mem::replace(&mut self.warned_unowned_draw, true)
    }

    pub(crate) fn record(
        &mut self,
        tag: String,
        mut record: RendererAssetRecord,
    ) -> Result<(), RendererAssetRecord> {
        if let Some(existing) = self.records.get(&tag) {
            if existing.kind != record.kind || existing.id != record.id {
                return Err(record);
            }
            record.demand_restoration = match (existing.demand_restoration, &record.backing) {
                (DemandRestoration::Unavailable, AssetBacking::Cache(_)) => {
                    DemandRestoration::Pending
                }
                (state, _) => state,
            };
        } else if !self.owned_ids.insert(record.id) {
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

    pub(crate) fn has_pending_restorable(&self) -> bool {
        self.records.values().any(|record| {
            record.backing.is_restorable()
                && record.demand_restoration == DemandRestoration::Pending
        })
    }

    pub(crate) fn pending_by_id(
        &self,
        id: RendererAssetId,
    ) -> Option<(String, RendererAssetRecord)> {
        self.records
            .iter()
            .find(|(_, record)| {
                record.backing.is_restorable()
                    && record.demand_restoration == DemandRestoration::Pending
                    && record.id == id
            })
            .map(|(tag, record)| (tag.clone(), record.clone()))
    }

    pub(crate) fn disable_restoration(&mut self, tag: &str) {
        self.set_demand_restoration(tag, DemandRestoration::Unavailable);
    }

    pub(crate) fn mark_pending(&mut self, tag: &str) {
        self.set_demand_restoration(tag, DemandRestoration::Pending);
    }

    pub(crate) fn mark_resident(&mut self, tag: &str) {
        self.set_demand_restoration(tag, DemandRestoration::Resident);
    }

    fn set_demand_restoration(&mut self, tag: &str, state: DemandRestoration) {
        if let Some(record) = self.records.get_mut(tag) {
            record.demand_restoration = state;
        }
    }

    pub(crate) fn remove_prefix(&mut self, prefix: &str) {
        let owned_ids = &mut self.owned_ids;
        self.records.retain(|tag, record| {
            if !bmc_wasm_protocol::tag_matches_prefix(tag, prefix) {
                return true;
            }
            assert!(
                owned_ids.remove(&record.id),
                "BUG: renderer asset record must have an owned ID"
            );
            false
        });
    }

    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.owned_ids.clear();
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
            demand_restoration: DemandRestoration::Pending,
        }
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn record_and_id_kinds_share_the_structured_log_vocabulary() {
        let svg = SvgId::from_ffi(1).expect("BUG: fixture SVG ID must be non-zero");
        let bitmap = BitmapId::from_ffi(1).expect("BUG: fixture bitmap ID must be non-zero");
        let mesh = MeshId::from_ffi(1).expect("BUG: fixture mesh ID must be non-zero");
        for (kind, id) in [
            (RendererAssetKind::Svg, RendererAssetId::Svg(svg)),
            (
                RendererAssetKind::Bitmap(BitmapSampling::Linear),
                RendererAssetId::Bitmap(bitmap),
            ),
            (
                RendererAssetKind::Bitmap(BitmapSampling::Nearest),
                RendererAssetId::Bitmap(bitmap),
            ),
            (RendererAssetKind::Mesh, RendererAssetId::Mesh(mesh)),
        ] {
            assert_eq!(kind.name(), id.kind_name());
        }
    }

    #[test]
    fn only_cache_and_volatile_backings_can_replace_each_other() {
        let package = AssetBacking::Package(PackageAssetId::from_bytes([7; 32]));
        let cache_a = AssetBacking::Cache("a".into());
        let cache_b = AssetBacking::Cache("b".into());
        let volatile = AssetBacking::Volatile;

        assert!(package.can_transition_to(&package));
        assert!(!package.can_transition_to(&cache_a));
        assert!(!cache_a.can_transition_to(&package));
        assert!(cache_a.can_transition_to(&cache_b));
        assert!(cache_a.can_transition_to(&volatile));
        assert!(volatile.can_transition_to(&cache_a));
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
        assert!(!ledger.owns(svg_record(2, AssetBacking::Volatile).id));
    }

    #[test]
    fn repeated_registration_preserves_residency_but_cache_refill_rearms_a_miss() {
        let mut ledger = RendererAssetLedger::default();
        let record = svg_record(1, AssetBacking::Cache("asset".into()));
        let id = record.id;
        ledger
            .record("asset".to_owned(), record.clone())
            .expect("first record must be accepted");
        assert!(ledger.has_pending_restorable());
        ledger.mark_resident("asset");
        assert!(!ledger.has_pending_restorable());
        ledger
            .record("asset".to_owned(), record.clone())
            .expect("repeated record must be accepted");
        assert_eq!(
            ledger.get("asset").map(|record| record.demand_restoration),
            Some(DemandRestoration::Resident)
        );

        ledger.disable_restoration("asset");
        ledger
            .record("asset".to_owned(), record)
            .expect("cache refill must be accepted");
        assert!(ledger.has_pending_restorable());
        assert_eq!(
            ledger.get("asset").map(|record| record.demand_restoration),
            Some(DemandRestoration::Pending)
        );

        ledger.remove_prefix("asset");

        assert!(!ledger.owns(id));
    }

    #[test]
    fn ownership_ends_when_the_widget_evicts_its_tag() {
        let mut ledger = RendererAssetLedger::default();
        let record = svg_record(1, AssetBacking::Volatile);
        let id = record.id;
        ledger
            .record("asset".to_owned(), record)
            .expect("first record must be accepted");

        assert!(ledger.owns(id));

        ledger.remove_prefix("asset");

        assert!(!ledger.owns(id));
    }

    #[test]
    fn clearing_the_ledger_ends_ownership() {
        let mut ledger = RendererAssetLedger::default();
        let record = svg_record(1, AssetBacking::Volatile);
        let id = record.id;
        ledger
            .record("asset".to_owned(), record)
            .expect("first record must be accepted");

        ledger.clear();

        assert!(!ledger.owns(id));
    }

    #[test]
    fn record_rejects_an_id_owned_by_another_tag() {
        let mut ledger = RendererAssetLedger::default();
        let record = svg_record(1, AssetBacking::Volatile);
        let id = record.id;
        ledger
            .record("first".to_owned(), record.clone())
            .expect("first record must be accepted");
        assert!(ledger.record("second".to_owned(), record).is_err());
        assert!(ledger.owns(id));
        assert!(ledger.get("second").is_none());
    }

    #[test]
    fn restoration_state_updates_are_scoped_to_the_ledger_tag() {
        let mut ledger = RendererAssetLedger::default();
        ledger
            .record(
                "first".to_owned(),
                svg_record(1, AssetBacking::Cache("first".into())),
            )
            .expect("first record must be accepted");
        ledger
            .record(
                "second".to_owned(),
                svg_record(2, AssetBacking::Cache("second".into())),
            )
            .expect("second record must be accepted");

        let first_id =
            RendererAssetId::Svg(SvgId::from_ffi(1).expect("BUG: fixture SVG ID must be non-zero"));
        assert_eq!(
            ledger.pending_by_id(first_id).map(|(tag, _)| tag),
            Some("first".to_owned())
        );

        ledger.mark_resident("second");

        assert_eq!(
            ledger.get("first").map(|record| record.demand_restoration),
            Some(DemandRestoration::Pending)
        );
        assert_eq!(
            ledger.get("second").map(|record| record.demand_restoration),
            Some(DemandRestoration::Resident)
        );
        let second_id =
            RendererAssetId::Svg(SvgId::from_ffi(2).expect("BUG: fixture SVG ID must be non-zero"));
        assert!(ledger.pending_by_id(second_id).is_none());
    }

    #[test]
    fn unowned_warning_is_emitted_once_for_the_runtime() {
        let mut ledger = RendererAssetLedger::default();
        let record = svg_record(1, AssetBacking::Volatile);

        assert!(ledger.should_warn_unowned());
        assert!(!ledger.should_warn_unowned());
        ledger
            .record("asset".to_owned(), record)
            .expect("BUG: first record must be accepted");
        ledger.remove_prefix("asset");
        assert!(!ledger.should_warn_unowned());
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
