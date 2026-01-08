// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_time::time::Timezone;
use bmc_widget_protocol::{Localization, SizeType};
use std::sync::Arc;
use tracing::{info, warn};

use crate::compositor::{Compositor, Position, SceneLayout, Size, WidgetPlacement};
use crate::config::LocalizationConfig;
use crate::scene::{Scene, Widget, WidgetSize};

use super::WidgetManager;

/// Widget configuration passed to spawner via environment variables.
#[derive(Debug, Clone)]
pub struct WidgetEnv {
    pub instance_id: String,
    pub wayland_display: String,
    pub size_type: SizeType,
    pub width: u32,
    pub height: u32,
    pub params: serde_json::Value,
    pub timezone: Option<String>,
    pub night_mode: bool,
    pub localization: Option<Localization>,
}

pub struct Coordinator {
    widget_manager: WidgetManager,
    compositor: Arc<dyn Compositor>,
}

impl std::fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coordinator")
            .field("widget_manager", &self.widget_manager)
            .finish_non_exhaustive()
    }
}

impl Coordinator {
    pub fn new(widget_manager: WidgetManager, compositor: Arc<dyn Compositor>) -> Self {
        Self {
            widget_manager,
            compositor,
        }
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
            // Set the first enabled scene as active so compositor knows where to render widgets
            self.set_active_scene(scene);
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
        let instance_id = widget.id.as_uuid().to_string();
        let position = Self::widget_to_position(widget);
        let size = Self::widget_size_to_size(widget.size);

        // Register widget with compositor before spawning
        if let Err(e) = self
            .compositor
            .register_widget(instance_id.clone(), position, size, None)
        {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                error = %e,
                "failed to register widget with compositor"
            );
            return;
        }

        let Some(wayland_display) = self.compositor.wayland_display() else {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                "compositor not started, cannot spawn widget"
            );
            return;
        };

        let widget_env = WidgetEnv {
            instance_id: instance_id.clone(),
            wayland_display,
            size_type: Self::widget_size_to_size_type(widget.size),
            width: widget.size.width(),
            height: widget.size.height(),
            params: widget.params.clone(),
            timezone: Some(timezone.to_string()),
            night_mode: night_mode_active,
            localization: Some(Self::build_localization(localization)),
        };

        info!(
            scene_id = %scene_id,
            widget_id = %instance_id,
            widget_type = %widget.widget_type_id,
            size = %widget.size,
            "spawning widget"
        );

        if let Err(e) = self
            .widget_manager
            .spawn_widget(widget.widget_type_id, widget_env)
            .await
        {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                widget_type = %widget.widget_type_id,
                error = %e,
                "failed to spawn widget"
            );
            // Unregister from compositor on spawn failure
            let _ = self.compositor.unregister_widget(&instance_id);
        }
    }

    pub async fn stop_widget(&self, instance_id: &str) {
        self.widget_manager.stop_widget(instance_id).await;
        let _ = self.compositor.unregister_widget(&instance_id.to_owned());
    }

    pub async fn stop_scene_widgets(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.stop_widget(&widget.id.as_uuid().to_string()).await;
        }
    }

    /// Sets the active scene layout on the compositor.
    pub fn set_active_scene(&self, scene: &Scene) {
        let layout = Self::scene_to_layout(scene);
        info!(
            scene_id = %scene.id,
            widget_count = layout.widgets.len(),
            "setting active scene on compositor"
        );
        for widget in &layout.widgets {
            info!(
                instance_id = %widget.instance_id,
                x = widget.position.x,
                y = widget.position.y,
                width = widget.size.width,
                height = widget.size.height,
                visible = widget.visible,
                "scene widget placement"
            );
        }
        if let Err(e) = self.compositor.set_active_scene(layout) {
            warn!(scene_id = %scene.id, error = %e, "failed to set active scene");
        }
    }

    fn scene_to_layout(scene: &Scene) -> SceneLayout {
        let widgets = scene
            .widgets
            .values()
            .map(|widget| WidgetPlacement {
                instance_id: widget.id.as_uuid().to_string(),
                position: Self::widget_to_position(widget),
                size: Self::widget_size_to_size(widget.size),
                visible: true,
            })
            .collect();

        SceneLayout { widgets }
    }

    fn widget_to_position(widget: &Widget) -> Position {
        // Convert grid position to pixel position
        // Grid is 4x2, each cell is 320x240
        const CELL_WIDTH: u32 = 320;
        const CELL_HEIGHT: u32 = 240;

        Position {
            x: u32::from(widget.position.col) * CELL_WIDTH,
            y: u32::from(widget.position.row) * CELL_HEIGHT,
        }
    }

    fn widget_size_to_size(size: WidgetSize) -> Size {
        Size {
            width: size.width(),
            height: size.height(),
        }
    }

    fn widget_size_to_size_type(size: WidgetSize) -> SizeType {
        match size {
            WidgetSize::Small => SizeType::Small,
            WidgetSize::Medium => SizeType::Medium,
            WidgetSize::Large => SizeType::Large,
            WidgetSize::Full => SizeType::Full,
        }
    }

    fn build_localization(config: &LocalizationConfig) -> Localization {
        Localization {
            date_format: config.date_format,
            time_format: config.time_system,
            number_format: config.number_format,
            temperature_unit: config.temperature_unit,
            first_day_of_week: config.first_day_of_week,
        }
    }
}
