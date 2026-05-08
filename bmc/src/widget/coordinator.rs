// Copyright (C) 2025  Braiins Systems s.r.o.

use bmc_shared_time::time::Timezone;
use bmc_widget::{ParamKey, ParamValue};
use bmc_widget_protocol::{Localization, SizeType, WidgetInitialConfig};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::compositor::{
    Compositor, CompositorError, Position, SceneLayout, Size, WidgetPlacement,
};
use crate::config::LocalizationConfig;
use crate::scene::{Scene, SceneId, Widget, WidgetSize};

use super::WidgetManager;

/// Minimum environment the spawner puts on a widget process.
///
/// Every piece of widget-specific configuration (instance id, size,
/// params) now flows through the `deck_widget_v1` Wayland protocol, so
/// this struct no longer carries any of that — the spawner only needs
/// to know where the Wayland socket lives and which instance id to
/// attribute the spawn to for logging.
#[derive(Debug, Clone)]
pub struct WidgetEnv {
    pub instance_id: String,
    pub wayland_display: String,
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
        scenes: &IndexMap<SceneId, Scene>,
        localization: &LocalizationConfig,
        timezone: &Timezone,
        night_mode_active: bool,
    ) {
        // Seed the compositor's setting cache so it can emit these as
        // part of the initial configure batch for every widget that
        // connects (and also propagate subsequent changes).
        use bmc_widget_protocol::SettingUpdate;
        let _ = self
            .compositor
            .broadcast_setting(SettingUpdate::Timezone(timezone.to_string()));
        let _ = self
            .compositor
            .broadcast_setting(SettingUpdate::NightMode(night_mode_active));
        let loc = Self::build_localization(localization);
        for setting in SettingUpdate::from_localization(&loc) {
            let _ = self.compositor.broadcast_setting(setting);
        }

        let enabled_count = scenes.values().filter(|s| s.enabled).count();
        info!(count = enabled_count, "spawning widgets for enabled scenes");
        for scene in scenes.values().filter(|s| s.enabled) {
            self.spawn_scene_widgets(scene).await;
        }

        self.refresh_scene_cycling(scenes);

        info!("all scene widgets spawned");
    }

    /// Push the current enabled-scenes layout list to the compositor's
    /// drag-cycling state. Call after any scene-set or widget-layout
    /// mutation so swipe targets the post-mutation layouts.
    pub fn refresh_scene_cycling(&self, scenes: &IndexMap<SceneId, Scene>) {
        let layouts: Vec<_> = scenes
            .values()
            .filter(|s| s.enabled)
            .map(Self::scene_to_layout)
            .collect();
        debug!(
            count = layouts.len(),
            "refreshing scene cycling on compositor"
        );
        if let Err(e) = self.compositor.set_scene_cycling(layouts) {
            warn!(error = %e, "failed to refresh scene cycling");
        }
    }

    pub async fn spawn_scene_widgets(&self, scene: &Scene) {
        info!(
            scene_id = %scene.id,
            widget_count = scene.widgets.len(),
            "spawning scene widgets"
        );

        for widget in scene.widgets.values() {
            self.spawn_widget(&scene.id, widget).await;
        }
    }

    pub async fn spawn_widget(&self, scene_id: &crate::scene::SceneId, widget: &Widget) {
        let instance_id = widget.id.as_uuid().to_string();
        let position = Self::widget_to_position(widget);
        let size = Self::widget_size_to_size(widget.size);
        let initial_config = WidgetInitialConfig {
            size: Self::widget_size_to_size_type(widget.size),
            width: widget.size.width(),
            height: widget.size.height(),
            params: params_to_json_map(&widget.params),
        };

        // Register widget with compositor before spawning. This call blocks
        // until the compositor has stored the initial config — otherwise a
        // fast-starting widget could reach `get_widget_surface` before the
        // compositor knows what to emit.
        if let Err(e) =
            self.compositor
                .register_widget(instance_id.clone(), position, size, initial_config)
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
            let _ = self.compositor.unregister_widget(&instance_id);
            return;
        };

        let widget_env = WidgetEnv {
            instance_id: instance_id.clone(),
            wayland_display,
        };

        info!(
            scene_id = %scene_id,
            widget_id = %instance_id,
            widget_type = %widget.widget_type_id,
            size = %widget.size,
            "spawning widget"
        );

        let (pid, exit_rx) = match self
            .widget_manager
            .spawn_widget(widget.widget_type_id, widget_env)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                warn!(
                    scene_id = %scene_id,
                    widget_id = %instance_id,
                    widget_type = %widget.widget_type_id,
                    error = %e,
                    "failed to spawn widget"
                );
                let _ = self.compositor.unregister_widget(&instance_id);
                return;
            }
        };

        if let Err(e) = self.compositor.set_widget_pid(&instance_id, pid) {
            warn!(
                scene_id = %scene_id,
                widget_id = %instance_id,
                pid,
                error = %e,
                "failed to associate pid with widget; widget may not receive initial state"
            );
        }

        // Clear the pid from the compositor when the child exits so a
        // recycled pid cannot be mistaken for this widget.
        let compositor = Arc::clone(&self.compositor);
        tokio::spawn(async move {
            if let Ok(exited_pid) = exit_rx.await {
                let _ = compositor.clear_pid(exited_pid);
            }
        });
    }

    pub async fn stop_widget(&self, instance_id: &str) {
        self.widget_manager.stop_widget(instance_id).await;
        let _ = self.compositor.unregister_widget(&instance_id.to_owned());
    }

    pub fn update_widget_params(
        &self,
        instance_id: &str,
        params: &BTreeMap<ParamKey, ParamValue>,
    ) -> Result<(), CompositorError> {
        self.compositor
            .update_widget_params(&instance_id.to_owned(), params_to_json_map(params))
    }

    pub async fn stop_scene_widgets(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.stop_widget(&widget.id.as_uuid().to_string()).await;
        }
    }

    /// Stop all widget processes and shut down the compositor.
    ///
    /// Widgets are stopped first (SIGTERM → 10s timeout → SIGKILL) because they
    /// need the Wayland display socket to clean up GPU resources (GEM/DMA-BUF).
    /// The compositor is shut down second.
    pub async fn stop_all(&self) {
        info!("stopping all widgets and compositor");
        self.widget_manager.stop_all().await;
        if let Err(e) = self.compositor.shutdown() {
            warn!(error = %e, "failed to shut down compositor");
        }
        info!("shutdown complete");
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

        SceneLayout {
            scene_id: Some(scene.id),
            widgets,
        }
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

fn params_to_json_map(
    params: &BTreeMap<ParamKey, ParamValue>,
) -> serde_json::Map<String, serde_json::Value> {
    params
        .iter()
        .map(|(k, v)| (k.as_str().to_owned(), v.to_json_value()))
        .collect()
}
