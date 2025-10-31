// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::WidgetSize;
use crate::display_controller::DisplayController;
use crate::generated;
use crate::generated::{AlarmAdapter, BaseDimensions, BrightnessAdapter, SoundAdapter};
use slint::ComponentHandle;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::debug;

#[derive(Debug)]
pub enum AlarmEvent {
    Stop,
    Snooze,
}

#[derive(Debug)]
pub enum BrightnessEvent {
    Increase,
    Decrease,
}

#[derive(Debug)]
pub enum SoundEvent {
    Increase,
    Decrease,
}

// When consumer creates two streams using two e.g. `on_alarm_events()` calls, then first stream will be closed,
// because Slint does not allow multiple subscribers.
impl DisplayController {
    pub(super) fn setup_static_callbacks(main_window: &generated::MainWindow) {
        let base_dimensions = main_window.global::<BaseDimensions<'_>>();

        base_dimensions.on_widget_width_int(|widget_size| {
            let widget_size = WidgetSize::from(widget_size);

            #[expect(clippy::cast_possible_wrap)]
            let width = widget_size.width() as i32;

            width
        });

        base_dimensions.on_widget_height_int(|widget_size| {
            let widget_size = WidgetSize::from(widget_size);

            #[expect(clippy::cast_possible_wrap)]
            let height = widget_size.height() as i32;

            height
        });
    }

    #[must_use]
    pub fn on_alarm_events(&self) -> UnboundedReceiverStream<AlarmEvent> {
        let (tx, rx) = unbounded_channel();

        self.in_event_loop(move |main_window| {
            let alarm_adapter = main_window.global::<AlarmAdapter<'_>>();

            alarm_adapter.on_stop_alarm({
                let tx = tx.clone();
                move || {
                    debug!("Stop alarm clicked!");
                    _ = tx.send(AlarmEvent::Stop);
                }
            });

            alarm_adapter.on_snooze_alarm(move || {
                debug!("Snooze alarm clicked!");
                _ = tx.send(AlarmEvent::Snooze);
            });
        });
        UnboundedReceiverStream::new(rx)
    }

    #[must_use]
    pub fn on_brightness_events(&self) -> UnboundedReceiverStream<BrightnessEvent> {
        let (tx, rx) = unbounded_channel();

        self.in_event_loop(move |main_window| {
            let brightness_adapter = main_window.global::<BrightnessAdapter<'_>>();

            brightness_adapter.on_brightness_increase({
                let tx = tx.clone();
                move || {
                    debug!("Brightness increase clicked!");
                    _ = tx.send(BrightnessEvent::Increase);
                }
            });

            brightness_adapter.on_brightness_decrease(move || {
                debug!("Brightness decrease clicked!");
                _ = tx.send(BrightnessEvent::Decrease);
            });
        });
        UnboundedReceiverStream::new(rx)
    }

    #[must_use]
    pub fn on_sound_events(&self) -> UnboundedReceiverStream<SoundEvent> {
        let (tx, rx) = unbounded_channel();

        self.in_event_loop(move |main_window| {
            let sound_adapter = main_window.global::<SoundAdapter<'_>>();

            sound_adapter.on_volume_increase({
                let tx = tx.clone();
                move || {
                    debug!("Sound volume increase clicked!");
                    _ = tx.send(SoundEvent::Increase);
                }
            });

            sound_adapter.on_volume_decrease(move || {
                debug!("Sound volume decrease clicked!");
                _ = tx.send(SoundEvent::Decrease);
            });
        });
        UnboundedReceiverStream::new(rx)
    }

    #[must_use]
    pub fn on_restart_events(&self) -> UnboundedReceiverStream<()> {
        let (tx, rx) = unbounded_channel();

        self.in_event_loop(move |main_window| {
            main_window.on_restart(move || {
                debug!("Restart clicked!");
                _ = tx.send(());
            });
        });
        UnboundedReceiverStream::new(rx)
    }

    #[must_use]
    pub fn on_night_mode_toggle_events(&self) -> UnboundedReceiverStream<()> {
        let (tx, rx) = unbounded_channel();

        self.in_event_loop(move |main_window| {
            let night_mode_adapter = main_window.global::<generated::NightModeAdapter<'_>>();

            night_mode_adapter.on_toggle(move || {
                debug!("Night mode toggle clicked!");
                _ = tx.send(());
            });
        });
        UnboundedReceiverStream::new(rx)
    }

    #[expect(unused)]
    fn set_unbounded_callback<T, F>(&self, func: F) -> UnboundedReceiverStream<T>
    where
        T: Send + 'static,
        F: FnOnce(generated::MainWindow, UnboundedSender<T>) + Send + 'static,
    {
        let (tx, rx) = unbounded_channel();

        self.in_event_loop(move |main_window| func(main_window, tx));

        UnboundedReceiverStream::new(rx)
    }
}
