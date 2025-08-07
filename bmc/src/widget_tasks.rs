// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::BmcManager;
use crate::config::ConfigHandle;
use bmc_display::data::{SceneId, Widget, WidgetId, WidgetKind};
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::interval;

#[derive(Debug)]
struct TaskHandle {
    scene_id: SceneId,
    widget_id: WidgetId,
    handle: JoinHandle<()>,
}

#[derive(Debug)]
pub(crate) struct WidgetTasks<T: BmcManager> {
    task_handles: Arc<Mutex<Vec<TaskHandle>>>,
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
            task_handles: Arc::default(),
            display_controller,
            config_handle,
            manager,
        }
    }

    pub async fn spawn_all(
        &self,
        scene_id: &SceneId,
        widgets: impl ExactSizeIterator<Item = &Widget>,
    ) {
        if widgets.len() == 0 {
            return;
        }
        let mut task_handles = self.task_handles.lock().await;

        for widget in widgets {
            if let Some(task_handle) = self.spawn_internal(scene_id, widget) {
                task_handles.push(task_handle);
            }
        }
    }

    pub async fn spawn(&self, scene_id: &SceneId, widget: &Widget) {
        let mut task_handles = self.task_handles.lock().await;

        if let Some(task_handle) = self.spawn_internal(scene_id, widget) {
            task_handles.push(task_handle);
        }
    }

    fn spawn_internal(&self, scene_id: &SceneId, widget: &Widget) -> Option<TaskHandle> {
        let join_handle = match &widget.kind {
            WidgetKind::Clock(clock_widget) => Some(spawn(self.make_clock_task(
                scene_id.clone(),
                widget.id.clone(),
                clock_widget.timezone.clone(),
            ))),
        };

        join_handle.map(|handle| TaskHandle {
            scene_id: scene_id.clone(),
            widget_id: widget.id.clone(),
            handle,
        })
    }

    pub async fn abort_all(&self, scene_id: &SceneId) {
        self.abort_internal(|task_handle| task_handle.scene_id == *scene_id)
            .await;
    }

    pub async fn abort(&self, scene_id: &SceneId, widget_id: &WidgetId) {
        self.abort_internal(|task_handle| {
            task_handle.scene_id == *scene_id && task_handle.widget_id == *widget_id
        })
        .await;
    }

    async fn abort_internal(&self, predicate: impl Fn(&TaskHandle) -> bool) {
        let mut task_handles = self.task_handles.lock().await;

        // NOTE: refactor code below to use `Vec::extract_if` after upgrade to Rust >= 1.87.0
        if !task_handles.iter().any(&predicate) {
            return;
        }

        let (to_abort, to_keep): (Vec<_>, Vec<_>) =
            task_handles.drain(..).partition(|task_handle| {
                let should_abort = predicate(task_handle);

                if should_abort {
                    task_handle.handle.abort();
                }

                should_abort
            });

        task_handles.extend(to_keep);

        for task_handle in to_abort {
            let _ = task_handle.handle.await;
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
            task_handles: self.task_handles.clone(),
            display_controller: self.display_controller.clone(),
            config_handle: self.config_handle.clone(),
            manager: self.manager.clone(),
        }
    }
}
