// Copyright (C) 2025  Braiins Systems s.r.o.

use slint::EventLoopError;
use slint::platform::EventLoopProxy;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Clone, Debug)]
pub struct Proxy {
    pub quit_loop: Arc<AtomicBool>,
    event_channel: flume::Sender<Box<dyn FnOnce() + Send>>,
}

impl Proxy {
    #[must_use]
    pub fn new(event_channel: flume::Sender<Box<dyn FnOnce() + Send>>) -> Self {
        Self {
            quit_loop: Arc::new(AtomicBool::new(false)),
            event_channel,
        }
    }
}

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.quit_loop
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(())
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        self.event_channel
            .send(event)
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
}
