// Copyright (C) 2025  Braiins Systems s.r.o.

use anyhow::Result;
use bmc_button::{ButtonEventStream, ButtonId, Buttons, Edge};
use futures::StreamExt;
use futures::stream::empty;
use std::sync::Arc;

const BMC_RESET_BUTTON_NUMBER: u64 = 0;

#[derive(Clone, Debug)]
pub struct GeneralButton {
    pub button_number: u64,
    pub button_id: ButtonId,
    pub edge: Edge,
}

#[derive(Clone, Debug)]
pub struct GeneralButtons {
    pub list: Vec<GeneralButton>,
}

impl Buttons for GeneralButtons {
    fn to_stream(&self) -> Result<ButtonEventStream> {
        Ok(empty().boxed())
    }
}

#[must_use]
pub fn build_buttons() -> Arc<Box<dyn Buttons + Send + Sync + 'static>> {
    Arc::new(Box::new(GeneralButtons {
        list: vec![GeneralButton {
            button_number: BMC_RESET_BUTTON_NUMBER,
            button_id: ButtonId::Reset,
            edge: Edge::BothEdges,
        }],
    }))
}
