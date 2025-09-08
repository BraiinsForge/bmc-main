// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::display_controller::DisplayController;
use crate::generated;
use crate::generated::AlarmAdapter;
use slint::ComponentHandle;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::debug;

#[derive(Debug)]
pub enum AlarmEvent {
    Stop,
    Snooze,
}

// When consumer creates two streams using two e.g. `on_example()` calls, then first stream will be closed,
// because Slint does not allow multiple subscribers.
impl DisplayController {
    // NOTE: example how to implement callback mapping from sync (slint callback) to async (stream).
    // TODO: remove after real callback is implemented
    // pub fn on_example(&self) -> impl Stream<Item = ()> + use<> {
    //     self.set_callback(|main_window, tx| {
    //         main_window.on_example(move |payload| {
    //             let _ = tx.send(payload);
    //         })
    //     })
    // }

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
