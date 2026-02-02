// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::countdown_types::CountdownCompletionAction;
use crate::sound::SoundController;
use bmc_display::countdown_data::CountdownData;
use bmc_display::data::{SceneId, WidgetId};
use bmc_display::display_controller::DisplayController;
use bmc_led::data::{LedCommand, LedEventPersistence};
use std::time::Duration;
use tokio::sync::mpsc::Sender;
use tokio::time::{interval, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};

/// Duration to show completion effects before stopping
const COMPLETION_EFFECT_DURATION: Duration = Duration::from_secs(30);

/// LED breathing period
const BREATHE_PERIOD: Duration = Duration::from_millis(4000);

/// Delay before automatically dismissing a completed countdown scene
const AUTO_DISMISS_DELAY: Duration = Duration::from_secs(5 * 60);

#[expect(clippy::too_many_arguments)]
#[instrument(name = "countdown", skip_all, fields(%scene_id, %widget_id))]
pub async fn run(
    display_controller: DisplayController,
    scene_id: SceneId,
    widget_id: WidgetId,
    label: String,
    target_timestamp: i64,
    completion_action: Option<CountdownCompletionAction>,
    led_event_tx: Option<Sender<LedCommand>>,
    sound_controller: Option<SoundController>,
) {
    info!(
        label = %label,
        target_timestamp = %target_timestamp,
        has_completion_action = completion_action.is_some(),
        "Starting countdown widget"
    );

    let mut tick_interval = interval(Duration::from_secs(1));

    loop {
        tick_interval.tick().await;

        // Calculate countdown data
        let countdown_data = CountdownData::new(target_timestamp, chrono::Utc::now().timestamp());

        // Update the display
        display_controller.update_countdown_widget(
            scene_id.clone(),
            widget_id.clone(),
            label.clone(),
            countdown_data.clone(),
        );

        // Check if countdown completed and trigger actions
        if countdown_data.is_completed {
            if let Some(ref action) = completion_action {
                info!("Countdown completed, triggering completion actions");

                // Trigger LED effect
                if let Some(ref led) = action.led {
                    if let Some(ref tx) = led_event_tx {
                        let effect = led.effect.with_color(led.color);
                        let cmd = LedCommand::SetEffect(
                            effect,
                            LedEventPersistence::Temporary(COMPLETION_EFFECT_DURATION),
                            BREATHE_PERIOD,
                        );

                        if let Err(err) = tx.try_send(cmd) {
                            warn!(?err, "Failed to send LED command");
                        } else {
                            debug!("LED effect triggered");
                        }
                    }
                }

                // Trigger sound
                if let Some(ref sound_settings) = action.sound {
                    let Some(ref controller) = sound_controller else {
                        warn!("Sound configured but no sound controller available");
                        return;
                    };

                    // Save current volume to restore after playback
                    let previous_volume = controller.sound_volume().await;

                    // Set countdown-specific volume
                    if let Err(err) = controller
                        .set_audio_sound_volume(sound_settings.volume)
                        .await
                    {
                        warn!(?err, "Failed to set sound volume");
                    }

                    // Play sound in loop until cancelled
                    let token = CancellationToken::new();
                    let token_cancel = token.clone();

                    // Cancel after completion effect duration
                    tokio::spawn(async move {
                        sleep(COMPLETION_EFFECT_DURATION).await;
                        token_cancel.cancel();
                    });

                    let controller = controller.clone();
                    let sound = sound_settings.sound.clone();
                    tokio::spawn(async move {
                        controller.play_until_cancelled(sound, token).await;
                        // Restore previous volume
                        if let Err(err) = controller.set_audio_sound_volume(previous_volume).await {
                            warn!(?err, "Failed to restore sound volume");
                        }
                    });

                    debug!(sound = %sound_settings.sound, "Sound triggered");
                }
            }

            // Auto-dismiss the scene after a delay
            let dc = display_controller.clone();
            let sid = scene_id.clone();
            let wid = widget_id.clone();
            tokio::spawn(async move {
                sleep(AUTO_DISMISS_DELAY).await;
                info!("Auto-dismissing completed countdown scene");
                dc.auto_dismiss_countdown(sid, wid);
            });

            return;
        }
    }
}
