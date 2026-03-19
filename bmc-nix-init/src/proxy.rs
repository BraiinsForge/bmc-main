// Copyright (C) 2026  Braiins Systems s.r.o.

use slint::EventLoopError;
use slint::platform::EventLoopProxy;

#[expect(missing_debug_implementations)]
pub enum ProxyEvent {
    Event(Box<dyn FnOnce() + Send>),
    Quit,
}

#[derive(Clone, Debug)]
pub struct Proxy {
    event_sender: flume::Sender<ProxyEvent>,
}

impl Proxy {
    #[must_use]
    pub fn new(event_sender: flume::Sender<ProxyEvent>) -> Self {
        Self { event_sender }
    }
}

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        self.event_sender
            .send(ProxyEvent::Quit)
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }

    fn invoke_from_event_loop(
        &self,
        event: Box<dyn FnOnce() + Send>,
    ) -> Result<(), EventLoopError> {
        self.event_sender
            .send(ProxyEvent::Event(event))
            .map_err(|_| EventLoopError::EventLoopTerminated)
    }
}
