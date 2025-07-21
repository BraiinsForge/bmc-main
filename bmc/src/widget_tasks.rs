// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::config::ConfigHandle;
use bmc_display::data::{Scene, SceneId, Widget, WidgetId, WidgetKind};
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::interval;

type HandleKey = (SceneId, WidgetId);

#[derive(Debug)]
pub(crate) struct WidgetTasks<T: BmcManager> {
    handles: Arc<Mutex<HashMap<HandleKey, JoinHandle<()>>>>,
    display_controller: DisplayController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    manager: Arc<T>,
}

impl<T: BmcManager> WidgetTasks<T> {
    pub(crate) fn new(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        manager: Arc<T>,
    ) -> Self {
        Self {
            handles: Arc::default(),
            display_controller,
            config_handle,
            manager,
        }
    }

    pub async fn spawn_all(&self, scene: &Scene, force_enable: bool) {
        if scene.enabled || force_enable {
            for widget in scene.widgets.values() {
                self.spawn(scene.id.clone(), widget).await;
            }
        }
    }

    pub async fn spawn(&self, scene_id: SceneId, widget: &Widget) {
        let mut handles = self.handles.lock().await;
        let key = (scene_id.clone(), widget.id.clone());

        if let Some(handle) = handles.remove(&key) {
            handle.abort();
            let _ = handle.await;
        }

        let future = match &widget.kind {
            WidgetKind::Clock(clock_widget) => {
                self.make_clock_task(scene_id, widget.id.clone(), clock_widget.timezone.clone())
            }
        };

        handles.insert(key, spawn(future));
    }

    pub async fn abort_all(&self, scene: &Scene) {
        for widget in scene.widgets.values() {
            self.abort(scene.id.clone(), widget.id.clone()).await;
        }
    }

    pub async fn abort(&self, scene_id: SceneId, widget_id: WidgetId) {
        let mut handles = self.handles.lock().await;
        let key = (scene_id, widget_id);

        if let Some(handle) = handles.remove(&key) {
            handle.abort();
            let _ = handle.await;
        }
    }

    fn make_clock_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        timezone: Option<Timezone>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();
        let config_handle = self.config_handle.clone();
        let manager = self.manager.clone();

        async move {
            let mut interval = interval(Duration::from_millis(250));
            let mut is_24_format = config_handle
                .read()
                .await
                .localization_config()
                .time_system
                .is_24();

            loop {
                interval.tick().await;

                let timezone = timezone.clone().unwrap_or_else(|| manager.timezone());
                let now = chrono::Local::now()
                    .with_timezone(timezone.chrono())
                    .fixed_offset();

                // FIXME: avoid lock contention
                if let Ok(config) = config_handle.try_read() {
                    is_24_format = config.localization_config().time_system.is_24();
                }

                display_controller.update_clock_widget(
                    scene_id.clone(),
                    widget_id.clone(),
                    now,
                    timezone.to_string(),
                    is_24_format,
                );
            }
        }
    }
}

impl<T: BmcManager> Clone for WidgetTasks<T> {
    fn clone(&self) -> Self {
        Self {
            handles: self.handles.clone(),
            display_controller: self.display_controller.clone(),
            config_handle: self.config_handle.clone(),
            manager: self.manager.clone(),
        }
    }
}
