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
use bmc_widget_manifest::{ParamKey, ParamValue, ViewportShape};
use indexmap::{IndexMap, indexmap};
use uuid::Uuid;

use super::params_map;
use super::widget_uuids::{
    BITCOIN_MINING_DATA_UID, BLOCK_HEIGHT_UID, CLOCK_UID, MINING_CLOCK_UID, MINING_INFO_UID,
    TICKER_SINGLE_UID,
};
use crate::scene::{
    Scene, SceneId, SceneKind, Widget, WidgetId, WidgetPlacement, WidgetPosition, WidgetSize,
};

pub(super) fn scenes_for(product: Product) -> IndexMap<SceneId, Scene> {
    match product {
        Product::Bmc100 => bmc100_scenes(),
        Product::Bfm100 => bfm100_scenes(),
        Product::Bmm100 | Product::Bmm101 => bmm_scenes(),
    }
}

fn params(entries: &[(&str, ParamValue)]) -> BTreeMap<ParamKey, ParamValue> {
    params_map(entries).expect("BUG: invalid built-in ParamKey in default scenes")
}

fn clock_params(style: &str) -> BTreeMap<ParamKey, ParamValue> {
    params(&[
        ("clock_style", ParamValue::String(style.into())),
        ("numbers_font_style", ParamValue::String("semi-bold".into())),
        ("show_date", ParamValue::Boolean(true)),
        ("show_seconds", ParamValue::Boolean(true)),
        ("show_timezone", ParamValue::Boolean(true)),
    ])
}

fn blockheight_params() -> BTreeMap<ParamKey, ParamValue> {
    params(&[
        ("numbers_font_style", ParamValue::String("bold".into())),
        ("show_timestamp", ParamValue::Boolean(true)),
    ])
}

fn ticker_params(pair: &str, period: &str) -> BTreeMap<ParamKey, ParamValue> {
    params(&[
        ("pair", ParamValue::String(pair.into())),
        ("period", ParamValue::String(period.into())),
        ("view", ParamValue::String("sparkline".into())),
    ])
}

fn mining_info_params(view: &str) -> BTreeMap<ParamKey, ParamValue> {
    params(&[
        ("view", ParamValue::String(view.into())),
        (
            "miner_url",
            ParamValue::String("http://localhost/api/v1".into()),
        ),
        ("miner_password", ParamValue::String("root".into())),
    ])
}

fn mining_clock_params() -> BTreeMap<ParamKey, ParamValue> {
    params(&[
        (
            "miner_url",
            ParamValue::String("http://localhost/api/v1".into()),
        ),
        ("miner_password", ParamValue::String("root".into())),
        ("numbers_font_style", ParamValue::String("semi-bold".into())),
        ("show_date", ParamValue::Boolean(true)),
        ("show_seconds", ParamValue::Boolean(true)),
        ("show_timezone", ParamValue::Boolean(true)),
    ])
}

fn widget(
    type_uid: Uuid,
    shape: ViewportShape,
    position: WidgetPosition,
    placement: WidgetPlacement,
    params: BTreeMap<ParamKey, ParamValue>,
) -> Widget {
    Widget {
        id: WidgetId::generate(),
        position,
        placement,
        widget_type_id: type_uid,
        viewport_shape: shape,
        params,
        credential_bindings: BTreeMap::new(),
    }
}

fn fullscreen(
    type_uid: Uuid,
    shape: ViewportShape,
    params: BTreeMap<ParamKey, ParamValue>,
) -> Scene {
    let widget = widget(
        type_uid,
        shape,
        WidgetPosition { row: 0, col: 0 },
        WidgetPlacement::Fullscreen,
        params,
    );
    Scene {
        id: SceneId::generate(),
        enabled: true,
        cycle_duration: None,
        kind: SceneKind::Fullscreen,
        widgets: indexmap! { widget.id => widget },
    }
}

fn bmc100_scenes() -> IndexMap<SceneId, Scene> {
    let rect = ViewportShape::Rectangular;

    let digital = fullscreen(CLOCK_UID, rect, clock_params("digital"));

    let ticker = fullscreen(TICKER_SINGLE_UID, rect, ticker_params("BTC-USD", "7d"));

    let combined = {
        let clock_w = widget(
            CLOCK_UID,
            rect,
            WidgetPosition { row: 0, col: 0 },
            WidgetPlacement::from(WidgetSize::Medium),
            clock_params("analog_rect"),
        );
        let block_w = widget(
            BLOCK_HEIGHT_UID,
            rect,
            WidgetPosition { row: 1, col: 0 },
            WidgetPlacement::from(WidgetSize::Medium),
            blockheight_params(),
        );
        let ticker_w = widget(
            TICKER_SINGLE_UID,
            rect,
            WidgetPosition { row: 0, col: 2 },
            WidgetPlacement::from(WidgetSize::Large),
            ticker_params("BTC-USD", "1d"),
        );
        Scene {
            id: SceneId::generate(),
            enabled: true,
            cycle_duration: None,
            kind: SceneKind::Combined,
            widgets: indexmap! {
                clock_w.id => clock_w,
                block_w.id => block_w,
                ticker_w.id => ticker_w,
            },
        }
    };

    indexmap! {
        digital.id => digital,
        ticker.id => ticker,
        combined.id => combined,
    }
}

fn bfm100_scenes() -> IndexMap<SceneId, Scene> {
    let round = ViewportShape::Round;

    let geek = fullscreen(MINING_INFO_UID, round, mining_info_params("geek"));
    let clock = fullscreen(MINING_CLOCK_UID, round, mining_clock_params());

    indexmap! {
        geek.id => geek,
        clock.id => clock,
    }
}

fn bmm_scenes() -> IndexMap<SceneId, Scene> {
    let rect = ViewportShape::Rectangular;

    let clock = fullscreen(CLOCK_UID, rect, clock_params("analog_rect"));
    let ticker = fullscreen(TICKER_SINGLE_UID, rect, ticker_params("BTC-USD", "7d"));
    let mining = fullscreen(MINING_INFO_UID, rect, mining_info_params("mining"));
    let geek = fullscreen(MINING_INFO_UID, rect, mining_info_params("geek"));
    let overload = fullscreen(MINING_INFO_UID, rect, mining_info_params("info_overload"));
    // Takes the slot the Miner Info network view vacated: Bitcoin-network data
    // still ships out of the box, from the widget that owns it. It declares no
    // params.
    let bitcoin = fullscreen(BITCOIN_MINING_DATA_UID, rect, BTreeMap::new());

    indexmap! {
        clock.id => clock,
        ticker.id => ticker,
        mining.id => mining,
        geek.id => geek,
        overload.id => overload,
        bitcoin.id => bitcoin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::widget_uuids::WEATHER_UID;

    #[test]
    fn bmc100_has_three_scenes_with_one_combined() {
        let scenes = scenes_for(Product::Bmc100);
        assert_eq!(scenes.len(), 3);
        let combined = scenes
            .values()
            .filter(|scene| scene.kind == SceneKind::Combined)
            .count();
        assert_eq!(combined, 1);
    }

    #[test]
    fn bfm100_scenes_are_round_fullscreen() {
        let scenes = scenes_for(Product::Bfm100);
        assert_eq!(scenes.len(), 2);
        for scene in scenes.values() {
            assert_eq!(scene.kind, SceneKind::Fullscreen);
            for widget in scene.widgets.values() {
                assert_eq!(widget.viewport_shape, ViewportShape::Round);
            }
        }
    }

    #[test]
    fn bmm_platforms_have_six_fullscreen_scenes() {
        for product in [Product::Bmm100, Product::Bmm101] {
            let scenes = scenes_for(product);
            assert_eq!(scenes.len(), 6);
            assert!(
                scenes
                    .values()
                    .all(|scene| scene.kind == SceneKind::Fullscreen)
            );
        }
    }

    #[test]
    fn rectangular_defaults_replace_weather_with_btc_tickers() {
        for (product, expected_ticker_count) in [
            (Product::Bmc100, 2),
            (Product::Bmm100, 1),
            (Product::Bmm101, 1),
        ] {
            let widgets = scenes_for(product)
                .into_values()
                .flat_map(|scene| scene.widgets.into_values())
                .collect::<Vec<_>>();
            assert!(
                widgets
                    .iter()
                    .all(|widget| widget.widget_type_id != WEATHER_UID),
                "{product:?} defaults must not contain the weather widget"
            );
            let tickers = widgets
                .iter()
                .filter(|widget| widget.widget_type_id == TICKER_SINGLE_UID)
                .collect::<Vec<_>>();
            assert_eq!(
                tickers.len(),
                expected_ticker_count,
                "{product:?} defaults must contain the expected ticker placements"
            );
            for ticker in tickers {
                let period = if ticker.placement == WidgetPlacement::Fullscreen {
                    "7d"
                } else {
                    "1d"
                };
                assert_eq!(
                    ticker.params,
                    ticker_params("BTC-USD", period),
                    "{product:?} ticker must use BTC with the period for its placement"
                );
            }
        }
    }

    #[test]
    fn every_platform_default_validates() {
        for product in [
            Product::Bmc100,
            Product::Bmm100,
            Product::Bmm101,
            Product::Bfm100,
        ] {
            crate::config::Config::platform_default(product)
                .validate()
                .expect("BUG: platform default must validate");
        }
    }
}
