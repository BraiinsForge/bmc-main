// Copyright (C) 2026  Braiins Systems s.r.o.
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

use index_bmc::{BmcPlatform, BmcRelease};
use index_common::{Index, IndexStatus, NormalizedIndex};

const BMC_INDEX_JSON: &str = include_str!("fixtures/index.v1.json");

fn fixture_value() -> serde_json::Value {
    serde_json::from_str(BMC_INDEX_JSON).expect("BUG: fixture is not valid JSON")
}

#[test]
fn deserializes_and_normalizes_bmc_index_wire_format() {
    let index: Index =
        serde_json::from_str(BMC_INDEX_JSON).expect("representative index must keep parsing");
    let Index::Bmc(variant) = index;

    let normalized = NormalizedIndex::<BmcRelease>::normalize(variant);

    assert!(matches!(normalized.status, IndexStatus::Active));
    assert_eq!(normalized.inaccessible_releases, 0);
    assert_eq!(normalized.releases.len(), 3);

    let latest = &normalized.releases[0];
    assert_eq!(
        latest.bmc_version.to_string(),
        "2025-06-15-0-acde0123-25.06-plus"
    );
    assert!(latest.is_major);
    let asset = latest
        .asset_for_platform(BmcPlatform::Stm32mp157cIi3Bmc1)
        .expect("fixture release carries the stm32mp157c-ii3-bmc1 asset");
    assert_eq!(
        asset.url().as_str(),
        "https://example.com/firmware_2025-06-15-0-acde0123-25.06-plus.tar"
    );
}

#[test]
fn rejects_unknown_top_level_field() {
    let mut value = fixture_value();
    value["brand_new_field"] = serde_json::json!(1);

    serde_json::from_str::<Index>(&value.to_string())
        .expect_err("deny_unknown_fields rejects top-level fields added by newer index versions");
}

#[test]
fn rejects_non_bmc_index_type() {
    let mut value = fixture_value();
    value["type"] = serde_json::json!("toolbox");

    serde_json::from_str::<Index>(&value.to_string())
        .expect_err("only bmc-typed indexes parse in this vendored copy");
}
