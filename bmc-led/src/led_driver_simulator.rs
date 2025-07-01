// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::data::LedEvent;
use std::fmt::Debug;
use tokio::sync::mpsc::Sender;

const EVENT_BUFFER_SIZE: usize = 4;

#[async_trait::async_trait]
pub trait LedHandle: Sync + Send + Clone + Debug {
    fn init(&self) -> anyhow::Result<()>;
    async fn emit_event(&self, event: LedEvent);
}

#[derive(Debug)]
pub struct LedDriver;

impl LedDriver {
    #[must_use]
    pub fn new() -> Self {
        Self {}
    }

    pub fn change_state(&self, _enabled: bool) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn state(&self) -> anyhow::Result<bool> {
        Ok(true)
    }

    pub fn toggle_state(&mut self) -> anyhow::Result<()> {
        self.state().and_then(|state| self.change_state(!state))
    }

    pub fn turn_on(&self) -> anyhow::Result<()> {
        self.change_state(true)
    }

    pub fn turn_off(&self) -> anyhow::Result<()> {
        self.change_state(false)
    }

    pub fn brightness(&self) -> anyhow::Result<u8> {
        Ok(0)
    }

    #[must_use]
    pub fn max_brightness(&self) -> u8 {
        0
    }

    pub fn set_brightness(&self, _value: u8) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn init(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct LedHandler {
    event_sender: Sender<LedEvent>,
}

impl LedHandler {
    #[must_use]
    pub fn new(_: LedDriver) -> Self {
        Self {
            event_sender: EventHandler::init(),
        }
    }
}

#[async_trait::async_trait]
impl LedHandle for LedHandler {
    fn init(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn emit_event(&self, event: LedEvent) {
        _ = self.event_sender.send(event).await;
    }
}

struct EventHandler;

impl EventHandler {
    fn init() -> Sender<LedEvent> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(EVENT_BUFFER_SIZE);

        tokio::spawn(async move {
            while let Some(_event) = receiver.recv().await {
                // Do nothing
            }
        });

        sender
    }
}
