// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_ipc::{AppMessage, Localization, Settings, SizeInfo, SizeType};
use bmc_shared_time::time::Timezone;
use tracing::{info, warn};
use uuid::Uuid;

use crate::config::LocalizationConfig;
use crate::scene::{Scene, Widget, WidgetSize};

use super::WidgetManager;

#[derive(Debug)]
pub struct Coordinator {
    widget_manager: WidgetManager,
}

impl Coordinator {
    pub fn new(widget_manager: WidgetManager) -> Self {
        Self { widget_manager }
    }

    pub async fn spawn_initial_widgets(
        &self,
        scenes: &indexmap::IndexMap<crate::scene::SceneId, Scene>,
        localization: &LocalizationConfig,
        timezone: &Timezone,
        night_mode_active: bool,
    ) {
        let enabled_scenes: Vec<_> = scenes.values().filter(|s| s.enabled).collect();
        info!(
            count = enabled_scenes.len(),
            "spawning widgets for enabled scenes"
        );

        for scene in enabled_scenes {
            self.spawn_scene_widgets(scene, localization, timezone, night_mode_active)
                .await;
        }

        info!("all scene widgets spawned");
    }

    pub async fn spawn_scene_widgets(
        &self,
        scene: &Scene,
        localization: &LocalizationConfig,
        timezone: &Timezone,
        night_mode_active: bool,
    ) {
        info!(
            scene_id = %scene.id,
            widget_count = scene.widgets.len(),
            "spawning scene widgets"
        );

        for widget in scene.widgets.values() {
            self.spawn_widget(&scene.id, widget, localization, timezone, night_mode_active)
                .await;
        }
    }

    pub async fn spawn_widget(
        &self,
        scene_id: &crate::scene::SceneId,
        widget: &Widget,
        localization: &LocalizationConfig,
        timezone: &Timezone,
        night_mode_active: bool,
    ) {
        let size_info = Self::widget_size_to_size_info(widget.size);
        let settings = Self::build_settings(localization, timezone, night_mode_active);

        let init_msg = AppMessage::Init {
            size: size_info,
            params: widget.params.clone(),
            settings,
        };

        let instance_id = widget.id.as_uuid();

        info!(
            scene_id = %scene_id,
            widget_id = %instance_id,
            widget_type = %widget.widget_type_id,
            size = %widget.size,
            "spawning widget"
        );

        if let Err(e) = self
            .widget_manager
            .spawn_widget(widget.widget_type_id, instance_id, init_msg)
            .await
        {
            warn!(
                scene_id = %scene_id,
                widget_id = %widget.id,
                widget_type = %widget.widget_type_id,
                error = %e,
                "failed to spawn widget"
            );
        }
    }

    pub async fn stop_widget(&self, instance_id: Uuid) {
        self.widget_manager.stop_widget(instance_id).await;
    }

    pub async fn stop_scene_widgets(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.stop_widget(widget.id.as_uuid()).await;
        }
    }

    fn widget_size_to_size_info(size: WidgetSize) -> SizeInfo {
        SizeInfo {
            name: match size {
                WidgetSize::Small => SizeType::Small,
                WidgetSize::Medium => SizeType::Medium,
                WidgetSize::Large => SizeType::Large,
                WidgetSize::Full => SizeType::Full,
            },
            width: size.width(),
            height: size.height(),
        }
    }

    fn build_settings(
        localization: &LocalizationConfig,
        timezone: &Timezone,
        night_mode_active: bool,
    ) -> Settings {
        Settings {
            localization: Some(Localization {
                date_format: localization.date_format,
                time_format: localization.time_system,
                number_format: localization.number_format,
                temperature_unit: localization.temperature_unit,
                first_day_of_week: localization.first_day_of_week,
            }),
            timezone: Some(timezone.to_string()),
            night_mode: Some(night_mode_active),
        }
    }
}
