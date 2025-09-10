// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::config::ConfigHandle;
use bmc_display::btc_history_data::BtcHistoryData;
use bmc_display::clock_data::ClockData;
use bmc_display::data::{SceneId, TimeFrame, Widget, WidgetId, WidgetKind};
use bmc_display::display_controller::DisplayController;
use bmc_shared_time::time::Timezone;
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use tokio::sync::{Mutex, RwLock, watch};
use tokio::task::JoinHandle;
use tokio::time::interval;
use tracing::{debug, warn};

const BTC_HISTORY_API_URL: &str = "https://public-api.braiins.com/v1/price-history";
const BTC_HISTORY_TIMEFRAME_API_PARAM: &str = "timeframe";
const API_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
struct TaskHandle {
    scene_id: SceneId,
    widget_id: WidgetId,
    handle: JoinHandle<()>,
}

#[derive(Debug)]
pub(crate) struct WidgetTasks {
    task_handles: Arc<Mutex<Vec<TaskHandle>>>,
    display_controller: DisplayController,
    config_handle: Arc<RwLock<ConfigHandle>>,
    system_timezone_receiver: watch::Receiver<Timezone>,
}

impl WidgetTasks {
    pub(crate) fn new(
        display_controller: DisplayController,
        config_handle: Arc<RwLock<ConfigHandle>>,
        system_timezone_receiver: watch::Receiver<Timezone>,
    ) -> Self {
        Self {
            task_handles: Arc::default(),
            display_controller,
            config_handle,
            system_timezone_receiver,
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
            WidgetKind::TickerBtc(ticker_widget) => Some(spawn(self.make_btc_graph_task(
                scene_id.clone(),
                widget.id.clone(),
                ticker_widget.time_frame.clone(),
            ))),
            // BlockHeight widget does not have any widget specific data
            WidgetKind::BlockHeight(_) => None,
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
        let mut system_timezone_receiver = self.system_timezone_receiver.clone();

        async move {
            let mut interval = interval(Duration::from_millis(250));

            loop {
                interval.tick().await;

                let timezone = timezone
                    .clone()
                    .unwrap_or_else(|| system_timezone_receiver.borrow_and_update().clone());

                let now = chrono::Local::now()
                    .with_timezone(timezone.chrono())
                    .fixed_offset();

                let is_24_format = config_handle
                    .read()
                    .await
                    .localization_config()
                    .time_system
                    .is_24();

                let clock_data = ClockData::new(now);

                display_controller.update_clock_widget(
                    scene_id.clone(),
                    widget_id.clone(),
                    now,
                    timezone.to_string(),
                    is_24_format,
                    clock_data,
                );
            }
        }
    }

    fn make_btc_graph_task(
        &self,
        scene_id: SceneId,
        widget_id: WidgetId,
        timeframe: TimeFrame,
    ) -> impl Future<Output = ()> + Send + 'static {
        let display_controller = self.display_controller.clone();

        async move {
            let mut interval = interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                debug!("Getting bitcoin history data...");
                let client = Client::new();
                let btc_history_data = if let Ok(response) = client
                    .get(BTC_HISTORY_API_URL)
                    .query(&[(
                        BTC_HISTORY_TIMEFRAME_API_PARAM,
                        Into::<String>::into(timeframe.clone()),
                    )])
                    .timeout(API_TIMEOUT)
                    .send()
                    .await
                {
                    response.json::<BtcHistoryData>().await.unwrap_or_default()
                } else {
                    warn!("Failed to get btc history data from API");
                    BtcHistoryData::default()
                };

                display_controller.update_btc_graph(
                    scene_id.clone(),
                    widget_id.clone(),
                    btc_history_data,
                );
            }
        }
    }
}

impl Clone for WidgetTasks {
    fn clone(&self) -> Self {
        Self {
            task_handles: self.task_handles.clone(),
            display_controller: self.display_controller.clone(),
            config_handle: self.config_handle.clone(),
            system_timezone_receiver: self.system_timezone_receiver.clone(),
        }
    }
}
