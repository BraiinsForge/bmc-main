// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::bitcoin_data::BitcoinData;
use crate::blockheight_data::BlockheightData;
use crate::btc_history_data::BtcHistoryData;
use crate::clock_data::ClockData;
use crate::data::{
    ConnectInfoScreen, InitScreen, Scene, SceneCycling, SceneCyclingTransition, SceneId,
    SignalStrength, UpgradeScreen, Widget, WidgetId,
};
use crate::diff_hashrate_data::DiffHashrateData;
use crate::difficulty_data::DifficultyData;
use crate::display_controller::DisplayController;
use crate::generated::{
    self, AlarmAdapter, BaseDimensions, BitcoinAdapter, BlockHeightAdapter,
    BlockchainDataChartDimensions, BraiinsPoolStyle, ClockStyle, ConnectionAdapter,
    DifficultyAdapter, HashrateAdapter, PoolChartDimensions, SceneCyclingAdapter, ScreenAdapter,
    WidgetSize, WifiAdapter,
};
use crate::hashrate_data::HashrateData;
use crate::indexmap_model::IndexMapModel;
use crate::pool_data::{
    CurrentUserHashrate, CurrentUserWorkerStats, LatestUserRewards, RecentUserPayouts,
    UserFinancials, UserHashrateHistory, UserWorkerHistory,
};
use crate::remote_image_data::RemoteImageState;
use crate::{SharedImageBuffer, utils};
use bmc_shared_time::time::{DateFormat, Timezone};
use bmc_shared_utils::number_format::NumberFormat;
use chrono::{Datelike, Timelike, Utc};
use indexmap::IndexMap;
use slint::{FilterModel, Global, Model, ModelRc, SharedString, VecModel};
use std::any::type_name;
use std::net::IpAddr;
use std::time::Duration;

impl DisplayController {
    pub fn set_scenes(&self, scenes: IndexMap<SceneId, Scene>) {
        self.in_event_loop(move |main_window| {
            let scenes = ModelRc::new(
                scenes
                    .into_iter()
                    .map(|(id, scene)| (id, generated::Scene::from(scene)))
                    .collect::<IndexMapModel<_, _>>(),
            );

            let cycler_scenes =
                ModelRc::new(FilterModel::new(scenes.clone(), |scene| scene.enabled));

            main_window.set_scenes(scenes);
            main_window.set_preview_scene_index(-1);
            main_window.set_cycler_scenes(cycler_scenes);
            main_window.set_cycler_scene_index(0);
        });
    }

    pub fn reset_cycler(&self) {
        self.in_event_loop(|main_window| {
            main_window.set_cycler_scene_index(0);
        });
    }

    pub fn set_scene_cycling(&self, scene_cycling: SceneCycling) {
        self.in_event_loop(move |main_window| {
            let adapter = SceneCyclingAdapter::get(&main_window);
            adapter.set_automatic_cycling_enabled(scene_cycling.automatic_cycling_enabled);

            #[expect(clippy::cast_possible_truncation)]
            let automatic_cycling_default_duration =
                scene_cycling.automatic_cycling_default_duration.as_millis() as i64;
            adapter.set_automatic_cycling_default_duration(automatic_cycling_default_duration);

            adapter.set_transition(match scene_cycling.transition {
                SceneCyclingTransition::Slide => generated::SceneCyclingTransition::Slide,
                SceneCyclingTransition::Fade => generated::SceneCyclingTransition::Fade,
            });
        });
    }

    pub fn set_night_mode(&self, enabled: bool) {
        self.in_event_loop(move |main_window| {
            main_window.set_night_mode_enabled(enabled);
        });
    }

    pub fn set_preview_scene(&self, scene_id: Option<SceneId>) {
        self.in_event_loop(move |main_window| {
            if let Some(scene_id) = scene_id {
                let scenes_ref = main_window.get_scenes();
                let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

                let index = scenes_ref.get_index_of(&scene_id);
                debug_assert!(index.is_some());

                if let Some(index) = index {
                    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    main_window.set_preview_scene_index(index as i32);
                }
            } else {
                main_window.set_preview_scene_index(-1);
            }
        });
    }

    pub fn add_scene(&self, scene: Scene) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            let replaced_scene = scenes_ref.insert(scene.id.clone(), scene.into());
            debug_assert!(replaced_scene.is_none());
        });
    }

    pub fn insert_scene(&self, index: usize, scene: Scene) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            let replaced_scene = scenes_ref.shift_insert(index, scene.id.clone(), scene.into());
            debug_assert!(replaced_scene.is_none());
        });
    }

    pub fn update_scene(&self, id: SceneId, enabled: bool, cycle_duration: Option<Duration>) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            scenes_ref.modify(&id, |scene| {
                scene.enabled = enabled;

                // NOTE: value -1 is used as sentinel value to signal that we should use default
                // value from SceneCyclingAdapter
                scene.cycle_duration = cycle_duration.map_or(-1, |cycle_duration| {
                    #[expect(clippy::cast_possible_truncation)]
                    let cycle_duration = cycle_duration.as_millis() as i64;
                    cycle_duration
                });
            });
        });
    }

    pub fn move_scene(&self, from_index: usize, to_index: usize) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            scenes_ref.move_index(from_index, to_index);
        });
    }

    pub fn remove_scene(&self, id: SceneId) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            let removed_scene = scenes_ref.shift_remove(&id);
            debug_assert!(removed_scene.is_some());
        });
    }

    pub fn add_scene_widget(&self, scene_id: SceneId, widget: Widget) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                let replaced_widget = widgets_ref.insert(widget.id.clone(), widget.into());
                debug_assert!(replaced_widget.is_none());
            }
        });
    }

    pub fn replace_scene_widget(&self, scene_id: SceneId, widget: Widget) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                let replaced_widget = widgets_ref.insert(widget.id.clone(), widget.into());
                debug_assert!(replaced_widget.is_some());
            }
        });
    }

    pub fn remove_scene_widget(&self, scene_id: SceneId, widget_id: WidgetId) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                let removed_widget = widgets_ref.shift_remove(&widget_id);
                debug_assert!(removed_widget.is_some());
            }
        });
    }

    pub fn update_clock_widget(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        datetime: chrono::DateTime<chrono::FixedOffset>,
        timezone: String,
        is_24_format: bool,
        clock_data: ClockData,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                let new_datetime = to_datetime(datetime, timezone, is_24_format);

                // NOTE: `modify` always marks row as changed, even if we make no changes inside the closure
                let datetime_changed = widgets_ref
                    .get(&widget_id)
                    .is_some_and(|widget| widget.clock.datetime != new_datetime);

                if !datetime_changed {
                    return;
                }

                widgets_ref.modify(&widget_id, |widget| {
                    let config = &widget.clock.config;
                    let datetime = &mut widget.clock.datetime;
                    let analog_hands = &mut widget.clock.analog_clock_hands;

                    // NOTE: this is optimization for analog hands, because we need to avoid image
                    // re-creation each time
                    if datetime.minute != new_datetime.minute {
                        match config.clock_style {
                            ClockStyle::AnalogRound => {
                                analog_hands.hour_hand_round = clock_data.hour_hand_round();
                                analog_hands.minute_hand_round = clock_data.minute_hand_round();
                            }
                            ClockStyle::AnalogRect => {
                                analog_hands.minute_hand_rect = clock_data.minute_hand_rect();
                                analog_hands.hour_hand_rect = clock_data.hour_hand_rect();
                            }
                            ClockStyle::Digital => {}
                        }
                    }

                    if datetime.second != new_datetime.second {
                        match config.clock_style {
                            ClockStyle::AnalogRound => {
                                analog_hands.second_hand_round = clock_data.second_hand_round();
                            }
                            ClockStyle::AnalogRect => {
                                analog_hands.second_hand_rect = clock_data.second_hand_rect();
                            }
                            ClockStyle::Digital => {}
                        }
                    }

                    *datetime = new_datetime;
                });
            }
        });
    }

    pub fn update_btc_price(&self, btc_data: BitcoinData, number_format: NumberFormat) {
        self.in_event_loop(move |main_window| {
            let bitcoin_adapter = BitcoinAdapter::get(&main_window);
            bitcoin_adapter.set_price(btc_data.price_as_shared(number_format));
            bitcoin_adapter.set_price_change(btc_data.price_change_as_shared(number_format));
            bitcoin_adapter.set_price_increase(btc_data.increasing_trend());
        });
    }

    pub fn update_ticker_btc(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        btc_history_data: BtcHistoryData,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let widget_size = widget.size;
                    let base_dimensions = BaseDimensions::get(&main_window);
                    let width: u32 = base_dimensions
                        .invoke_widget_width_int(widget_size)
                        .try_into()
                        .unwrap_or_default();
                    let height: u32 = base_dimensions
                        .invoke_widget_height_int(widget_size)
                        .try_into()
                        .unwrap_or_default();
                    widget.ticker_btc.btc_graph =
                        btc_history_data.graph_image(&main_window, width, height);
                });
            }
        });
    }

    pub fn update_blockheight_data(
        &self,
        blockheight_data: BlockheightData,
        timezone: Timezone,
        is_24_format: bool,
        date_format: DateFormat,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let blockheight_adapter = BlockHeightAdapter::get(&main_window);
            blockheight_adapter.set_block_height(
                blockheight_data
                    .clone()
                    .blockheight_as_shared(number_format),
            );
            blockheight_adapter.set_timestamp(blockheight_data.timestamp_as_shared(
                &timezone,
                is_24_format,
                date_format,
            ));
        });
    }

    pub fn update_difficulty_data(
        &self,
        difficulty_data: DifficultyData,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let difficulty_adapter = DifficultyAdapter::get(&main_window);
            difficulty_adapter.set_difficulty(difficulty_data.difficulty_as_shared(number_format));
            difficulty_adapter.set_blocks_epoch(difficulty_data.block_epoch(number_format));
            difficulty_adapter.set_epoch_block_time(difficulty_data.epoch_block_time());
            difficulty_adapter.set_prev_adjustment_increase(difficulty_data.prev_adjust_increase());
            difficulty_adapter
                .set_prev_adjustment_change(difficulty_data.prev_adjust_as_shared(number_format));
            difficulty_adapter.set_prev_adjustment_time(difficulty_data.prev_adjust_time());
            difficulty_adapter.set_next_adjustment_increase(difficulty_data.next_adjust_increase());
            difficulty_adapter
                .set_next_adjustment_change(difficulty_data.next_adjust_as_shared(number_format));
            difficulty_adapter.set_next_adjustment_time(difficulty_data.next_adjust_time());
        });
    }

    pub fn update_hashrate_data(&self, hashrate_data: HashrateData, number_format: NumberFormat) {
        self.in_event_loop(move |main_window| {
            let hashrate_adapter = HashrateAdapter::get(&main_window);
            hashrate_adapter
                .set_avg_fees_per_block(hashrate_data.avg_fees_per_block(number_format));
            hashrate_adapter.set_fees_percent(hashrate_data.fees_percent(number_format));
            hashrate_adapter.set_current_hashrate(hashrate_data.current_hashrate(number_format));
            hashrate_adapter.set_hashprice(hashrate_data.hashprice(number_format));
            hashrate_adapter.set_total_revenue(hashrate_data.total_revenue(number_format));
        });
    }

    pub fn update_blockchain_btc_graph(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        btc_history_data: BtcHistoryData,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let widget_size = widget.size;
                    let chart_dimensions = BlockchainDataChartDimensions::get(&main_window);
                    let image_dimensions = chart_dimensions.invoke_get_dimensions(widget_size);
                    let width: u32 = image_dimensions.width.try_into().unwrap_or_default();
                    let height: u32 = image_dimensions.height.try_into().unwrap_or_default();

                    widget.blockchain_data.btc_price_graph =
                        btc_history_data.graph_small_image(&main_window, width, height, true);
                });
            }
        });
    }

    pub fn update_hashrate_info(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        hashrate_data: DiffHashrateData,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let widget_size = widget.size;
                    let chart_dimensions = BlockchainDataChartDimensions::get(&main_window);
                    let image_dimensions = chart_dimensions.invoke_get_dimensions(widget_size);
                    let width: u32 = image_dimensions.width.try_into().unwrap_or_default();
                    let height: u32 = image_dimensions.height.try_into().unwrap_or_default();

                    widget.blockchain_data.hashrate_graph =
                        hashrate_data.graph_hashrate_image(&main_window, width, height, true);
                    widget.blockchain_data.hashrate_trend_increase =
                        hashrate_data.hashrate_increasing_trend();
                    widget.blockchain_data.hashrate_trend_change =
                        hashrate_data.hashrate_change_trend(number_format);
                });
            }
        });
    }

    pub fn update_difficulty_graph(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        difficulty_data: DiffHashrateData,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let widget_size = widget.size;
                    let chart_dimensions = BlockchainDataChartDimensions::get(&main_window);
                    let image_dimensions = chart_dimensions.invoke_get_dimensions(widget_size);
                    let width: u32 = image_dimensions.width.try_into().unwrap_or_default();
                    let height: u32 = image_dimensions.height.try_into().unwrap_or_default();

                    widget.blockchain_data.difficulty_graph =
                        difficulty_data.graph_dificulty_image(&main_window, width, height, true);
                });
            }
        });
    }

    pub fn update_blocks_last_24h(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        blocks: Vec<BlockheightData>,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let now = Utc::now();
                    let block_count = blocks
                        .into_iter()
                        .filter(|block| {
                            block
                                .timestamp_as_datetime()
                                .is_some_and(|timestamp| (now - timestamp).num_hours() <= 24)
                        })
                        .count();
                    widget.blockchain_data.blocks_24h =
                        SharedString::from(format!("{block_count}/144"));
                });
            }
        });
    }

    pub fn update_account_name(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        account_name: String,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.braiins_pool.account_name = SharedString::from(account_name);
                });
            }
        });
    }

    pub fn update_current_user_hashrate(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        current_user_hashrate: CurrentUserHashrate,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.braiins_pool.current_hashrate =
                        current_user_hashrate.hashrate_as_shared(number_format);
                    widget.braiins_pool.current_hashrate_unit =
                        current_user_hashrate.hashrate_units();
                });
            }
        });
    }

    pub fn update_rewards_latest(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        latest_rewards: LatestUserRewards,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.braiins_pool.reward_btc = latest_rewards.today_reward_btc(number_format);
                    widget.braiins_pool.reward_usd = latest_rewards.today_reward_usd(number_format);
                });
            }
        });
    }

    #[expect(clippy::too_many_arguments)]
    pub fn update_hashrate_history(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        system_timezone: Timezone,
        is_24_format: bool,
        date_format: DateFormat,
        hashrate_history: UserHashrateHistory,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let widget_size = widget.size;
                    let pool_style = widget.braiins_pool.config.pool_style;
                    let pool_chart_dimensions = PoolChartDimensions::get(&main_window);
                    let image_dimensions =
                        pool_chart_dimensions.invoke_get_dimensions(widget_size, pool_style);
                    let width: u32 = image_dimensions.width.try_into().unwrap_or_default();
                    let height: u32 = image_dimensions.height.try_into().unwrap_or_default();

                    widget.braiins_pool.hashrate_units =
                        hashrate_history.graph_units(number_format);

                    match (pool_style, widget_size) {
                        (BraiinsPoolStyle::Overview, WidgetSize::Large) => {
                            widget.braiins_pool.chart_overview_large = hashrate_history
                                .into_graph_image(&main_window, width, height, false);
                        }
                        (BraiinsPoolStyle::Overview, WidgetSize::Full) => {
                            widget.braiins_pool.chart_overview_full_tmp = hashrate_history
                                .into_graph_image(&main_window, width, height, true);
                        }
                        // Other Overview sizes do not have graphs
                        (BraiinsPoolStyle::Overview, _) => {}
                        (BraiinsPoolStyle::BigChart, WidgetSize::Small) => {
                            widget.braiins_pool.chart_bigchart_small = hashrate_history
                                .into_graph_image(&main_window, width, height, false);
                        }
                        (BraiinsPoolStyle::BigChart, WidgetSize::Medium) => {
                            widget.braiins_pool.chart_bigchart_medium_tmp = hashrate_history
                                .into_graph_image(&main_window, width, height, true);
                        }
                        (BraiinsPoolStyle::BigChart, WidgetSize::Large) => {
                            widget.braiins_pool.chart_bigchart_large_tmp = hashrate_history
                                .into_graph_image(&main_window, width, height, true);
                        }
                        (BraiinsPoolStyle::BigChart, WidgetSize::Full) => {
                            widget.braiins_pool.chart_bigchart_full_tmp = hashrate_history
                                .into_graph_image(&main_window, width, height, true);
                            widget.braiins_pool.hashrate_unit_label =
                                hashrate_history.hashrate_units();
                            widget.braiins_pool.timestamps = hashrate_history.timestamps(
                                &system_timezone,
                                is_24_format,
                                date_format,
                            );
                        }
                    }
                });
            }
        });
    }

    pub fn update_current_workers(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        workers_stats: CurrentUserWorkerStats,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.braiins_pool.worker_status = workers_stats.worker_stats(number_format);
                });
            }
        });
    }

    pub fn update_worker_history(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        worker_history: UserWorkerHistory,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    let widget_size = widget.size;
                    let pool_style = widget.braiins_pool.config.pool_style;
                    let pool_chart_dimensions = PoolChartDimensions::get(&main_window);
                    let image_dimensions =
                        pool_chart_dimensions.invoke_get_dimensions(widget_size, pool_style);
                    let width: u32 = image_dimensions.width.try_into().unwrap_or_default();
                    let height: u32 = image_dimensions.height.try_into().unwrap_or_default();

                    widget.braiins_pool.workers_units = worker_history.graph_units(number_format);

                    match (pool_style, widget_size) {
                        (BraiinsPoolStyle::Overview, WidgetSize::Full) => {
                            let original_image = &widget.braiins_pool.chart_overview_full_tmp;
                            widget.braiins_pool.chart_overview_full = worker_history
                                .into_graph_image(
                                    &main_window,
                                    width,
                                    height,
                                    true,
                                    original_image,
                                );
                        }
                        (BraiinsPoolStyle::BigChart, WidgetSize::Medium) => {
                            let original_image = &widget.braiins_pool.chart_bigchart_medium_tmp;
                            widget.braiins_pool.chart_bigchart_medium = worker_history
                                .into_graph_image(
                                    &main_window,
                                    width,
                                    height,
                                    true,
                                    original_image,
                                );
                        }
                        (BraiinsPoolStyle::BigChart, WidgetSize::Large) => {
                            let original_image = &widget.braiins_pool.chart_bigchart_large_tmp;
                            widget.braiins_pool.chart_bigchart_large = worker_history
                                .into_graph_image(
                                    &main_window,
                                    width,
                                    height,
                                    true,
                                    original_image,
                                );
                        }
                        (BraiinsPoolStyle::BigChart, WidgetSize::Full) => {
                            let original_image = &widget.braiins_pool.chart_bigchart_full_tmp;
                            widget.braiins_pool.chart_bigchart_full = worker_history
                                .into_graph_image(
                                    &main_window,
                                    width,
                                    height,
                                    true,
                                    original_image,
                                );
                        }
                        // Other Overview sizes do not have graphs
                        // Big Chart Small widget does not display workers history graph
                        (BraiinsPoolStyle::BigChart, WidgetSize::Small)
                        | (BraiinsPoolStyle::Overview, _) => {}
                    }
                });
            }
        });
    }

    pub fn update_payout_stats(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        user_financials: UserFinancials,
        recent_payouts: RecentUserPayouts,
        number_format: NumberFormat,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.braiins_pool.next_payout_estimate =
                        user_financials.next_payout_estimate_to_shared();
                    widget.braiins_pool.last_payout =
                        recent_payouts.last_payout_to_shared(number_format);
                    if let (Some(next_payout_estimate), Some(last_payout)) = (
                        user_financials.next_payout_estimate(),
                        recent_payouts.last_payout_datetime(),
                    ) {
                        let now = Utc::now();
                        let base = (next_payout_estimate - last_payout).abs().num_seconds();
                        let until_now = (now - last_payout).abs().num_seconds();
                        #[expect(clippy::integer_division, clippy::cast_precision_loss)]
                        let fraction = (100 * until_now / base) as f32;
                        widget.braiins_pool.progress = fraction;
                    }
                });
            }
        });
    }

    pub fn update_recent_payouts(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        recent_payouts: RecentUserPayouts,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.braiins_pool.pool_payouts = recent_payouts.payouts();
                });
            }
        });
    }

    pub fn update_remote_image(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        state: RemoteImageState,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                // NOTE: we want to keep previous image displayed on error
                widgets_ref.modify(&widget_id, |widget| match state {
                    RemoteImageState::Initial => {
                        widget.remote_image.state = generated::WidgetRemoteImageState::Initial;
                        widget.remote_image.image = slint::Image::default();
                    }
                    RemoteImageState::ConfigurationError => {
                        widget.remote_image.state =
                            generated::WidgetRemoteImageState::ConfigurationError;
                    }
                    RemoteImageState::Loading => {
                        widget.remote_image.state = generated::WidgetRemoteImageState::Loading;
                    }
                    RemoteImageState::LoadingSuccess(buffer) => {
                        widget.remote_image.state =
                            generated::WidgetRemoteImageState::LoadingSuccess;

                        widget.remote_image.image = match buffer {
                            SharedImageBuffer::RGB8(buffer) => slint::Image::from_rgb8(buffer),
                            SharedImageBuffer::RGBA8(buffer) => slint::Image::from_rgba8(buffer),
                            SharedImageBuffer::RGBA8Premultiplied(buffer) => {
                                slint::Image::from_rgba8_premultiplied(buffer)
                            }
                        };
                    }
                    RemoteImageState::LoadingError(_) => {
                        widget.remote_image.state = generated::WidgetRemoteImageState::LoadingError;
                    }
                    RemoteImageState::UnexpectedError => {
                        widget.remote_image.state =
                            generated::WidgetRemoteImageState::UnexpectedError;
                    }
                });
            }
        });
    }

    pub fn update_download_firmware_progress(&self, downloaded_mb: f32, total_mb: f32) {
        fn round_to_one_decimal(value: f32) -> f32 {
            (value * 10.0).round() / 10.0
        }

        self.in_event_loop(move |main_window| {
            let mut progress = 0.0;
            if total_mb > 0.0 {
                progress = downloaded_mb / total_mb;
            }

            let upgrade_download_adapter = generated::UpgradeDownloadAdapter::get(&main_window);

            upgrade_download_adapter.set_progress(progress);
            upgrade_download_adapter.set_downloaded_mb_text(slint::SharedString::from(format!(
                "{} MB of {} MB",
                round_to_one_decimal(downloaded_mb),
                round_to_one_decimal(total_mb)
            )));
            upgrade_download_adapter.set_progress_text(slint::SharedString::from(format!(
                "Downloading firmware {}%...",
                (progress * 100.0).round(),
            )));
        });
    }

    pub fn set_scene_cycler_screen(&self, enabled: bool) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let adapter = ScreenAdapter::get(&main_window);
            adapter.set_scene_cycler(enabled);
        });
    }

    pub fn set_clock_alarm_screen(&self, enabled: bool) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let adapter = ScreenAdapter::get(&main_window);
            adapter.set_clock_alarm(enabled);
        });
    }

    pub fn set_init_screen(&self, screen: Option<InitScreen>) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let adapter = ScreenAdapter::get(&main_window);
            adapter.set_init(screen.into());
        });
    }

    pub fn set_connect_info_screen(&self, screen: Option<ConnectInfoScreen>) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let adapter = ScreenAdapter::get(&main_window);
            adapter.set_connect_info(screen.into());
        });
    }

    pub fn set_upgrade_screen(&self, screen: Option<UpgradeScreen>) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let adapter = ScreenAdapter::get(&main_window);
            adapter.set_upgrade(screen.into());
        });
    }

    pub fn set_wifi_ssid(&self, wifi_ssid: String) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let wifi_adapter = WifiAdapter::get(&main_window);
            wifi_adapter.set_ssid(wifi_ssid.into());
        });
    }

    pub fn set_wifi_signal_strength(&self, signal_strength: SignalStrength) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let wifi_adapter = WifiAdapter::get(&main_window);
            wifi_adapter.set_signal_strength(signal_strength.into());
        });
    }

    pub fn set_connect_ip_qr_code(&self, ip: Option<IpAddr>) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let connection_adapter = ConnectionAdapter::get(&main_window);

            connection_adapter.set_ip_qr_code(utils::ip_as_qrcode(ip));

            if let Some(ip_address) = ip {
                connection_adapter.set_ip(ip_address.to_string().into());
            }
        });
    }

    pub fn set_alarm_data(&self, label: String, show_snooze: bool) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let alarm_adapter = AlarmAdapter::get(&main_window);

            alarm_adapter.set_label(slint::SharedString::from(label));
            alarm_adapter.set_snooze_visible(show_snooze);
        });
    }

    pub fn set_brightness(&self, brightness_pct: u8) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let brightness_adapter = generated::BrightnessAdapter::get(&main_window);
            brightness_adapter.set_brightness(i32::from(brightness_pct));
        });
    }

    pub fn set_next_alarm(&self, maybe_next_alarm: Option<chrono::DateTime<chrono::FixedOffset>>) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let alarm_adapter = AlarmAdapter::get(&main_window);

            alarm_adapter.set_next_alarm_is_defined(maybe_next_alarm.is_some());

            if let Some(datetime) = maybe_next_alarm {
                let system_datetime = main_window.get_system_datetime();
                alarm_adapter.set_next_alarm_time(to_datetime(
                    datetime,
                    system_datetime.timezone.to_string(),
                    system_datetime.is_24_format,
                ));
            }
        });
    }

    pub fn update_system_datetime(
        &self,
        datetime: chrono::DateTime<chrono::FixedOffset>,
        timezone: String,
        is_24_format: bool,
    ) {
        self.in_event_loop(move |main_window| {
            main_window.set_system_datetime(to_datetime(datetime, timezone, is_24_format));
        });
    }

    pub fn set_is_wifi_offline(&self, is_offline: bool) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let adapter = generated::StatusOverlayAdapter::get(&main_window);
            adapter.set_is_wifi_offline(is_offline);
        });
    }
}

#[allow(unused, clippy::allow_attributes)]
#[track_caller]
/// Use this function to cast ModelRc<T> into VecModel<T> when you want to manipulate items in the VecModel
fn vec_model_ref<T: 'static>(model_rc: &ModelRc<T>) -> &VecModel<T> {
    let expect_message = format!("BUG: failed to downcast VecModel<{}>", type_name::<T>());
    model_rc
        .as_any()
        .downcast_ref::<VecModel<T>>()
        .expect(&expect_message)
}

#[allow(unused, clippy::allow_attributes)]
#[track_caller]
/// Use this function to cast ModelRc<V> into IndexMapModel<K, V> when you want to manipulate items in the IndexMapModel
fn indexmap_model_ref<K: 'static, V: 'static>(model_rc: &ModelRc<V>) -> &IndexMapModel<K, V> {
    let expect_message = format!(
        "BUG: failed to downcast IndexMapModel<{}, {}>",
        type_name::<K>(),
        type_name::<V>()
    );

    model_rc
        .as_any()
        .downcast_ref::<IndexMapModel<K, V>>()
        .expect(&expect_message)
}

fn to_datetime(
    datetime: chrono::DateTime<chrono::FixedOffset>,
    timezone: String,
    is_24_format: bool,
) -> generated::DateTime {
    let hour24 = i32::try_from(datetime.hour()).unwrap_or_default();
    let hour12 = i32::try_from(datetime.hour12().1).unwrap_or_default();
    let is_pm = datetime.hour12().0;
    let minute = i32::try_from(datetime.minute()).unwrap_or_default();
    let second = i32::try_from(datetime.second()).unwrap_or_default();
    let day = i32::try_from(datetime.day()).unwrap_or_default();
    let month = i32::try_from(datetime.month()).unwrap_or_default();
    let year = datetime.year();
    let weekday = slint::format!("{}", datetime.weekday());
    let time_sec_24 = slint::format!("{hour24:02}:{minute:02}:{second:02}");
    let time_sec_12 = slint::format!("{hour12:02}:{minute:02}:{second:02}");
    let time_24 = slint::format!("{hour24:02}:{minute:02}");
    let time_12 = slint::format!("{hour12:02}:{minute:02}");
    let date = slint::format!("{day:02}. {month:02}. {year}");

    generated::DateTime {
        is_24_format,
        hour24,
        hour12,
        is_pm,
        minute,
        second,
        day,
        month,
        year,
        weekday,
        time_sec_24,
        time_sec_12,
        time_12,
        time_24,
        date,
        timezone: timezone.into(),
    }
}
