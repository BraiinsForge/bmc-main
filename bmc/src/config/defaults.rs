// Copyright (C) 2026  Braiins Systems s.r.o.

use std::collections::BTreeMap;

use bmc_platform::Product;
use bmc_widget_manifest::{ParamKey, ParamValue, ViewportShape};
use indexmap::{IndexMap, indexmap};
use uuid::Uuid;

use crate::scene::{
    Scene, SceneId, SceneKind, Widget, WidgetId, WidgetPlacement, WidgetPosition, WidgetSize,
};

use super::params_map;

const CLOCK: &str = "fbc867c9-b722-4bdb-8738-c15d20fe2b88";
const WEATHER: &str = "2379712a-e573-46db-8e9c-94f6ed75d92c";
const BLOCKHEIGHT: &str = "7cb584a8-1f26-42a0-867e-955aadd2391c";
const MINING_INFO: &str = "6d0c6a2d-24d0-4384-8f8b-6f4ac2c9675a";
const MINING_CLOCK: &str = "0f0b7df0-f6d5-4d21-9ddc-7755e5030503";

pub(super) fn scenes_for(product: Product) -> IndexMap<SceneId, Scene> {
    match product {
        Product::Bmc100 => bmc100_scenes(),
        Product::Bfm100 => bfm100_scenes(),
        Product::Bmm100 | Product::Bmm101 => bmm_scenes(),
    }
}

fn uid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("BUG: invalid built-in widget UID")
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

fn weather_params(location: &str) -> BTreeMap<ParamKey, ParamValue> {
    params(&[
        ("location", ParamValue::String(location.into())),
        ("time_zone", ParamValue::String("location".into())),
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
        ("currency", ParamValue::String("usd".into())),
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

    let digital = fullscreen(uid(CLOCK), rect, clock_params("digital"));

    let weather = fullscreen(uid(WEATHER), rect, weather_params("Prague"));

    let combined = {
        let clock_w = widget(
            uid(CLOCK),
            rect,
            WidgetPosition { row: 0, col: 0 },
            WidgetPlacement::from(WidgetSize::Medium),
            clock_params("analog_rect"),
        );
        let block_w = widget(
            uid(BLOCKHEIGHT),
            rect,
            WidgetPosition { row: 1, col: 0 },
            WidgetPlacement::from(WidgetSize::Medium),
            blockheight_params(),
        );
        let weather_w = widget(
            uid(WEATHER),
            rect,
            WidgetPosition { row: 0, col: 2 },
            WidgetPlacement::from(WidgetSize::Large),
            weather_params("Prague"),
        );
        Scene {
            id: SceneId::generate(),
            enabled: true,
            cycle_duration: None,
            kind: SceneKind::Combined,
            widgets: indexmap! {
                clock_w.id => clock_w,
                block_w.id => block_w,
                weather_w.id => weather_w,
            },
        }
    };

    indexmap! {
        digital.id => digital,
        weather.id => weather,
        combined.id => combined,
    }
}

fn bfm100_scenes() -> IndexMap<SceneId, Scene> {
    let round = ViewportShape::Round;

    let geek = fullscreen(uid(MINING_INFO), round, mining_info_params("geek"));
    let clock = fullscreen(uid(MINING_CLOCK), round, mining_clock_params());

    indexmap! {
        geek.id => geek,
        clock.id => clock,
    }
}

fn bmm_scenes() -> IndexMap<SceneId, Scene> {
    let rect = ViewportShape::Rectangular;

    let clock = fullscreen(uid(CLOCK), rect, clock_params("analog_rect"));
    let weather = fullscreen(uid(WEATHER), rect, weather_params("Prague"));
    let mining = fullscreen(uid(MINING_INFO), rect, mining_info_params("mining"));
    let geek = fullscreen(uid(MINING_INFO), rect, mining_info_params("geek"));
    let network = fullscreen(uid(MINING_INFO), rect, mining_info_params("network"));
    let overload = fullscreen(uid(MINING_INFO), rect, mining_info_params("info_overload"));

    indexmap! {
        clock.id => clock,
        weather.id => weather,
        mining.id => mining,
        geek.id => geek,
        network.id => network,
        overload.id => overload,
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use bmc_widget_manifest::{Manifest, ParamKind};

    use super::*;

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
    fn every_default_widget_matches_its_manifest() {
        let manifests = [
            include_str!("../../../widgets-wasm/clock/manifest.json"),
            include_str!("../../../widgets-wasm/weather/manifest.json"),
            include_str!("../../../widgets-wasm/blockheight/manifest.json"),
            include_str!("../../../widgets-wasm/mining-info/manifest.json"),
            include_str!("../../../widgets-wasm/mining-clock/manifest.json"),
        ]
        .map(|json| Manifest::from_str(json).expect("BUG: in-tree manifest must parse"));

        for product in [
            Product::Bmc100,
            Product::Bmm100,
            Product::Bmm101,
            Product::Bfm100,
        ] {
            for scene in scenes_for(product).values() {
                for widget in scene.widgets.values() {
                    let manifest = manifests
                        .iter()
                        .find(|manifest| manifest.uid == widget.widget_type_id)
                        .expect("BUG: default scene uses a widget without an in-tree manifest");
                    for (key, definition) in &manifest.params {
                        assert!(
                            definition.is_optional || widget.params.contains_key(key),
                            "{product:?} default for {} is missing required param {key:?}",
                            manifest.name
                        );
                    }
                    for (key, value) in &widget.params {
                        let definition = manifest.params.get(key).unwrap_or_else(|| {
                            panic!(
                                "{product:?} default for {} carries param {key:?} \
                                 that the manifest does not declare",
                                manifest.name
                            )
                        });
                        let kind_matches = match value {
                            ParamValue::Null => definition.is_optional,
                            ParamValue::Boolean(_) => {
                                matches!(definition.kind, ParamKind::Boolean { .. })
                            }
                            ParamValue::Integer(_) => {
                                matches!(definition.kind, ParamKind::Integer { .. })
                            }
                            ParamValue::Double(_) => {
                                matches!(definition.kind, ParamKind::Double { .. })
                            }
                            ParamValue::String(_) => matches!(
                                definition.kind,
                                ParamKind::String { .. } | ParamKind::Timezone { .. }
                            ),
                        };
                        assert!(
                            kind_matches,
                            "{product:?} default for {} sets param {key:?} to {value:?} \
                             which does not match the manifest type",
                            manifest.name
                        );
                        if let (ParamValue::String(value), ParamKind::String { enum_values, .. }) =
                            (value, &definition.kind)
                        {
                            assert!(
                                enum_values.is_empty()
                                    || enum_values.iter().any(|option| option.value == *value),
                                "{product:?} default for {} sets param {key:?} to {value:?} \
                                 which is not a declared enum value",
                                manifest.name
                            );
                        }
                    }
                }
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
