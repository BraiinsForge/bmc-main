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

use bmc_platform::Product;
use bmc_widget_manifest::{ParamKey, ParamValue};
use uuid::Uuid;

use crate::config::widget_uuids::{
    BITCOIN_MINING_DATA_UID, BLOCK_HEIGHT_UID, BRAIINS_POOL_UID, CLOCK_UID, FORMULA_1_UID,
    HALVING_COUNTDOWN_UID, ISS_POSITION_UID, MINING_CLOCK_UID, MINING_INFO_UID, NAMEDAY_UID,
    RANDOM_FACTS_UID, REMOTE_IMAGE_UID, SPACEX_LAUNCH_UID, TICKER_LIST_UID, TICKER_SINGLE_UID,
    WEATHER_UID,
};

#[derive(Debug)]
pub struct DefaultWidget {
    pub product: Product,
    pub widget_type_id: Uuid,
    pub params: BTreeMap<ParamKey, ParamValue>,
}

#[must_use]
pub fn default_widgets() -> Vec<DefaultWidget> {
    [
        Product::Bmc100,
        Product::Bmm100,
        Product::Bmm101,
        Product::Bfm100,
    ]
    .into_iter()
    .flat_map(|product| {
        crate::config::Config::platform_default(product)
            .scenes()
            .values()
            .flat_map(move |scene| {
                scene.widgets.values().map(move |widget| DefaultWidget {
                    product,
                    widget_type_id: widget.widget_type_id,
                    params: widget.params.clone(),
                })
            })
            .collect::<Vec<_>>()
    })
    .collect()
}

#[must_use]
pub fn widget_uids() -> [(&'static str, Uuid); 16] {
    [
        ("clock", CLOCK_UID),
        ("weather", WEATHER_UID),
        ("blockheight", BLOCK_HEIGHT_UID),
        ("halving-countdown", HALVING_COUNTDOWN_UID),
        ("mining-info", MINING_INFO_UID),
        ("mining-clock", MINING_CLOCK_UID),
        ("image", REMOTE_IMAGE_UID),
        ("iss-position", ISS_POSITION_UID),
        ("nameday", NAMEDAY_UID),
        ("random-facts", RANDOM_FACTS_UID),
        ("spacex-launch", SPACEX_LAUNCH_UID),
        ("braiins-pool", BRAIINS_POOL_UID),
        ("ticker-single", TICKER_SINGLE_UID),
        ("ticker-list", TICKER_LIST_UID),
        ("formula-1", FORMULA_1_UID),
        ("bitcoin-mining-data", BITCOIN_MINING_DATA_UID),
    ]
}

#[derive(Debug)]
pub struct MigrationManifestExpectations {
    pub clock_font: &'static str,
    pub block_height_font: &'static str,
    pub halving_countdown_font: &'static str,
    pub weather_location: &'static str,
    pub weather_time_zone: &'static str,
    pub image_refresh_seconds: i32,
    pub nameday_country: &'static str,
    pub nameday_countries: &'static [&'static str],
    pub translated_font_styles: [&'static str; 3],
    pub pool_style: &'static str,
    pub pool_chart_frame: &'static str,
    pub pool_worker_states: bool,
    pub pool_styles: [&'static str; 2],
    pub translated_pool_chart_frames: [&'static str; 4],
    pub pool_credential_slot: &'static str,
    pub ticker_pair: &'static str,
    pub ticker_period: &'static str,
    pub ticker_list_symbols: [&'static str; 8],
    pub translated_ticker_periods: [&'static str; 6],
    pub translated_btc_time_frames: [&'static str; 10],
    pub ticker_views: [&'static str; 2],
}

#[must_use]
pub fn migration_manifest_expectations() -> MigrationManifestExpectations {
    crate::config_migration::upgrade_v0::manifest_test_expectations()
}
