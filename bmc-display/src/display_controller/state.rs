// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::{Scene, SceneCycling, SceneCyclingTransition, SceneId, Screen, Widget, WidgetId};
use crate::display_controller::DisplayController;
use crate::generated::{self, ConnectionAdapter, InitSetupWifiAdapter, SceneCyclingAdapter};
use crate::indexmap_model::IndexMapModel;
use crate::utils;
use chrono::{Datelike, Timelike};
use indexmap::IndexMap;
use slint::{FilterModel, Global, Model, ModelRc, VecModel};
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

    pub fn update_clock_widget(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        datetime: chrono::DateTime<chrono::FixedOffset>,
        timezone: String,
        is_24_format: bool,
    ) {
        self.in_event_loop(move |main_window| {
            let scenes_ref = main_window.get_scenes();
            let scenes_ref = indexmap_model_ref::<SceneId, _>(&scenes_ref);

            if let Some(scene) = scenes_ref.get(&scene_id) {
                let widgets_ref = indexmap_model_ref::<WidgetId, _>(&scene.widgets);

                widgets_ref.modify(&widget_id, |widget| {
                    widget.clock.datetime = to_datetime(datetime, timezone, is_24_format);
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

    pub fn set_screen(&self, screen: Screen) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            main_window.set_screen_id(screen.into());
        });
    }

    pub fn set_wifi_ssid(&self, wifi_ssid: String) {
        self.in_event_loop(move |main_window: generated::MainWindow| {
            let init_setup_wifi_adapter = InitSetupWifiAdapter::get(&main_window);
            init_setup_wifi_adapter.set_ssid(wifi_ssid.into());
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
