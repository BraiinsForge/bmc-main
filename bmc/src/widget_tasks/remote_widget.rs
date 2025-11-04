// Copyright (C) 2025  Braiins Systems s.r.o.

use std::time::Duration;

use bmc_display::data::{SceneId, WidgetId, WidgetSize};
use bmc_display::display_controller::DisplayController;
use tokio::time::{Instant, MissedTickBehavior, interval};
use tracing::{Instrument, info, instrument, warn};

const DATA_REFRESH_PERIOD: Duration = Duration::from_secs(10);

#[instrument(name = "remote_widget", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    widget_size: WidgetSize,
    url: String,
) {
    let mut interval = interval(DATA_REFRESH_PERIOD);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        interval.tick().await;

        info!("Remote Widget Run");
    }
}
